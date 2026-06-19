//! Shared workspace UI: cards, panel shell, split layout.

mod panel_shell;
mod split;
mod surface;

pub use panel_shell::WorkspacePanelShell;
pub use surface::{AccentSurface, ChipButton, StatusBadge, SurfaceCard, SurfaceCardBody, SurfaceIconHeader, WidthSegment};
