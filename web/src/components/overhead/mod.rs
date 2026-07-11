//! TorchProbe overhead UI.

mod panel;

pub use panel::TorchOverheadPanel;
pub use crate::api::OVERHEAD_POLL_MS;
pub use crate::overhead::{overhead_trigger_label, table_missing_trigger_label};
