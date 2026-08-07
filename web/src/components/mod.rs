//! Reusable UI building blocks. See `DESIGN.md` for layout and color conventions.
//!
//! - **agent** — Investigation skill runner helpers and LLM settings overlay.
//! - **card** — Card with optional header_right.
//! - **common** — LoadingState, ErrorState, EmptyState.
//! - **colors** — Tailwind color constants.
//! - **table_view** / **dataframe_view** — Tables.
//! - **flamegraph** — Native flamegraph visualizations.
//! - **timeline_viewer** — Native Chrome trace timeline + Perfetto export.
//! - **profiling_controls** — Capture controls shared with the Next sidebar.

pub mod agent;
pub mod app_overlays;
pub mod callstack_view;
pub mod card;
pub mod colors;
pub mod common;
pub mod data;
pub mod dataframe_view;
pub mod flamegraph;
pub mod global_command_panel;
pub mod icon;
pub mod keyboard_shortcuts;
pub mod markdown_view;
pub mod overhead;
pub mod overlay_shell;
pub mod poll_status;
pub mod profile_snapshot_bar;
pub mod profiling;
pub mod profiling_controls;
pub mod rl;
pub mod source_viewer;
pub mod span_timeline;
pub mod stat_card;
pub mod table_view;
pub mod timeline_viewer;
pub mod ui_task_runtime;
pub mod value_list;
pub mod workspace;
