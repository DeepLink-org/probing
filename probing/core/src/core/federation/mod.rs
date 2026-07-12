mod aggregate_pushdown;
mod cluster_executor;
mod convert;
mod fanout_scope;
mod federated_scan_exec;
mod global_catalog;
mod global_table;
mod query_guard;
mod rewrite;
mod route;
mod sql_gen;

pub use aggregate_pushdown::{
    plan_federated_aggregate_pushdown, try_execute_aggregate_pushdown, FederatedAggregatePlan,
};
#[cfg(any(test, feature = "test-utils"))]
pub use cluster_executor::set_remote_query_hook;
pub use cluster_executor::{
    check_fanout_strict, enforce_fanout_strict, fanout_stats_partial, fanout_strict_enabled,
    remote_fanout_concurrency, remote_query_timeout, reset_fanout_stats, set_fanout_stats,
    take_fanout_stats, FanoutStats, ProbeClusterExecutor, RemoteFanoutResult,
};
pub use convert::{
    cluster_local_rank_for_endpoint, cluster_node_rank_for_endpoint, cluster_rank_for_endpoint,
    cluster_role_for_endpoint, federated_output_schema, federation_tags_for_endpoint,
    is_federation_tag_column, tag_proto_dataframe, FederationEndpointTags, FEDERATION_TAG_COLUMNS,
    PROBE_ADDR_COL, PROBE_HOST_COL, PROBE_LOCAL_RANK_COL, PROBE_NODE_RANK_COL, PROBE_RANK_COL,
    PROBE_ROLE_COL,
};
pub use fanout_scope::{
    current_fanout_scope, hierarchical_fanout_enabled, is_local0_from_env, resolve_fanout_scope,
    set_fanout_scope, take_fanout_scope, with_fanout_scope, FanoutScope,
};
pub use global_catalog::{install_global_catalog, GLOBAL_CATALOG};
pub use query_guard::{
    cap_materialized_rows, ensure_global_scan_limit, global_scan_max_rows, require_broadcast_limit,
    sql_has_limit, validate_global_query,
};
pub use rewrite::{
    can_fanout_via_global_catalog, ensure_global_node_columns, prepare_global_query,
    rewrite_global_catalog_to_probe, rewrite_sql_for_global_fanout,
};
pub use route::{
    classify_cluster_sql, classify_federated_sql, classify_federated_sql_with_context,
    estimate_federation_cost, explain_federation, explain_physical_plan,
    recommended_fanout_concurrency, FederatedQueryPath, FederationCostEstimate,
    FederationExplainReport, FederationRouteContext,
};
