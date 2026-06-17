use std::sync::Arc;

use async_trait::async_trait;
use datafusion::catalog::{CatalogProvider, SchemaProvider};
use datafusion::datasource::TableProvider;
use datafusion::error::Result;
use datafusion::execution::context::SessionContext;

use super::global_table::GlobalFederatedTable;

pub const GLOBAL_CATALOG: &str = "global";
const SKIP_SCHEMAS: &[&str] = &["information_schema"];

/// Register a `global` catalog that always delegates to the live `probe` catalog.
///
/// Schemas and tables are discovered on demand at query time, so tables registered
/// after engine build (e.g. new Python extensions or memtable files) are visible
/// under `global.*` without refreshing or rebuilding the catalog.
pub fn install_global_catalog(ctx: &SessionContext) -> Result<()> {
    let shared_ctx = Arc::new(ctx.clone());
    ctx.register_catalog(
        GLOBAL_CATALOG,
        Arc::new(DynamicGlobalCatalog::new(shared_ctx)),
    );
    Ok(())
}

/// Read-only view over `probe` that exposes federated wrappers for every table.
struct DynamicGlobalCatalog {
    ctx: Arc<SessionContext>,
}

impl std::fmt::Debug for DynamicGlobalCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicGlobalCatalog")
            .field("catalog", &GLOBAL_CATALOG)
            .finish_non_exhaustive()
    }
}

impl DynamicGlobalCatalog {
    fn new(ctx: Arc<SessionContext>) -> Self {
        Self { ctx }
    }

    fn probe_catalog(&self) -> Option<Arc<dyn CatalogProvider>> {
        self.ctx.catalog("probe")
    }
}

impl CatalogProvider for DynamicGlobalCatalog {
    fn schema_names(&self) -> Vec<String> {
        let Some(probe) = self.probe_catalog() else {
            return Vec::new();
        };
        probe
            .schema_names()
            .into_iter()
            .filter(|name| !SKIP_SCHEMAS.contains(&name.as_str()))
            .collect()
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        if SKIP_SCHEMAS.contains(&name) {
            return None;
        }
        let probe = self.probe_catalog()?;
        let inner = probe.schema(name)?;
        Some(Arc::new(GlobalSchemaProvider::new(
            name.to_string(),
            inner,
        )))
    }
}

/// Lazily wraps each `probe` table as a [`GlobalFederatedTable`] on access.
#[derive(Debug)]
struct GlobalSchemaProvider {
    schema_name: String,
    inner: Arc<dyn SchemaProvider>,
}

impl GlobalSchemaProvider {
    fn new(schema_name: String, inner: Arc<dyn SchemaProvider>) -> Self {
        Self {
            schema_name,
            inner,
        }
    }
}

#[async_trait]
impl SchemaProvider for GlobalSchemaProvider {
    fn table_names(&self) -> Vec<String> {
        self.inner.table_names()
    }

    async fn table(&self, name: &str) -> Result<Option<Arc<dyn TableProvider>>> {
        let Some(local) = self.inner.table(name).await? else {
            return Ok(None);
        };
        Ok(Some(Arc::new(GlobalFederatedTable::new(
            &self.schema_name,
            name,
            local,
        ))))
    }

    fn table_exist(&self, name: &str) -> bool {
        self.inner.table_exist(name)
    }
}
