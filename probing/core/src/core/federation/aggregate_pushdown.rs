//! Push `GROUP BY` / aggregate queries to each probing node, then merge partial
//! results at the coordinator instead of fanning out raw rows.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arrow::compute::concat_batches;
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::catalog::MemTable;
use datafusion::error::{DataFusionError, Result};
use datafusion::prelude::SessionContext;
use datafusion::sql::sqlparser::ast::{
    DuplicateTreatment, Expr, Function, FunctionArguments, GroupByExpr, Ident, ObjectNamePart,
    Query, Select, SelectItem, SetExpr, Statement, TableFactor,
};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;

use crate::core::arrow_convert::arrow_array_to_seq;
use crate::core::Engine;

use super::cluster_executor::{
    reset_fanout_stats, set_fanout_stats, FanoutStats, ProbeClusterExecutor,
};
use super::convert::{
    cluster_rank_for_endpoint, is_federation_tag_column, proto_dataframe_to_record_batch,
    tag_proto_dataframe,
};

static PARTIAL_TABLE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct PlannedAggregate {
    alias: String,
    merge_fn: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedAggregatePlan {
    pub per_node_sql: String,
    pub coordinator_sql: Option<String>,
    /// Suffix applied on the coordinator after merge, e.g. ` ORDER BY avg_ms DESC LIMIT 5`.
    pub post_merge_tail: Option<String>,
    pub inject_tags: bool,
}

pub fn plan_federated_aggregate_pushdown(sql: &str) -> Option<FederatedAggregatePlan> {
    let stmt = parse_single_statement(sql)?;
    let Statement::Query(query) = stmt else {
        return None;
    };
    plan_from_query(&query)
}

pub async fn try_execute_aggregate_pushdown(
    engine: &Engine,
    sql: &str,
) -> Result<Option<probing_proto::prelude::DataFrame>> {
    let plan = match plan_federated_aggregate_pushdown(sql) {
        Some(plan) => plan,
        None => return Ok(None),
    };

    log::debug!(
        "federated aggregate pushdown: per_node={} coordinator={:?}",
        plan.per_node_sql,
        plan.coordinator_sql
    );

    reset_fanout_stats();
    let mut proto_parts = Vec::new();

    let host = ProbeClusterExecutor::local_host_label();
    let addr = ProbeClusterExecutor::local_addr_label();
    let rank = cluster_rank_for_endpoint(&host, &addr);

    let mut local_proto = sql_to_proto_dataframe(engine, &plan.per_node_sql).await?;
    if plan.inject_tags {
        tag_proto_dataframe(&mut local_proto, &host, &addr, rank);
    }
    if !local_proto.is_empty() || plan.coordinator_sql.is_some() {
        proto_parts.push(local_proto);
    }

    let per_node_sql = plan.per_node_sql.clone();
    let outcomes = tokio::task::spawn_blocking(move || {
        ProbeClusterExecutor::fanout_query_to_peers(&per_node_sql)
    })
    .await
    .map_err(|e| DataFusionError::Execution(format!("aggregate fan-out join failed: {e}")))?;

    let mut stats = FanoutStats::default();
    for outcome in outcomes {
        match outcome.result {
            Ok(mut df) => {
                stats.nodes_succeeded += 1;
                if plan.inject_tags {
                    tag_proto_dataframe(&mut df, &outcome.host, &outcome.addr, outcome.rank);
                }
                proto_parts.push(df);
            }
            Err(err) => {
                log::debug!("aggregate pushdown skipped {}: {err}", outcome.addr);
                stats.nodes_failed.push(outcome.addr);
            }
        }
    }
    set_fanout_stats(stats);

    if proto_parts.is_empty() {
        return Ok(None);
    }

    let result = if let Some(merge_sql) = plan.coordinator_sql {
        let batches: Vec<RecordBatch> = proto_parts
            .iter()
            .filter_map(|df| proto_dataframe_to_record_batch(df).ok())
            .collect();
        merge_on_coordinator(&engine.context, &merge_sql, batches).await?
    } else if proto_parts.len() == 1 {
        proto_parts.remove(0)
    } else {
        merge_proto_dataframes(&proto_parts)?
    };

    let result = if let Some(tail) = plan.post_merge_tail {
        apply_post_merge_tail(&engine.context, &tail, result).await?
    } else {
        result
    };

    Ok(Some(result))
}

async fn sql_to_proto_dataframe(
    engine: &Engine,
    sql: &str,
) -> Result<probing_proto::prelude::DataFrame> {
    let batches = engine.sql(sql).await?.collect().await?;
    batches_to_dataframe(batches)
}

fn parse_single_statement(sql: &str) -> Option<Statement> {
    let dialect = GenericDialect {};
    let mut stmts = Parser::parse_sql(&dialect, sql).ok()?;
    if stmts.len() != 1 {
        return None;
    }
    Some(stmts.remove(0))
}

fn plan_from_query(query: &Query) -> Option<FederatedAggregatePlan> {
    let post_merge_tail = build_post_merge_tail(query);
    let SetExpr::Select(select) = query.body.as_ref() else {
        return None;
    };
    if select.from.len() != 1 || !select.lateral_views.is_empty() || select.having.is_some() {
        return None;
    }
    if matches!(select.group_by, GroupByExpr::All(_)) {
        return None;
    }
    let group_exprs = group_by_expressions(select);
    if group_exprs.is_empty() && !select_projection_has_aggregate(select) {
        return None;
    }

    let (schema_name, table_name) = global_table_ref(select)?;
    let mut tag_group = Vec::new();
    let mut data_group = Vec::new();
    for expr in &group_exprs {
        let name = expr_column_name(expr)?;
        if is_federation_tag_column(&name) {
            tag_group.push(name);
        } else {
            data_group.push(name);
        }
    }

    let (aggregates, has_unsafe_distinct) = plan_aggregates(select, &data_group)?;
    if aggregates.is_empty() || has_unsafe_distinct {
        return None;
    }

    let per_node_sql = build_per_node_sql(select, &schema_name, &table_name, &data_group)?;
    let inject_tags = select_mentions_tags(select) || !tag_group.is_empty();
    let coordinator_sql = if data_group.is_empty() && tag_group.is_empty() {
        Some(build_global_merge_sql(&[], &[], &aggregates))
    } else if data_group.is_empty() {
        None
    } else {
        Some(build_global_merge_sql(&data_group, &tag_group, &aggregates))
    };

    Some(FederatedAggregatePlan {
        per_node_sql,
        coordinator_sql,
        post_merge_tail,
        inject_tags,
    })
}

fn global_table_ref(select: &Select) -> Option<(String, String)> {
    let table_with_joins = select.from.first()?;
    if !table_with_joins.joins.is_empty() {
        return None;
    }
    let TableFactor::Table { name, .. } = &table_with_joins.relation else {
        return None;
    };
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(|part| match part {
            ObjectNamePart::Identifier(Ident { value, .. }) => Some(value.clone()),
            ObjectNamePart::Function(_) => None,
        })
        .collect();
    if parts.len() != 3 || !parts[0].eq_ignore_ascii_case("global") {
        return None;
    }
    Some((parts[1].clone(), parts[2].clone()))
}

fn group_by_expressions(select: &Select) -> Vec<Expr> {
    match &select.group_by {
        GroupByExpr::Expressions(exprs, _) => exprs.clone(),
        GroupByExpr::All(_) => Vec::new(),
    }
}

fn select_projection_has_aggregate(select: &Select) -> bool {
    select.projection.iter().any(|item| match item {
        SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
            expr_has_aggregate(expr)
        }
        _ => false,
    })
}

