use std::{collections::BTreeMap, thread};

use anyhow::Result;
use html_escape::encode_text;
use inferno;
use log::{error, warn};

use crate::extensions::python::PythonPlugin;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Frame {
    stage: String,
    module: String,
}

const TORCH_QUERY: &str = r#"
    select module, stage, median(CAST(duration AS DOUBLE))
        from python.torch_trace
        where module <> 'None'
        group by module, stage
        order by (stage, module);
"#;

/// Query torch profiling data. Prefer the global ENGINE (server's engine) so that
/// when PROBING_TORCH_PROFILING=on the flamegraph uses the same data as the UI.
fn query_profiling_impl() -> Result<probing_proto::types::DataFrame> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create runtime: {e}"))?;

    rt.block_on(async {
        let engine = probing_core::ENGINE.read().await;
        let result = engine
            .async_query(TORCH_QUERY)
            .await
            .map_err(|e| anyhow::anyhow!("Torch query failed: {e}"))?;
        Ok(result.unwrap_or_default())
    })
}

pub fn query_profiling() -> Result<Vec<String>> {
    let data = thread::spawn(|| -> Result<probing_proto::types::DataFrame> {
        // Use global ENGINE first (server's engine with python.torch_trace data)
        match query_profiling_impl() {
            Ok(df) => return Ok(df),
            Err(e) => {
                log::debug!("Global engine torch query failed ({e}), trying minimal engine");
            }
        }
        // Fallback: build a minimal engine (e.g. when not running inside server)
        let engine = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                probing_core::create_engine()
                    .with_plugin(PythonPlugin::create("python"))
                    .build()
                    .await
            })?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Ok(rt.block_on(async { engine.async_query(TORCH_QUERY).await })?
            .unwrap_or_default())
    })
    .join()
    .map_err(|_| anyhow::anyhow!("error joining thread"))??;

    let mut frames = BTreeMap::default();

    for line in data.iter() {
        let frame = Frame {
            stage: line[1].to_string(),
            module: line[0].to_string(),
        };
        let duration = match line[2] {
            probing_proto::types::Ele::F32(x) => x as f64,
            probing_proto::types::Ele::F64(x) => x,
            _ => 0 as f64,
        };

        frames
            .entry(frame.clone())
            .and_modify(|x| *x += duration)
            .or_insert(duration);

        let mut parts = frame.module.split(".").collect::<Vec<_>>();
        if parts.len() > 1 {
            parts.pop();
            let parent = Frame {
                stage: frame.stage.clone(),
                module: parts.join("."),
            };
            frames.entry(parent).and_modify(|x| *x -= duration);
        }
    }

    Ok(frames
        .iter()
        .map(|(frame, duration)| {
            let mut line = String::default();
            line.push_str(&frame.stage);
            line.push(';');

            let parts = frame.module.split(".").collect::<Vec<_>>();
            for part in parts {
                line.push_str(part);
                line.push(';');
            }

            let duration = if *duration < 0. { 0. } else { *duration };

            // Convert duration from seconds to nanoseconds for accurate time representation
            // in the flame graph (inferno expects sample counts, we use nanoseconds as units)
            let duration_ns = (duration * 1_000_000_000.0) as u64;
            line.push_str(&format!(" {}", duration_ns));

            line
        })
        .collect())
}

pub fn flamegraph() -> String {
    let mut graph: Vec<u8> = vec![];
    match query_profiling() {
        Err(err) => {
            error!("Failed to query torch profiling data: {err}");
            return empty_svg("Torch profiling data unavailable");
        }
        Ok(lines) => {
            if lines.is_empty() {
                warn!("Torch profiling returned no samples; skipping flamegraph generation");
                return empty_svg("No torch profiling samples collected");
            }

            let line_refs = lines.iter().map(|x| x.as_str()).collect::<Vec<_>>();
            let mut opt = inferno::flamegraph::Options::default();
            opt.deterministic = true;
            // Set title to indicate this is a torch profiling flamegraph with time units (nanoseconds)
            opt.title = "Torch Profiling Flamegraph (time in nanoseconds)".to_string();
            // Set count name to indicate the unit (nanoseconds instead of samples)
            opt.count_name = "ns".to_string();
            match inferno::flamegraph::from_lines(&mut opt, line_refs, &mut graph) {
                Ok(_) => String::from_utf8(graph)
                    .unwrap_or_else(|_| empty_svg("Invalid flamegraph output")),
                Err(e) => {
                    error!("Failed to build torch flamegraph: {e}");
                    empty_svg("Unable to build torch flamegraph")
                }
            }
        }
    }
}

fn empty_svg(message: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='800' height='120'>\
         <rect width='100%' height='100%' fill='#f5f5f5'/>\
         <text x='50%' y='50%' dominant-baseline='middle' text-anchor='middle'\
           font-family='sans-serif' font-size='16' fill='#666'>{}</text>\
         </svg>",
        encode_text(message)
    )
}
