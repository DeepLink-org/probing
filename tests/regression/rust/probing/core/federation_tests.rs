//! Regression tests for the `global` federated catalog path:
//! probe catalog (local) vs global catalog (fan-out + `_addr` / `_rank` tagging).

use std::sync::Arc;

use probing_core::core::cluster::{reset_cluster_for_tests, update_node};
use probing_core::core::federation::{
    set_remote_query_hook, take_fanout_stats, FEDERATION_TAG_COLUMNS, GLOBAL_CATALOG,
    PROBE_ADDR_COL, PROBE_HOST_COL, PROBE_LOCAL_RANK_COL, PROBE_NODE_RANK_COL, PROBE_RANK_COL,
    PROBE_ROLE_COL,
};
use probing_core::core::{Engine, ProbeDataSource};
use probing_proto::prelude::{Node, Seq};
use probing_rust_regression::test_helpers::{federation_test_lock, GenericTableProbeDataSource};

fn df_col_i32(df: &probing_proto::prelude::DataFrame, name: &str) -> Vec<i32> {
    let idx = df
        .names
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("column {name} missing from {:?}", df.names));
    match &df.cols[idx] {
        Seq::SeqI32(v) => v.clone(),
        other => panic!("column {name} expected SeqI32, got {other:?}"),
    }
}

fn df_col_i64(df: &probing_proto::prelude::DataFrame, name: &str) -> Vec<i64> {
    let idx = df
        .names
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("column {name} missing from {:?}", df.names));
    match &df.cols[idx] {
        Seq::SeqI64(v) => v.clone(),
        Seq::SeqI32(v) => v.iter().map(|&x| i64::from(x)).collect(),
        other => panic!("column {name} expected integer column, got {other:?}"),
    }
}

#[allow(dead_code)]
fn df_col_str(df: &probing_proto::prelude::DataFrame, name: &str) -> Vec<String> {
    let idx = df
        .names
        .iter()
        .position(|n| n == name)
        .unwrap_or_else(|| panic!("column {name} missing from {:?}", df.names));
    match &df.cols[idx] {
        Seq::SeqText(v) => v.clone(),
        other => panic!("column {name} expected SeqText, got {other:?}"),
    }
}