fn select_mentions_tags(select: &Select) -> bool {
    let in_projection = select.projection.iter().any(|item| match item {
        SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
            expr_mentions_tag(expr)
        }
        _ => false,
    });
    let in_group = group_by_expressions(select).iter().any(expr_mentions_tag);
    in_projection || in_group
}

fn expr_mentions_tag(expr: &Expr) -> bool {
    expr_column_name(expr).is_some_and(|name| is_federation_tag_column(&name))
}

fn expr_column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Identifier(Ident { value, .. }) => Some(value.clone()),
        Expr::CompoundIdentifier(parts) => parts.last().map(|i| i.value.clone()),
        _ => None,
    }
}

fn expr_has_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Function(func) => is_aggregate_function(func),
        Expr::Nested(inner) => expr_has_aggregate(inner),
        _ => false,
    }
}

fn is_aggregate_function(func: &Function) -> bool {
    let name = object_name_last(&func.name).to_lowercase();
    matches!(name.as_str(), "count" | "sum" | "min" | "max" | "avg")
}

fn object_name_last(name: &datafusion::sql::sqlparser::ast::ObjectName) -> String {
    name.0
        .last()
        .map(|part| match part {
            ObjectNamePart::Identifier(Ident { value, .. }) => value.clone(),
            ObjectNamePart::Function(f) => f.name.to_string(),
        })
        .unwrap_or_default()
}

fn function_is_distinct(func: &Function) -> bool {
    match &func.args {
        FunctionArguments::List(list) => {
            matches!(list.duplicate_treatment, Some(DuplicateTreatment::Distinct))
        }
        _ => false,
    }
}

