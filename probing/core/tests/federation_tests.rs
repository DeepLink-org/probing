//! Regression tests for the `global` federated catalog path:
//! probe catalog (local) vs global catalog (fan-out + `_addr` / `_rank` tagging).

mod test_helpers;

use std::sync::Arc;

use probing_core::core::federation::{
    GLOBAL_CATALOG, PROBE_ADDR_COL, PROBE_HOST_COL, PROBE_RANK_COL,
};
use probing_core::core::{Engine, ProbeDataSource};
use probing_proto::prelude::Seq;
use test_helpers::GenericTableProbeDataSource;

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

#[tokio::test]
async fn global_catalog_discovers_probe_schema() {
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
async fn global_select_star_includes_probe_addr_and_rank() {
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
        ]
    );
}

#[tokio::test]
async fn explicit_probe_tags_not_duplicated() {
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
    use probing_core::core::federation::prepare_global_query;

    let sql = "SELECT * FROM global.process.envs";
    let prepared = prepare_global_query(sql);
    assert!(prepared.contains("EXCLUDE"));
    assert!(prepared.contains(PROBE_ADDR_COL));
    assert!(prepared.contains(PROBE_RANK_COL));
}

#[tokio::test]
async fn global_select_probe_rank_only_returns_requested_column() {
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
