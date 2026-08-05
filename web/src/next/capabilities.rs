use std::collections::BTreeSet;

use dioxus::prelude::*;
use probing_proto::prelude::{DataFrame, Ele};

use crate::api::ApiClient;
use crate::hooks::use_poll_tick_gated;

const REFRESH_MS: u32 = 15_000;
const CATALOG_SQL: &str = "SELECT table_schema, table_name, column_name \
    FROM information_schema.columns WHERE table_catalog = 'probe'";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityStatus {
    Checking,
    Available,
    Missing,
    CatalogUnavailable,
}

impl CapabilityStatus {
    /// Preserve existing behavior if the catalog itself cannot be read.
    pub fn allows_query(self) -> bool {
        matches!(self, Self::Available | Self::CatalogUnavailable)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CapabilityCatalog {
    columns: BTreeSet<String>,
}

impl CapabilityCatalog {
    fn from_dataframe(dataframe: &DataFrame) -> Self {
        let index = |name: &str| dataframe.names.iter().position(|column| column == name);
        let (Some(schema), Some(table), Some(column)) = (
            index("table_schema"),
            index("table_name"),
            index("column_name"),
        ) else {
            return Self::default();
        };
        let columns = dataframe
            .iter()
            .filter_map(|row| {
                Some(format!(
                    "{}.{}.{}",
                    text(row.get(schema))?,
                    text(row.get(table))?,
                    text(row.get(column))?
                ))
            })
            .collect();
        Self { columns }
    }

    fn supports(&self, schema: &str, table: &str, required: &[&str]) -> bool {
        required
            .iter()
            .all(|column| self.columns.contains(&format!("{schema}.{table}.{column}")))
    }
}

fn text(value: Option<&Ele>) -> Option<&str> {
    match value? {
        Ele::Text(value) | Ele::Url(value) => Some(value),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CapabilityCatalogState {
    Checking,
    Ready(CapabilityCatalog),
    Unavailable,
}

static CAPABILITY_CATALOG: GlobalSignal<CapabilityCatalogState> =
    Signal::global(|| CapabilityCatalogState::Checking);

pub fn capability_status(schema: &str, table: &str, required: &[&str]) -> CapabilityStatus {
    match &*CAPABILITY_CATALOG.read() {
        CapabilityCatalogState::Checking => CapabilityStatus::Checking,
        CapabilityCatalogState::Ready(catalog) if catalog.supports(schema, table, required) => {
            CapabilityStatus::Available
        }
        CapabilityCatalogState::Ready(_) => CapabilityStatus::Missing,
        CapabilityCatalogState::Unavailable => CapabilityStatus::CatalogUnavailable,
    }
}

async fn refresh_capability_catalog() {
    *CAPABILITY_CATALOG.write() = match ApiClient::new().execute_query(CATALOG_SQL).await {
        Ok(dataframe) => {
            CapabilityCatalogState::Ready(CapabilityCatalog::from_dataframe(&dataframe))
        }
        Err(_) => CapabilityCatalogState::Unavailable,
    };
}

/// Async guard for snapshot code that can also run outside the Next shell.
pub async fn capability_available(schema: &str, table: &str, required: &[&str]) -> bool {
    if matches!(
        &*CAPABILITY_CATALOG.read(),
        CapabilityCatalogState::Checking
    ) {
        refresh_capability_catalog().await;
    }
    capability_status(schema, table, required).allows_query()
}

#[component]
pub fn CapabilityCatalogPoller() -> Element {
    let tick = use_poll_tick_gated(REFRESH_MS, None);
    let _catalog = use_resource(move || {
        let _ = tick();
        async move {
            refresh_capability_catalog().await;
        }
    });
    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use probing_proto::prelude::Seq;

    #[test]
    fn required_columns_distinguish_real_tables_from_error_placeholders() {
        let dataframe = DataFrame::new(
            vec![
                "table_schema".into(),
                "table_name".into(),
                "column_name".into(),
            ],
            vec![
                Seq::SeqText(vec!["python".into(), "python".into(), "python".into()]),
                Seq::SeqText(vec![
                    "torch_trace".into(),
                    "torch_trace".into(),
                    "comm_collective".into(),
                ]),
                Seq::SeqText(vec!["rank".into(), "local_step".into(), "_error".into()]),
            ],
        );
        let catalog = CapabilityCatalog::from_dataframe(&dataframe);

        assert!(catalog.supports("python", "torch_trace", &["rank", "local_step"]));
        assert!(!catalog.supports("python", "comm_collective", &["rank", "duration_ms"]));
    }
}