fn plan_aggregates(
    select: &Select,
    data_group: &[String],
) -> Option<(Vec<PlannedAggregate>, bool)> {
    let mut aggregates = Vec::new();
    let mut has_unsafe_distinct = false;
    for item in &select.projection {
        let (expr, alias) = match item {
            SelectItem::UnnamedExpr(expr) => (expr, None),
            SelectItem::ExprWithAlias { expr, alias } => (expr, Some(alias.value.clone())),
            _ => continue,
        };
        let Expr::Function(func) = expr else {
            if expr_column_name(expr).is_some() {
                continue;
            }
            return None;
        };
        let distinct = function_is_distinct(func);
        let merge_fn = if distinct {
            if !data_group.is_empty() {
                has_unsafe_distinct = true;
            }
            None
        } else {
            merge_fn_for_function(func)
        };
        if distinct && !data_group.is_empty() {
            continue;
        }
        if !distinct && merge_fn.is_none() {
            return None;
        }
        let alias = alias.unwrap_or_else(|| expr.to_string());
        aggregates.push(PlannedAggregate { alias, merge_fn });
    }
    Some((aggregates, has_unsafe_distinct))
}

fn merge_fn_for_function(func: &Function) -> Option<&'static str> {
    let name = object_name_last(&func.name).to_lowercase();
    match name.as_str() {
        "count" | "sum" => Some("sum"),
        "min" => Some("min"),
        "max" => Some("max"),
        _ => None,
    }
}

