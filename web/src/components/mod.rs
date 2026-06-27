//! Reusable UI building blocks. See `DESIGN.md` for layout and color conventions.
//!
//! - **layout** — App shell (sidebar + main area).
//! - **sidebar** — Navigation, logo, Profiling submenu.
//! - **page** — PageTitle, PageContainer for content pages.
//! - **card** — Card with optional header_right.
//! - **common** — LoadingState, ErrorState, EmptyState.
//! - **colors** — Tailwind color constants.
//! - **table_view** / **dataframe_view** — Tables.
//! - **data** — KeyValueList and similar.
//! - **icon** — Icon component.
//! - **collapsible_card** / **card_view** / **callstack_view** / **value_list** — Domain helpers.
//! - **timeline_viewer** — Native Chrome trace timeline + Perfetto export.
//! - **flamegraph** — Native flamegraph visualizations.

pub mod agent;
pub mod callstack_view;
pub mod card;
pub mod card_view;
pub mod collapsible_card;
pub mod colors;
pub mod common;
pub mod cpu_threads_table;
pub mod data;
pub mod dataframe_view;
pub mod flamegraph;
pub mod global_command_panel;
pub mod icon;
pub mod investigation_context_hint;
pub mod keyboard_shortcuts;
pub mod layout;
pub mod markdown_view;
pub mod page;
pub mod page_context_sync;
pub mod poll_status;
pub mod profile_snapshot_bar;
pub mod profiling;
pub mod profiling_sidebar_hint;
pub mod sidebar;
pub mod source_viewer;
pub mod span_timeline;
pub mod stat_card;
pub mod table_view;
pub mod timeline_viewer;
pub mod ui_task_runtime;
pub mod value_list;
pub mod workspace;
