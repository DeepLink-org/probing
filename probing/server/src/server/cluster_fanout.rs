//! On-demand SQL fan-out across cluster nodes.
//!
//! Training agents write locally; cross-node aggregation runs only when a control-plane
//! caller explicitly requests `cluster=true`.

use std::time::Duration;

use probing_core::core::cluster::get_nodes;
use probing_proto::prelude::*;

use crate::engine::handle_query;

const DEFAULT_REMOTE_TIMEOUT_SECS: u64 = 2;

pub fn local_listen_addrs() -> Vec<String> {
    let mut addrs = Vec::new();
    if let Ok(addr) = crate::vars::PROBING_ADDRESS.read() {
        if !addr.is_empty() {
            addrs.push(addr.clone());
        }
    }
    addrs
}

pub fn local_host_label() -> String {
    crate::report::get_hostname().unwrap_or_else(|_| "localhost".into())
}

pub fn local_addr_label() -> String {
    local_listen_addrs()
        .into_iter()
        .next()
        .unwrap_or_else(|| "127.0.0.1:8080".into())
}

pub async fn query_local_df(sql: &str) -> anyhow::Result<DataFrame> {
    match handle_query(Query {
        expr: sql.to_string(),
        ..Default::default()
    })
    .await?
    {
        QueryDataFormat::DataFrame(df) => Ok(df),
        QueryDataFormat::Nil => Ok(DataFrame {
            names: vec![],
            cols: vec![],
            size: 0,
        }),
        QueryDataFormat::Error(err) => anyhow::bail!("query error: {}", err.message),
        QueryDataFormat::TimeSeries(_) => anyhow::bail!("unexpected timeseries"),
    }
}

pub async fn remote_query_df(addr: &str, sql: &str) -> anyhow::Result<DataFrame> {
    let url = format!("http://{addr}/query");
    let request = Message::new(Query {
        expr: sql.to_string(),
        ..Default::default()
    });
    let body = serde_json::to_string(&request)?;
    let timeout = Duration::from_secs(DEFAULT_REMOTE_TIMEOUT_SECS);
    let response = tokio::task::spawn_blocking(move || {
        ureq::post(&url)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .send(body)
            .map_err(|e| anyhow::anyhow!("{e}"))
    })
    .await??;

    let status = response.status().as_u16();
    let text = response.into_body().read_to_string()?;
    if status >= 400 {
        anyhow::bail!("HTTP {status}: {text}");
    }

    let msg: Message<QueryDataFormat> = serde_json::from_str(&text)?;
    match msg.payload {
        QueryDataFormat::DataFrame(df) => Ok(df),
        QueryDataFormat::Nil => Ok(DataFrame {
            names: vec![],
            cols: vec![],
            size: 0,
        }),
        QueryDataFormat::Error(err) => anyhow::bail!("remote query: {}", err.message),
        QueryDataFormat::TimeSeries(_) => anyhow::bail!("unexpected timeseries"),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FanoutMeta {
    pub cluster: bool,
    pub nodes_queried: usize,
    pub nodes_failed: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FanoutQueryResponse {
    pub dataframe: DataFrame,
    pub meta: FanoutMeta,
}

/// Run `sql` locally, optionally fanning out to peer nodes discovered via Pulsing.
pub async fn fanout_query(sql: &str, cluster: bool) -> anyhow::Result<FanoutQueryResponse> {
    let host = local_host_label();
    let addr = local_addr_label();
    let mut parts = vec![tag_dataframe(query_local_df(sql).await?, &host, &addr)];
    let mut nodes_queried = 1usize;
    let mut nodes_failed = Vec::new();

    if cluster {
        let local_addrs = local_listen_addrs();
        for node in get_nodes() {
            if local_addrs.contains(&node.addr) {
                continue;
            }
            match remote_query_df(&node.addr, sql).await {
                Ok(df) => {
                    parts.push(tag_dataframe(
                        df,
                        if node.host.is_empty() {
                            &node.addr
                        } else {
                            &node.host
                        },
                        &node.addr,
                    ));
                    nodes_queried += 1;
                }
                Err(err) => {
                    log::debug!("cluster fan-out {} failed: {err}", node.addr);
                    nodes_failed.push(node.addr.clone());
                }
            }
        }
    }

    Ok(FanoutQueryResponse {
        dataframe: merge_tagged_dataframes(&parts),
        meta: FanoutMeta {
            cluster,
            nodes_queried,
            nodes_failed,
        },
    })
}

fn tag_dataframe(mut df: DataFrame, host: &str, addr: &str) -> DataFrame {
    if df.is_empty() {
        return df;
    }
    let rows = df.len();
    df.names.push("_probe_host".to_string());
    df.names.push("_probe_addr".to_string());
    df.cols.push(Seq::SeqText(vec![host.to_string(); rows]));
    df.cols.push(Seq::SeqText(vec![addr.to_string(); rows]));
    df.size = df.len() as u64;
    df
}

fn merge_tagged_dataframes(parts: &[DataFrame]) -> DataFrame {
    let mut out = DataFrame::default();
    for df in parts {
        if df.is_empty() {
            continue;
        }
        if out.is_empty() {
            out = df.clone();
            continue;
        }
        append_dataframe(&mut out, df);
    }
    out.size = out.len() as u64;
    out
}

fn append_dataframe(base: &mut DataFrame, other: &DataFrame) {
    if other.is_empty() {
        return;
    }
    if base.is_empty() {
        *base = other.clone();
        return;
    }

    let other_rows = other.len();
    for name in &other.names {
        if !base.names.contains(name) {
            base.names.push(name.clone());
            base.cols
                .push(Seq::SeqText(vec![String::new(); base.len()]));
        }
    }
    for (col_idx, name) in base.names.clone().iter().enumerate() {
        let src_idx = other.names.iter().position(|n| n == name);
        for row in 0..other_rows {
            let ele = src_idx
                .and_then(|i| other.cols.get(i).map(|c| c.get(row)))
                .unwrap_or(Ele::Nil);
            if let Some(col) = base.cols.get_mut(col_idx) {
                let _ = col.append(ele);
            }
        }
    }
    base.size = base.len() as u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_probe_tags() {
        let local = tag_dataframe(
            DataFrame {
                names: vec!["rank".into()],
                cols: vec![Seq::SeqI32(vec![0])],
                size: 1,
            },
            "host-a",
            "10.0.0.1:8080",
        );
        let remote = tag_dataframe(
            DataFrame {
                names: vec!["rank".into()],
                cols: vec![Seq::SeqI32(vec![1])],
                size: 1,
            },
            "host-b",
            "10.0.0.2:8080",
        );
        let merged = merge_tagged_dataframes(&[local, remote]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.names.len(), 3);
        let host_col = merged
            .names
            .iter()
            .position(|n| n == "_probe_host")
            .unwrap();
        assert_eq!(merged.cols[host_col].get_str(0).as_deref(), Some("host-a"));
        assert_eq!(merged.cols[host_col].get_str(1).as_deref(), Some("host-b"));
    }

    #[test]
    fn merge_aligns_missing_columns_with_empty_strings() {
        let a = DataFrame {
            names: vec!["x".into(), "extra".into()],
            cols: vec![Seq::SeqI32(vec![1]), Seq::SeqText(vec!["a".into()])],
            size: 1,
        };
        let b = DataFrame {
            names: vec!["x".into()],
            cols: vec![Seq::SeqI32(vec![2])],
            size: 1,
        };
        let merged = merge_tagged_dataframes(&[a, b]);
        assert_eq!(merged.len(), 2);
        assert!(merged.names.contains(&"extra".to_string()));
    }
}