fn build_per_node_sql(
    select: &Select,
    schema_name: &str,
    table_name: &str,
    data_group: &[String],
) -> Option<String> {
    let mut select_parts = Vec::new();
    for item in &select.projection {
        match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                if expr_mentions_tag(expr) {
                    continue;
                }
                select_parts.push(expr_to_string(item));
            }
            _ => return None,
        }
    }
    if select_parts.is_empty() {
        return None;
    }

    let mut sql = format!(
        "SELECT {} FROM probe.{schema_name}.{table_name}",
        select_parts.join(", ")
    );
    if let Some(selection) = &select.selection {
        sql.push_str(" WHERE ");
        sql.push_str(&selection.to_string());
    }
    if !data_group.is_empty() {
        sql.push_str(" GROUP BY ");
        sql.push_str(
            &data_group
                .iter()
                .map(|name| quote_ident(name))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    Some(sql)
}

fn expr_to_string(item: &SelectItem) -> String {
    match item {
        SelectItem::UnnamedExpr(expr) => expr.to_string(),
        SelectItem::ExprWithAlias { expr, alias } => {
            format!("{} AS {}", expr, quote_ident(&alias.value))
        }
        _ => String::new(),
    }
}

fn build_global_merge_sql(
    data_group: &[String],
    tag_group: &[String],
    aggregates: &[PlannedAggregate],
) -> String {
    let group_cols: Vec<String> = data_group.iter().chain(tag_group.iter()).cloned().collect();
    let mut select_parts: Vec<String> = group_cols.iter().map(|c| quote_ident(c)).collect();
    for agg in aggregates {
        let merge_fn = agg.merge_fn.unwrap_or("sum");
        select_parts.push(format!(
            "{}({}) AS {}",
            merge_fn,
            quote_ident(&agg.alias),
            quote_ident(&agg.alias)
        ));
    }
    let mut sql = format!("SELECT {} FROM partials", select_parts.join(", "));
    if !group_cols.is_empty() {
        sql.push_str(" GROUP BY ");
        sql.push_str(
            &group_cols
                .iter()
                .map(|c| quote_ident(c))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    sql
}

fn quote_ident(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.is_empty()
        && !name.chars().next().unwrap().is_ascii_digit()
    {
        name.to_string()
    } else {
        format!("\"{name}\"")
    }
}

fn build_post_merge_tail(query: &Query) -> Option<String> {
    let order_by = format_order_by(query);
    let limit = format_limit_clause(query);
    if order_by.is_empty() && limit.is_empty() {
        None
    } else {
        Some(format!("{order_by}{limit}"))
    }
}

fn format_order_by(query: &Query) -> String {
    query
        .order_by
        .as_ref()
        .map(|order_by| format!(" {order_by}"))
        .unwrap_or_default()
}

fn format_limit_clause(query: &Query) -> String {
    let mut out = query
        .limit_clause
        .as_ref()
        .map(|limit| format!(" {limit}"))
        .unwrap_or_default();
    if let Some(fetch) = &query.fetch {
        out.push(' ');
        out.push_str(&fetch.to_string());
    }
    out
}

async fn apply_post_merge_tail(
    ctx: &SessionContext,
    tail: &str,
    df: probing_proto::prelude::DataFrame,
) -> Result<probing_proto::prelude::DataFrame> {
    let batch = proto_dataframe_to_record_batch(&df)?;
    if batch.num_rows() == 0 {
        return Ok(df);
    }
    let sql = format!("SELECT * FROM partials{tail}");
    merge_on_coordinator(ctx, &sql, vec![batch]).await
}

async fn merge_on_coordinator(
    ctx: &SessionContext,
    merge_sql: &str,
    batches: Vec<RecordBatch>,
) -> Result<probing_proto::prelude::DataFrame> {
    if batches.is_empty() {
        return Err(DataFusionError::Plan(
            "aggregate pushdown produced no partial batches".into(),
        ));
    }
    let schema = batches[0].schema();
    let table = MemTable::try_new(schema, vec![batches])?;
    let table_name = format!(
        "partials_{}",
        PARTIAL_TABLE_ID.fetch_add(1, Ordering::Relaxed)
    );
    ctx.register_table(&table_name, Arc::new(table))?;
    let sql = merge_sql.replace("partials", &table_name);
    let out_batches = ctx.sql(&sql).await?.collect().await?;
    let _ = ctx.deregister_table(&table_name);
    batches_to_dataframe(out_batches)
}

fn batches_to_dataframe(batches: Vec<RecordBatch>) -> Result<probing_proto::prelude::DataFrame> {
    if batches.is_empty() {
        return Ok(probing_proto::prelude::DataFrame::default());
    }
    let batch = concat_batches(&batches[0].schema(), batches.iter())?;
    let names = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    let cols = batch.columns().iter().map(arrow_array_to_seq).collect();
    Ok(probing_proto::prelude::DataFrame::new(names, cols))
}

fn merge_proto_dataframes(
    parts: &[probing_proto::prelude::DataFrame],
) -> Result<probing_proto::prelude::DataFrame> {
    let mut out = probing_proto::prelude::DataFrame::default();
    for df in parts {
        if df.is_empty() {
            continue;
        }
        if out.is_empty() {
            out = df.clone();
            continue;
        }
        append_proto_dataframe(&mut out, df)?;
    }
    out.size = out.len() as u64;
    Ok(out)
}

fn append_proto_dataframe(
    base: &mut probing_proto::prelude::DataFrame,
    other: &probing_proto::prelude::DataFrame,
) -> Result<()> {
    use probing_proto::prelude::{Ele, Seq};
    if other.is_empty() {
        return Ok(());
    }
    if base.is_empty() {
        *base = other.clone();
        return Ok(());
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_group_by_rank_count_distinct() {
        let sql = "SELECT _rank, count(distinct name) AS n FROM global.process.envs GROUP BY _rank";
        let plan = plan_federated_aggregate_pushdown(sql).expect("plan");
        assert!(plan.per_node_sql.contains("probe.process.envs"));
        assert!(plan.per_node_sql.to_lowercase().contains("count"));
        assert!(!plan.per_node_sql.contains("_rank"));
        assert!(plan.coordinator_sql.is_none());
        assert!(plan.inject_tags);
    }

    #[test]
    fn plans_group_by_name_count_star() {
        let sql = "SELECT name, count(*) AS n FROM global.process.envs GROUP BY name";
        let plan = plan_federated_aggregate_pushdown(sql).expect("plan");
        assert!(plan.per_node_sql.contains("GROUP BY name"));
        assert!(plan.coordinator_sql.as_ref().unwrap().contains("sum(n)"));
    }

    #[test]
    fn rejects_non_aggregate_scan() {
        let sql = "SELECT name FROM global.process.envs";
        assert!(plan_federated_aggregate_pushdown(sql).is_none());
    }

    #[test]
    fn rejects_count_distinct_grouped_by_data_column() {
        let sql = "SELECT name, count(distinct value) AS n FROM global.process.envs GROUP BY name";
        assert!(plan_federated_aggregate_pushdown(sql).is_none());
    }

    #[test]
    fn plans_order_by_limit_as_post_merge_tail() {
        let sql = "SELECT name, count(*) AS n FROM global.process.envs GROUP BY name ORDER BY n DESC LIMIT 3";
        let plan = plan_federated_aggregate_pushdown(sql).expect("plan");
        assert!(plan.coordinator_sql.is_some());
        let tail = plan.post_merge_tail.as_deref().unwrap();
        assert!(tail.contains("ORDER BY n DESC"));
        assert!(tail.contains("LIMIT 3"));
        assert!(!plan.per_node_sql.to_uppercase().contains("ORDER BY"));
        assert!(!plan.per_node_sql.to_uppercase().contains("LIMIT"));
    }
}
