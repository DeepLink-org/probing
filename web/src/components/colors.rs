// Unified color system definition
// Use Tailwind CSS class names to ensure color consistency across the application
//
// Design principles:
// - Sidebar: Dark slate background + blue accent color (professional, stable)
// - Main content area: Light gray/indigo background (clear, readable)
// - Accent color: blue (consistent with sidebar, maintains visual unity)

#[allow(clippy::module_inception)]
pub mod colors {
    pub const PRIMARY: &str = "blue-600";
    pub const PRIMARY_HOVER: &str = "blue-700";
    pub const BTN_SECONDARY_HOVER: &str = "gray-200";

    pub const SUCCESS: &str = "green-600";
    pub const SUCCESS_HOVER: &str = "green-700";

    pub const ERROR_LIGHT: &str = "red-50";
    pub const ERROR_TEXT: &str = "red-800";
    pub const ERROR_BORDER: &str = "red-200";

    /// Content-area accent (e.g. badges, tags on light background)
    pub const CONTENT_ACCENT_BG: &str = "blue-50";
    pub const CONTENT_ACCENT_TEXT: &str = "blue-700";
    pub const CONTENT_ACCENT_BORDER: &str = "blue-200";
}