async fn build_demo_engine() -> Engine {
    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let metrics =
        GenericTableProbeDataSource::single_column_table("metrics", "demo", "rank", vec![0, 1, 2]);
    Engine::builder()
        .with_data_source(Arc::new(metrics) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine build")
}

fn register_local_node(rank: i32, addr: &str, host: &str) {
    update_node(Node {
        host: host.into(),
        addr: addr.into(),
        rank: Some(rank),
        group_rank: Some(rank / 8),
        local_rank: Some(rank % 8),
        role: Some(format!("dp={rank}")),
        ..Default::default()
    });
}

struct FederatedTestCluster {
    local_engine: Engine,
    #[allow(dead_code)]
    peer_engine: Engine,
    #[allow(dead_code)]
    peer_addr: String,
}

impl FederatedTestCluster {
    async fn setup(local_values: Vec<i32>, peer_values: Vec<i32>) -> Self {
        reset_cluster_for_tests();
        set_remote_query_hook(None);

        let local_addr = "127.0.0.1:19999";
        let peer_addr = "127.0.0.1:20001".to_string();
        std::env::set_var("PROBING_ADDRESS", local_addr);
        std::env::set_var("HOSTNAME", "coord-host");

        register_local_node(0, local_addr, "coord-host");
        update_node(Node {
            host: "peer-host".into(),
            addr: peer_addr.clone(),
            rank: Some(1),
            group_rank: Some(0),
            local_rank: Some(1),
            role: Some("dp=1".into()),
            ..Default::default()
        });

        let local_table =
            GenericTableProbeDataSource::single_column_table("metrics", "demo", "v", local_values);
        let local_engine = Engine::builder()
            .with_data_source(Arc::new(local_table) as Arc<dyn ProbeDataSource + Send + Sync>)
            .build()
            .await
            .expect("local engine");

        let peer_table =
            GenericTableProbeDataSource::single_column_table("metrics", "demo", "v", peer_values);
        let peer_engine = Engine::builder()
            .with_data_source(Arc::new(peer_table) as Arc<dyn ProbeDataSource + Send + Sync>)
            .build()
            .await
            .expect("peer engine");

        let peer_for_hook = peer_engine.clone();
        let peer_addr_for_hook = peer_addr.clone();
        set_remote_query_hook(Some(Box::new(move |addr, sql| {
            if addr != peer_addr_for_hook {
                return Err(datafusion::error::DataFusionError::Execution(format!(
                    "unexpected peer addr: {addr}"
                )));
            }
            futures::executor::block_on(async {
                peer_for_hook.async_query(sql).await?.ok_or_else(|| {
                    datafusion::error::DataFusionError::Execution("peer query returned None".into())
                })
            })
        })));

        Self {
            local_engine,
            peer_engine,
            peer_addr,
        }
    }

    fn teardown(&self) {
        set_remote_query_hook(None);
        reset_cluster_for_tests();
    }
}

#[tokio::test]
async fn global_catalog_discovers_probe_schema() {
    let _lock = federation_test_lock().await;
    let engine = build_demo_engine().await;
    let global = engine
        .context
        .catalog(GLOBAL_CATALOG)
        .expect("global catalog should be registered");
    assert!(global.schema("demo").is_some());
    let schema = global.schema("demo").unwrap();
    assert!(schema.table_exist("metrics"));
}

#[tokio::test]
async fn global_catalog_discovers_tables_registered_after_build() {
    let _lock = federation_test_lock().await;
    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let engine = Engine::builder().build().await.expect("engine build");
    let late = GenericTableProbeDataSource::single_column_table("late", "demo", "v", vec![42]);
    engine
        .enable(Arc::new(late) as Arc<dyn ProbeDataSource + Send + Sync>)
        .await
        .expect("enable late table");

    let global = engine
        .context
        .catalog(GLOBAL_CATALOG)
        .expect("global catalog");
    let schema = global.schema("demo").expect("demo schema");
    assert!(schema.table_exist("late"));

    let df = engine
        .async_query("SELECT v FROM global.demo.late")
        .await
        .expect("query")
        .expect("dataframe");
    assert_eq!(df_col_i32(&df, "v"), vec![42]);
    assert_eq!(df.names, vec!["v".to_string()]);
}

#[tokio::test]
async fn probe_query_has_no_probe_addr_column() {
    let _lock = federation_test_lock().await;
    let engine = build_demo_engine().await;
    let df = engine
        .async_query("SELECT rank FROM probe.demo.metrics ORDER BY rank")
        .await
        .expect("query")
        .expect("dataframe");
    assert!(!df.names.iter().any(|n| n == PROBE_ADDR_COL));
    assert_eq!(df_col_i32(&df, "rank"), vec![0, 1, 2]);
}

#[tokio::test]
async fn global_explicit_column_select_omits_probe_tags() {
    let _lock = federation_test_lock().await;
    let engine = build_demo_engine().await;
    let df = engine
        .async_query("SELECT rank FROM global.demo.metrics ORDER BY rank")
        .await
        .expect("query")
        .expect("dataframe");
    assert_eq!(df.names, vec!["rank".to_string()]);
    assert_eq!(df_col_i32(&df, "rank"), vec![0, 1, 2]);
}

#[tokio::test]
async fn global_query_filter_pushdown_preserves_explicit_projection() {
    let _lock = federation_test_lock().await;
    let engine = build_demo_engine().await;
    let df = engine
        .async_query("SELECT rank FROM global.demo.metrics WHERE rank = 1")
        .await
        .expect("query")
        .expect("dataframe");
    assert_eq!(df.names, vec!["rank".to_string()]);
    assert_eq!(df_col_i32(&df, "rank"), vec![1]);
}

#[tokio::test]
async fn global_and_probe_return_same_ranks_without_peers() {
    let _lock = federation_test_lock().await;
    let engine = build_demo_engine().await;
    let probe_df = engine
        .async_query("SELECT rank FROM probe.demo.metrics ORDER BY rank")
        .await
        .expect("probe query")
        .expect("probe dataframe");
    let global_df = engine
        .async_query("SELECT rank FROM global.demo.metrics ORDER BY rank")
        .await
        .expect("global query")
        .expect("global dataframe");
    assert_eq!(
        df_col_i32(&probe_df, "rank"),
        df_col_i32(&global_df, "rank")
    );
}

#[tokio::test]
async fn global_select_name_returns_only_name() {
    let _lock = federation_test_lock().await;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;

    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let schema = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["PATH"])),
            Arc::new(StringArray::from(vec!["/bin"])),
        ],
    )
    .unwrap();
    let envs = GenericTableProbeDataSource::new("envs", "process", schema, vec![batch]);
    let engine = Engine::builder()
        .with_data_source(Arc::new(envs) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine build");

    let df = engine
        .async_query("SELECT name FROM global.process.envs")
        .await
        .expect("query")
        .expect("dataframe");
    assert_eq!(df.names, vec!["name".to_string()]);
}

