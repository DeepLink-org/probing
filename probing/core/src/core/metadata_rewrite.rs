//! Rewrite `DESCRIBE` / `SHOW CREATE TABLE` into documented catalog queries.

use datafusion::sql::sqlparser::ast::{DescribeAlias, ObjectName, ShowCreateObject, Statement};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::parser::Parser;

use super::semantic_catalog::{COLUMN_DOCS, DOCS_SCHEMA, TABLE_DOCS};

/// If `sql` is a table metadata statement, return an enriched SELECT; otherwise `None`.
pub fn prepare_metadata_query(sql: &str, default_schema: &str) -> Option<String> {
    let dialect = GenericDialect {};
    let mut stmts = Parser::parse_sql(&dialect, sql).ok()?;
    if stmts.len() != 1 {
        return None;
    }
    match stmts.remove(0) {
        Statement::ExplainTable {
            describe_alias: DescribeAlias::Describe | DescribeAlias::Desc,
            table_name,
            ..
        } => {
            let (schema, table) = object_name_to_ref(&table_name, default_schema);
            Some(describe_table_sql(&schema, &table))
        }
        Statement::ShowCreate {
            obj_type: ShowCreateObject::Table,
            obj_name,
        } => {
            let (schema, table) = object_name_to_ref(&obj_name, default_schema);
            Some(show_create_table_sql(&schema, &table))
        }
        _ => None,
    }
}

fn object_name_to_ref(name: &ObjectName, default_schema: &str) -> (String, String) {
    let mut parts: Vec<String> = name
        .0
        .iter()
        .filter_map(|part| part.as_ident().map(|ident| ident.value.clone()))
        .collect();
    if matches!(
        parts.first().map(|s| s.as_str()),
        Some("probe") | Some("global") | Some("datafusion")
    ) {
        parts.remove(0);
    }
    match parts.as_slice() {
        [] => (default_schema.to_string(), String::new()),
        [table] => (default_schema.to_string(), table.clone()),
        [schema, table] => (schema.clone(), table.clone()),
        [schema, rest @ ..] => (schema.clone(), rest.join(".")),
    }
}

fn sql_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn describe_table_sql(schema: &str, table: &str) -> String {
    format!(
        "SELECT \
            c.column_name, \
            c.data_type, \
            c.is_nullable, \
            cd.description AS comment, \
            td.description AS table_comment \
         FROM information_schema.columns c \
         LEFT JOIN probe.{docs}.{column_docs} cd \
           ON c.table_schema = cd.table_schema \
          AND c.table_name = cd.table_name \
          AND c.column_name = cd.column_name \
         LEFT JOIN probe.{docs}.{table_docs} td \
           ON c.table_schema = td.table_schema \
          AND c.table_name = td.table_name \
         WHERE c.table_schema = {schema} \
           AND c.table_name = {table} \
         ORDER BY c.ordinal_position",
        docs = DOCS_SCHEMA,
        column_docs = COLUMN_DOCS,
        table_docs = TABLE_DOCS,
        schema = sql_literal(schema),
        table = sql_literal(table),
    )
}

fn show_create_table_sql(schema: &str, table: &str) -> String {
    format!(
        "SELECT \
            c.table_schema, \
            c.table_name, \
            max(td.description) AS table_comment, \
            max(td.synonyms) AS synonyms, \
            max(td.notes) AS notes, \
            concat( \
              '-- ', coalesce(max(td.description), ''), '\n', \
              CASE WHEN max(td.notes) IS NOT NULL AND max(td.notes) != '' \
                   THEN concat('-- ', replace(max(td.notes), '\n', '\n-- '), '\n') \
                   ELSE '' END, \
              'CREATE TABLE ', c.table_schema, '.', c.table_name, ' (\n', \
              string_agg( \
                concat( \
                  '  ', c.column_name, ' ', c.data_type, \
                  CASE WHEN cd.description IS NOT NULL AND cd.description != '' \
                       THEN concat('  -- ', cd.description) \
                       ELSE '' END \
                ), \
                ',\n' ORDER BY c.ordinal_position \
              ), \
              '\n);' \
            ) AS create_statement \
         FROM information_schema.columns c \
         LEFT JOIN probe.{docs}.{table_docs} td \
           ON c.table_schema = td.table_schema \
          AND c.table_name = td.table_name \
         LEFT JOIN probe.{docs}.{column_docs} cd \
           ON c.table_schema = cd.table_schema \
          AND c.table_name = cd.table_name \
          AND c.column_name = cd.column_name \
         WHERE c.table_schema = {schema} \
           AND c.table_name = {table} \
         GROUP BY c.table_schema, c.table_name",
        docs = DOCS_SCHEMA,
        column_docs = COLUMN_DOCS,
        table_docs = TABLE_DOCS,
        schema = sql_literal(schema),
        table = sql_literal(table),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_describe_table() {
        let sql = prepare_metadata_query("DESCRIBE hccl.tasks", "probe").unwrap();
        assert!(sql.contains("information_schema.columns"));
        assert!(sql.contains("probing.column_docs"));
        assert!(sql.contains("'hccl'"));
        assert!(sql.contains("'tasks'"));
        assert!(sql.contains("AS comment"));
    }

    #[test]
    fn rewrite_desc_short_form() {
        let sql = prepare_metadata_query("DESC nccl.proxy_ops", "probe").unwrap();
        assert!(sql.contains("'nccl'"));
        assert!(sql.contains("'proxy_ops'"));
    }

    #[test]
    fn rewrite_show_create_table() {
        let sql = prepare_metadata_query("SHOW CREATE TABLE python.torch_trace", "probe").unwrap();
        assert!(sql.contains("create_statement"));
        assert!(sql.contains("string_agg"));
        assert!(sql.contains("'python'"));
        assert!(sql.contains("'torch_trace'"));
    }

    #[test]
    fn skip_explain_query_plan() {
        assert!(prepare_metadata_query("EXPLAIN SELECT 1", "probe").is_none());
    }

    #[test]
    fn skip_regular_select() {
        assert!(prepare_metadata_query("SELECT 1", "probe").is_none());
    }

    #[test]
    fn qualified_with_probe_catalog() {
        let sql = prepare_metadata_query("DESCRIBE probe.hccl.collectives", "probe").unwrap();
        assert!(sql.contains("'hccl'"));
        assert!(sql.contains("'collectives'"));
    }
}
