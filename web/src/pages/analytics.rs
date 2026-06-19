use dioxus::html::events::KeyboardEvent;
use dioxus::html::input_data::keyboard_types::Key;
use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::card::Card;
use crate::components::colors::colors;
use crate::components::common::{query_result, AppErrorDisplay, AsyncBoundary, LoadingState};
use crate::components::dataframe_view::DataFrameView;
use crate::components::icon::Icon;
use crate::components::page::{PageContainer, PageTitle};
use crate::hooks::use_app_resource;
use crate::utils::error::AppError;
use probing_proto::prelude::{DataFrame, Ele};

const HIDDEN_SCHEMAS: &[&str] = &["information_schema"];

#[derive(Clone, PartialEq, Eq)]
struct TableEntry {
    schema: String,
    table: String,
}

impl TableEntry {
    fn fqtn(&self, global_mode: bool) -> String {
        if global_mode {
            format!("global.{}.{}", self.schema, self.table)
        } else {
            format!("{}.{}", self.schema, self.table)
        }
    }

    fn preview_sql(&self, global_mode: bool) -> String {
        format!("SELECT * FROM {} LIMIT 10", self.fqtn(global_mode))
    }
}

fn catalog_sql(global_mode: bool) -> String {
    if global_mode {
        "SELECT table_schema, table_name FROM information_schema.tables \
         WHERE table_catalog = 'global' \
         ORDER BY table_schema, table_name"
            .to_string()
    } else {
        format!(
            "SELECT table_schema, table_name FROM information_schema.tables \
             WHERE table_catalog = 'probe' AND table_schema NOT IN ({}) \
             ORDER BY table_schema, table_name",
            HIDDEN_SCHEMAS
                .iter()
                .map(|s| format!("'{s}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn example_sql(global_mode: bool) -> &'static str {
    if global_mode {
        "SELECT * FROM global.python.comm_collective LIMIT 10"
    } else {
        "SELECT * FROM python.backtrace LIMIT 10"
    }
}

#[component]
pub fn Analytics() -> Element {
    let global_mode = use_signal(|| false);
    let mut sql = use_signal(String::new);
    let mut selected_table = use_signal(|| None::<String>);
    let mut preview_title = use_signal(String::new);
    let mut preview_open = use_signal(|| false);
    let mut preview =
        use_action(
            |fqtn: String| async move { ApiClient::new().execute_preview_last10(&fqtn).await },
        );

    let select_for_query = move |entry: TableEntry| {
        let global = global_mode();
        let fqtn = entry.fqtn(global);
        selected_table.set(Some(fqtn.clone()));
        sql.set(entry.preview_sql(global));
    };

    let open_preview = move |entry: TableEntry| {
        let global = global_mode();
        let fqtn = entry.fqtn(global);
        preview_title.set(format!("{fqtn} • latest 10 rows"));
        preview_open.set(true);
        preview.call(fqtn);
    };

    rsx! {
        PageContainer {
            PageTitle {
                title: "Analytics".to_string(),
                subtitle: Some("Browse tables, compose SQL, and inspect results".to_string()),
                icon: Some(&icondata::AiAreaChartOutlined),
            }

            div { class: "grid grid-cols-1 lg:grid-cols-12 gap-4 items-start",
                div { class: "lg:col-span-4 min-w-0",
                    Card {
                        title: "Catalog",
                        content_class: Some("p-0"),
                        header_right: Some(rsx! {
                            GlobalModeToggle {
                                global_mode,
                                on_change: move |_| {
                                    selected_table.set(None);
                                },
                            }
                        }),
                        AsyncBoundary {
                            message: Some("Loading tables...".to_string()),
                            TableCatalog {
                                global_mode,
                                selected_table,
                                on_select: select_for_query,
                                on_preview: open_preview,
                            }
                        }
                    }
                }

                div { class: "lg:col-span-8 min-w-0",
                    Card {
                        title: "SQL Editor",
                        content_class: Some("p-4"),
                        SqlEditorPanel {
                            global_mode,
                            sql,
                            selected_table,
                            on_clear_selection: move |_| selected_table.set(None),
                        }
                    }
                }
            }

            if *preview_open.read() {
                PreviewModal {
                    title: preview_title(),
                    preview,
                    on_close: move |_| preview_open.set(false),
                    on_use_in_editor: move |query: String| {
                        sql.set(query);
                        preview_open.set(false);
                    },
                }
            }
        }
    }
}

fn column_index(df: &DataFrame, candidates: &[&str]) -> Option<usize> {
    candidates.iter().find_map(|name| {
        df.names
            .iter()
            .position(|col| col.eq_ignore_ascii_case(name))
    })
}

fn parse_tables(df: &DataFrame) -> Vec<TableEntry> {
    let schema_idx = column_index(df, &["table_schema", "schema"]).unwrap_or(0);
    let table_idx = column_index(df, &["table_name", "table"]).unwrap_or(1);
    let nrows = df.cols.first().map(|c| c.len()).unwrap_or(0);

    (0..nrows)
        .filter_map(|row| {
            let schema = match df.cols.get(schema_idx).map(|c| c.get(row)) {
                Some(Ele::Text(name)) => name.to_string(),
                _ => return None,
            };
            let table = match df.cols.get(table_idx).map(|c| c.get(row)) {
                Some(Ele::Text(name)) => name.to_string(),
                _ => return None,
            };
            if HIDDEN_SCHEMAS
                .iter()
                .any(|hidden| hidden.eq_ignore_ascii_case(&schema))
            {
                return None;
            }
            Some(TableEntry { schema, table })
        })
        .collect()
}

fn filter_tables(entries: &[TableEntry], query: &str, global_mode: bool) -> Vec<TableEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return entries.to_vec();
    }
    entries
        .iter()
        .filter(|e| {
            e.schema.to_lowercase().contains(&q)
                || e.table.to_lowercase().contains(&q)
                || e.fqtn(global_mode).to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}

#[component]
fn GlobalModeToggle(mut global_mode: Signal<bool>, on_change: EventHandler<()>) -> Element {
    let active = global_mode();
    rsx! {
        div {
            class: "inline-flex items-center rounded-lg border border-gray-200 bg-gray-50 p-0.5 text-xs",
            title: "Local probe tables vs cluster-wide global.* federation",
            button {
                class: if !active {
                    "px-2.5 py-1 rounded-md bg-white shadow-sm font-medium text-gray-800"
                } else {
                    "px-2.5 py-1 rounded-md font-medium text-gray-500 hover:text-gray-700"
                },
                onclick: move |_| {
                    if global_mode() {
                        global_mode.set(false);
                        on_change.call(());
                    }
                },
                "Local"
            }
            button {
                class: if active {
                    "px-2.5 py-1 rounded-md bg-violet-100 shadow-sm font-medium text-violet-800"
                } else {
                    "px-2.5 py-1 rounded-md font-medium text-gray-500 hover:text-gray-700"
                },
                onclick: move |_| {
                    if !global_mode() {
                        global_mode.set(true);
                        on_change.call(());
                    }
                },
                "global.*"
            }
        }
    }
}

#[component]
fn TableCatalog(
    global_mode: Signal<bool>,
    selected_table: Signal<Option<String>>,
    on_select: EventHandler<TableEntry>,
    on_preview: EventHandler<TableEntry>,
) -> Element {
    let mut filter = use_signal(String::new);
    let tables = use_app_resource(move || {
        let query = catalog_sql(global_mode());
        async move { ApiClient::new().execute_query(&query).await }
    });
    let df = tables.suspend()?();
    let global = global_mode();

    query_result(
        df,
        |df| df.cols.is_empty(),
        if global {
            "No global tables found."
        } else {
            "No tables found."
        },
        move |df| {
            let all = parse_tables(&df);
            let filtered = filter_tables(&all, &filter(), global);
            rsx! {
                div { class: "border-b border-gray-200 px-3 py-2.5 bg-gray-50/80",
                    div { class: "relative",
                        span { class: "absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400 pointer-events-none",
                            Icon { icon: &icondata::AiSearchOutlined, class: "w-4 h-4" }
                        }
                        input {
                            r#type: "text",
                            class: "w-full pl-8 pr-3 py-2 text-sm rounded-md border border-gray-300 bg-white focus:outline-none focus:ring-2 focus:ring-blue-500/30 focus:border-blue-500",
                            placeholder: "Filter schema or table…",
                            value: "{filter}",
                            oninput: move |ev| filter.set(ev.value()),
                        }
                    }
                    p { class: "mt-1.5 text-xs text-gray-500",
                        "{filtered.len()} of {all.len()} tables · click to load query · "
                        if global {
                            span { class: "text-violet-700", "cluster fan-out via global.*" }
                        } else {
                            span { class: "text-gray-400", "eye icon to preview" }
                        }
                    }
                }
                if filtered.is_empty() {
                    div { class: "px-4 py-10 text-center text-sm text-gray-500",
                        "No tables match \"{filter}\""
                    }
                } else {
                    div { class: "max-h-[min(28rem,50vh)] overflow-y-auto divide-y divide-gray-100",
                        for entry in filtered {
                            {
                                let fqtn = entry.fqtn(global);
                                let is_selected = selected_table.read().as_deref() == Some(fqtn.as_str());
                                let entry_select = entry.clone();
                                let entry_preview = entry.clone();
                                let schema_badge_class = if global {
                                    "shrink-0 text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded border bg-violet-50 text-violet-800 border-violet-200".to_string()
                                } else {
                                    format!(
                                        "shrink-0 text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded border bg-{} text-{} border-{}",
                                        colors::CONTENT_ACCENT_BG,
                                        colors::CONTENT_ACCENT_TEXT,
                                        colors::CONTENT_ACCENT_BORDER,
                                    )
                                };
                                rsx! {
                                    div {
                                        class: if is_selected {
                                            if global {
                                                "flex items-center gap-2 px-3 py-2.5 bg-violet-50/80"
                                            } else {
                                                "flex items-center gap-2 px-3 py-2.5 bg-blue-50/80"
                                            }
                                        } else {
                                            "flex items-center gap-2 px-3 py-2.5 hover:bg-gray-50 transition-colors"
                                        },
                                        button {
                                            class: "flex-1 min-w-0 text-left",
                                            onclick: move |_| on_select.call(entry_select.clone()),
                                            div { class: "flex items-center gap-2 min-w-0",
                                                if global {
                                                    span { class: "shrink-0 font-mono text-[10px] text-violet-700", "global" }
                                                }
                                                span {
                                                    class: "{schema_badge_class}",
                                                    "{entry.schema}"
                                                }
                                                span { class: "font-mono text-sm text-gray-900 truncate", "{entry.table}" }
                                            }
                                        }
                                        button {
                                            class: "shrink-0 p-1.5 rounded-md text-gray-400 hover:text-blue-600 hover:bg-blue-50 transition-colors",
                                            title: "Preview latest 10 rows",
                                            onclick: move |_| on_preview.call(entry_preview.clone()),
                                            Icon { icon: &icondata::AiEyeOutlined, class: "w-4 h-4" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

#[component]
fn SqlEditorPanel(
    global_mode: Signal<bool>,
    sql: Signal<String>,
    selected_table: Signal<Option<String>>,
    on_clear_selection: EventHandler<()>,
) -> Element {
    let mut run_query = use_action(|query: String| async move {
        if query.trim().is_empty() {
            return Err(AppError::Api("SQL query cannot be empty".to_string()));
        }
        ApiClient::new().execute_query(&query).await
    });
    let global = global_mode();
    let placeholder = if global {
        "SELECT * FROM global.schema.table LIMIT 10"
    } else {
        "SELECT * FROM schema.table LIMIT 10"
    };

    rsx! {
        div { class: "space-y-4",
            if global {
                div {
                    class: "text-xs px-3 py-2 rounded-md border border-violet-200 bg-violet-50 text-violet-900",
                    "Cluster mode: SQL runs against "
                    span { class: "font-mono font-medium", "global.*" }
                    " tables (fan-out across nodes). Use "
                    span { class: "font-mono", "_addr"
                    }
                    " / "
                    span { class: "font-mono", "_rank" }
                    " columns to see node identity when present."
                }
            }
            if let Some(ref fqtn) = *selected_table.read() {
                div {
                    class: format!(
                        "flex flex-wrap items-center gap-2 text-xs px-3 py-2 rounded-md border bg-{} border-{} text-{}",
                        colors::CONTENT_ACCENT_BG,
                        colors::CONTENT_ACCENT_BORDER,
                        colors::CONTENT_ACCENT_TEXT,
                    ),
                    span { class: "font-medium", "Selected:" }
                    span { class: "font-mono", "{fqtn}" }
                    button {
                        class: "ml-auto text-gray-500 hover:text-gray-800 underline-offset-2 hover:underline",
                        onclick: move |_| on_clear_selection.call(()),
                        "Clear"
                    }
                }
            }

            div { class: "flex flex-wrap items-center gap-2",
                button {
                    class: format!(
                        "inline-flex items-center gap-1.5 px-4 py-2 text-sm font-medium text-white rounded-md bg-{} hover:bg-{} shadow-sm transition-colors {}",
                        colors::PRIMARY,
                        colors::PRIMARY_HOVER,
                        if run_query.pending() { "opacity-60 cursor-not-allowed" } else { "" }
                    ),
                    disabled: run_query.pending(),
                    onclick: move |_| {
                        if !run_query.pending() {
                            run_query.call(sql());
                        }
                    },
                    Icon { icon: &icondata::AiPlayCircleOutlined, class: "w-4 h-4" }
                    if run_query.pending() { "Running…" } else { "Run" }
                }
                button {
                    class: format!(
                        "px-3 py-2 text-sm rounded-md border border-gray-300 bg-white text-gray-700 hover:bg-{} transition-colors",
                        colors::BTN_SECONDARY_HOVER,
                    ),
                    onclick: move |_| sql.set(String::new()),
                    "Clear"
                }
                button {
                    class: format!(
                        "px-3 py-2 text-sm rounded-md border border-gray-300 bg-white text-gray-700 hover:bg-{} transition-colors",
                        colors::BTN_SECONDARY_HOVER,
                    ),
                    onclick: move |_| sql.set(example_sql(global_mode()).to_string()),
                    "Example"
                }
                span { class: "ml-auto text-xs text-gray-400 hidden sm:inline",
                    "⌘ Enter / Ctrl+Enter to run"
                }
            }

            div { class: "rounded-lg border border-gray-300 overflow-hidden focus-within:ring-2 focus-within:ring-blue-500/30 focus-within:border-blue-500",
                textarea {
                    class: "w-full min-h-[140px] max-h-[320px] font-mono text-sm p-3 bg-slate-50 text-gray-900 resize-y focus:outline-none",
                    placeholder: "{placeholder}",
                    value: "{sql}",
                    oninput: move |ev| sql.set(ev.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter && (e.modifiers().meta() || e.modifiers().ctrl()) {
                            e.prevent_default();
                            if !run_query.pending() {
                                run_query.call(sql());
                            }
                        }
                    },
                }
            }

            div { class: "min-h-[4rem]",
                if run_query.pending() {
                    LoadingState { message: Some("Running query…".to_string()) }
                } else if let Some(Ok(df_signal)) = run_query.value() {
                    {
                        let df = df_signal();
                        let rows = dataframe_row_count(&df);
                        let cols = df.names.len();
                        rsx! {
                            div { class: "space-y-2",
                                div { class: "flex flex-wrap items-center gap-3 text-xs text-gray-500",
                                    span { class: "font-medium text-gray-700", "Results" }
                                    span { "{rows} rows" }
                                    span { "·" }
                                    span { "{cols} columns" }
                                }
                                div { class: "rounded-lg border border-gray-200 overflow-hidden",
                                    DataFrameView { df: df.clone(), on_row_click: None }
                                }
                            }
                        }
                    }
                } else if let Some(Err(err)) = run_query.value() {
                    AppErrorDisplay {
                        error: AppError::Api(err.to_string()),
                        title: Some("Query failed".to_string()),
                    }
                } else {
                    div {
                        class: "rounded-lg border border-dashed border-gray-200 bg-gray-50/50 px-4 py-8 text-center text-sm text-gray-500",
                        "Run a query to see results here"
                    }
                }
            }
        }
    }
}

#[component]
fn PreviewModal(
    title: String,
    preview: Action<(String,), DataFrame>,
    on_close: EventHandler<()>,
    on_use_in_editor: EventHandler<String>,
) -> Element {
    let preview_query = title
        .split('•')
        .next()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|fqtn| format!("SELECT * FROM {fqtn} LIMIT 10"))
        .unwrap_or_default();

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4",
            tabindex: "-1",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape {
                    on_close.call(());
                }
            },
            div {
                class: "absolute inset-0 bg-slate-900/40 backdrop-blur-sm",
                onclick: move |_| on_close.call(()),
            }
            div {
                class: "relative w-full sm:max-w-5xl max-h-[90vh] flex flex-col bg-white sm:rounded-xl shadow-2xl border border-gray-200 overflow-hidden",
                div { class: "flex items-start justify-between gap-3 px-4 py-3 border-b border-gray-200 bg-gray-50/80",
                    div { class: "min-w-0",
                        div { class: "flex items-center gap-2",
                            Icon { icon: &icondata::AiTableOutlined, class: "w-5 h-5 text-blue-600 shrink-0" }
                            h3 { class: "text-base font-semibold text-gray-900 truncate", "{title}" }
                        }
                        p { class: "text-xs text-gray-500 mt-0.5", "Preview · Esc to close" }
                    }
                    div { class: "flex items-center gap-2 shrink-0",
                        if !preview_query.is_empty() {
                            button {
                                class: format!(
                                    "px-3 py-1.5 text-sm rounded-md border border-{} text-{} bg-{} hover:bg-{} transition-colors",
                                    colors::CONTENT_ACCENT_BORDER,
                                    colors::CONTENT_ACCENT_TEXT,
                                    colors::CONTENT_ACCENT_BG,
                                    colors::BTN_SECONDARY_HOVER,
                                ),
                                onclick: {
                                    let q = preview_query.clone();
                                    move |_| on_use_in_editor.call(q.clone())
                                },
                                "Use in editor"
                            }
                        }
                        button {
                            class: format!(
                                "px-3 py-1.5 text-sm rounded-md bg-{} hover:bg-{} text-gray-700 transition-colors",
                                colors::BTN_SECONDARY_BG,
                                colors::BTN_SECONDARY_HOVER,
                            ),
                            onclick: move |_| on_close.call(()),
                            "Close"
                        }
                    }
                }
                div { class: "flex-1 overflow-auto p-4",
                    if preview.pending() {
                        LoadingState { message: Some("Loading preview…".to_string()) }
                    } else if let Some(Ok(df_signal)) = preview.value() {
                        {
                            let df = df_signal();
                            let rows = dataframe_row_count(&df);
                            rsx! {
                                div { class: "space-y-2",
                                    p { class: "text-xs text-gray-500", "{rows} rows" }
                                    DataFrameView { df: df.clone(), on_row_click: None }
                                }
                            }
                        }
                    } else if let Some(Err(err)) = preview.value() {
                        AppErrorDisplay {
                            error: AppError::Api(err.to_string()),
                            title: Some("Preview failed".to_string()),
                        }
                    } else {
                        span { class: "text-gray-500 text-sm", "Preparing preview…" }
                    }
                }
            }
        }
    }
}

fn dataframe_row_count(df: &DataFrame) -> usize {
    df.cols.iter().map(|c| c.len()).max().unwrap_or(0)
}