#[tokio::test]
async fn global_empty_table_with_timestamp_explicit_select_preserves_schema() {
    let _lock = federation_test_lock().await;
    use arrow::array::{StringArray, TimestampMicrosecondArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use arrow::record_batch::RecordBatch;

    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let schema = Arc::new(Schema::new(vec![
        Field::new("host", DataType::Utf8, false),
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(Vec::<&str>::new())),
            Arc::new(TimestampMicrosecondArray::from(Vec::<i64>::new())),
        ],
    )
    .unwrap();
    let nodes = GenericTableProbeDataSource::new("nodes", "cluster", schema, vec![batch]);
    let engine = Engine::builder()
        .with_data_source(Arc::new(nodes) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine build");

    let df = engine
        .async_query("SELECT host FROM global.cluster.nodes LIMIT 5")
        .await
        .expect("query should not error")
        .expect("empty global explicit select should return schema, not None");
    assert_eq!(df.names, vec!["host".to_string()]);
    assert_eq!(df.len(), 0);
}

#[tokio::test]
async fn global_empty_table_explicit_select_preserves_schema() {
    let _lock = federation_test_lock().await;
    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let empty = GenericTableProbeDataSource::empty_table("nodes", "cluster");
    let engine = Engine::builder()
        .with_data_source(Arc::new(empty) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine build");

    let df = engine
        .async_query("SELECT id FROM global.cluster.nodes")
        .await
        .expect("query should not error")
        .expect("empty global explicit select should return schema, not None");
    assert_eq!(df.names, vec!["id".to_string()]);
    assert_eq!(df.len(), 0);
}

#[tokio::test]
async fn global_select_star_includes_probe_addr_and_rank() {
    let _lock = federation_test_lock().await;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;

    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let schema = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["PATH"])),
            Arc::new(StringArray::from(vec!["/bin"])),
        ],
    )
    .unwrap();
    let envs = GenericTableProbeDataSource::new("envs", "process", schema, vec![batch]);
    let engine = Engine::builder()
        .with_data_source(Arc::new(envs) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine build");

    let df = engine
        .async_query("SELECT * FROM global.process.envs")
        .await
        .expect("query")
        .expect("dataframe");
    assert_eq!(
        df.names,
        vec![
            "name".to_string(),
            "value".to_string(),
            PROBE_HOST_COL.to_string(),
            PROBE_ADDR_COL.to_string(),
            PROBE_RANK_COL.to_string(),
            PROBE_NODE_RANK_COL.to_string(),
            PROBE_LOCAL_RANK_COL.to_string(),
            PROBE_ROLE_COL.to_string(),
        ]
    );
    assert_eq!(df.names.len(), 2 + FEDERATION_TAG_COLUMNS.len());
}

