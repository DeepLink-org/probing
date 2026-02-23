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
//! - **chrome_tracing_iframe** — Chrome Tracing viewer wrapper.

pub mod card;
pub mod card_view;
pub mod callstack_view;
pub mod chrome_tracing_iframe;
pub mod collapsible_card;
pub mod colors;
pub mod common;
pub mod dataframe_view;
pub mod data;
pub mod icon;
pub mod layout;
pub mod page;
pub mod sidebar;
pub mod table_view;
pub mod value_list;
