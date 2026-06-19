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

pub mod workspace;
pub mod stat_card;
pub mod card;
pub mod agent;
pub mod global_command_panel;
pub mod card_view;
pub mod callstack_view;
pub mod collapsible_card;
pub mod cpu_threads_table;
pub mod colors;
pub mod common;
pub mod dataframe_view;
pub mod data;
pub mod flamegraph;
pub mod investigation_context_bar;
pub mod investigation_context_hint;
pub mod icon;
pub mod keyboard_shortcuts;
pub mod layout;
pub mod markdown_view;
pub mod page;
pub mod poll_status;
pub mod profiling_sidebar_hint;
pub mod profile_snapshot_bar;
pub mod profiling;
pub mod sidebar;
pub mod source_viewer;
pub mod table_view;
pub mod timeline_viewer;
pub mod value_list;
pub mod page_context_sync;
pub mod ui_task_runtime;