#[tokio::test]
async fn explicit_probe_tags_not_duplicated() {
    let _lock = federation_test_lock().await;
    let engine = build_demo_engine().await;
    let df = engine
        .async_query("SELECT rank, _addr, _rank FROM global.demo.metrics ORDER BY rank")
        .await
        .expect("query")
        .expect("dataframe");
    let addr_cols = df.names.iter().filter(|n| *n == PROBE_ADDR_COL).count();
    let rank_cols = df.names.iter().filter(|n| *n == PROBE_RANK_COL).count();
    assert_eq!(addr_cols, 1);
    assert_eq!(rank_cols, 1);
}

#[test]
fn cluster_fanout_sql_pipeline_for_single_table() {
    use probing_core::core::federation::{
        can_fanout_via_global_catalog, prepare_global_query, rewrite_sql_for_global_fanout,
        PROBE_ADDR_COL, PROBE_RANK_COL,
    };

    let user = "SELECT rank FROM python.comm_collective LIMIT 20";
    assert!(can_fanout_via_global_catalog(user));
    let global_sql = rewrite_sql_for_global_fanout(user);
    let prepared = prepare_global_query(&global_sql);
    assert!(prepared.contains("global.python.comm_collective"));
    assert!(!prepared.contains(PROBE_ADDR_COL));
    assert!(!prepared.contains(PROBE_RANK_COL));
}

#[test]
fn cluster_fanout_join_uses_legacy_broadcast() {
    use probing_core::core::federation::can_fanout_via_global_catalog;

    let sql = "SELECT a.x FROM python.a JOIN python.b ON a.id = b.id";
    assert!(!can_fanout_via_global_catalog(sql));
}

#[tokio::test]
async fn global_select_star_exclude_rewrite_works() {
    let _lock = federation_test_lock().await;
    use probing_core::core::federation::prepare_global_query;

    let sql = "SELECT * FROM global.process.envs";
    let prepared = prepare_global_query(sql);
    assert!(prepared.contains("EXCLUDE"));
    assert!(prepared.contains(PROBE_ADDR_COL));
    assert!(prepared.contains(PROBE_RANK_COL));
}

#[tokio::test]
async fn global_select_probe_rank_only_returns_requested_column() {
    let _lock = federation_test_lock().await;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;

    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let schema = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["PATH"])),
            Arc::new(StringArray::from(vec!["/bin"])),
        ],
    )
    .unwrap();
    let envs = GenericTableProbeDataSource::new("envs", "process", schema, vec![batch]);
    let engine = Engine::builder()
        .with_data_source(Arc::new(envs) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .unwrap();

    let df = engine
        .async_query("SELECT _rank FROM global.process.envs")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(df.names, vec![PROBE_RANK_COL.to_string()]);
}

#[tokio::test]
async fn global_group_by_rank_with_count_distinct() {
    let _lock = federation_test_lock().await;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;

    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "federation-test-host");

    let schema = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["PATH", "HOME"])),
            Arc::new(StringArray::from(vec!["/bin", "/home"])),
        ],
    )
    .unwrap();
    let envs = GenericTableProbeDataSource::new("envs", "process", schema, vec![batch]);
    let engine = Engine::builder()
        .with_data_source(Arc::new(envs) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine build");

    let df = engine
        .async_query(
            "SELECT _rank, count(distinct name) AS n FROM global.process.envs GROUP BY _rank",
        )
        .await
        .expect("query")
        .expect("dataframe");
    assert!(df.names.iter().any(|n| n == "_rank"));
    assert!(df.names.iter().any(|n| n == "n"));
}

#[tokio::test]
async fn aggregate_pushdown_merges_local_and_peer_sums() {
    let _lock = federation_test_lock().await;
    let cluster = FederatedTestCluster::setup(vec![1, 2, 3], vec![4, 5]).await;

    let df = cluster
        .local_engine
        .async_query("SELECT sum(v) AS total FROM global.demo.metrics")
        .await
        .expect("query")
        .expect("dataframe");

    assert_eq!(df_col_i64(&df, "total"), vec![15]);
    let stats = take_fanout_stats();
    assert_eq!(stats.nodes_succeeded, 1);
    assert!(stats.nodes_failed.is_empty());

    cluster.teardown();
}

#[tokio::test]
async fn aggregate_pushdown_groups_by_host_with_six_tags() {
    let _lock = federation_test_lock().await;
    let cluster = FederatedTestCluster::setup(vec![10, 20], vec![100]).await;

    let df = cluster
        .local_engine
        .async_query(
            "SELECT _host, sum(v) AS total FROM global.demo.metrics GROUP BY _host ORDER BY total DESC",
        )
        .await
        .expect("query")
        .expect("dataframe");

    assert!(df.names.iter().any(|n| n == "_host"));
    assert!(df.names.iter().any(|n| n == "total"));
    let addrs = df_col_str(&df, "_addr");
    assert_eq!(addrs, vec!["127.0.0.1:20001", "127.0.0.1:19999"]);
    assert_eq!(df_col_i64(&df, "total"), vec![100, 30]);

    cluster.teardown();
}

#[tokio::test]
async fn federated_scan_concatenates_local_and_peer_rows_with_tags() {
    let _lock = federation_test_lock().await;
    let cluster = FederatedTestCluster::setup(vec![1, 2], vec![3]).await;

    let df = cluster
        .local_engine
        .async_query("SELECT v, _host FROM global.demo.metrics ORDER BY v")
        .await
        .expect("query")
        .expect("dataframe");

    assert_eq!(df_col_i32(&df, "v"), vec![1, 2, 3]);
    assert!(df.names.iter().any(|n| n == "_host"));
    let hosts = df_col_str(&df, "_host");
    assert_eq!(hosts, vec!["coord-host", "coord-host", "peer-host"]);

    let stats = take_fanout_stats();
    assert_eq!(stats.nodes_succeeded, 1);

    cluster.teardown();
}

#[tokio::test]
async fn federated_scan_global_limit_with_peer() {
    let _lock = federation_test_lock().await;
    let cluster = FederatedTestCluster::setup(vec![1, 2, 3], vec![4, 5, 6]).await;

    let df = cluster
        .local_engine
        .async_query("SELECT v FROM global.demo.metrics ORDER BY v LIMIT 4")
        .await
        .expect("query")
        .expect("dataframe");

    assert_eq!(df_col_i32(&df, "v"), vec![1, 2, 3, 4]);

    cluster.teardown();
}

#[tokio::test]
async fn aggregate_pushdown_order_by_limit_post_merge() {
    let _lock = federation_test_lock().await;
    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;

    reset_cluster_for_tests();
    set_remote_query_hook(None);
    std::env::set_var("PROBING_ADDRESS", "127.0.0.1:19999");
    std::env::set_var("HOSTNAME", "coord-host");

    let schema = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["a", "a", "a", "b"])),
            Arc::new(StringArray::from(vec!["1", "1", "1", "2"])),
        ],
    )
    .unwrap();
    let envs = GenericTableProbeDataSource::new("envs", "process", schema, vec![batch]);
    let engine = Engine::builder()
        .with_data_source(Arc::new(envs) as Arc<dyn ProbeDataSource + Send + Sync>)
        .build()
        .await
        .expect("engine build");

    let df = engine
        .async_query(
            "SELECT name, count(*) AS n FROM global.process.envs GROUP BY name ORDER BY n DESC LIMIT 2",
        )
        .await
        .expect("query")
        .expect("dataframe");

    assert_eq!(df_col_str(&df, "name"), vec!["a", "b"]);
    assert_eq!(df_col_i64(&df, "n"), vec![3, 1]);
}
