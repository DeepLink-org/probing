// Unified color system definition
// Use Tailwind CSS class names to ensure color consistency across the application
//
// Design principles:
// - Sidebar: Dark slate background + blue accent color (professional, stable)
// - Main content area: Light gray/indigo background (clear, readable)
// - Accent color: blue (consistent with sidebar, maintains visual unity)

#[allow(dead_code)]
pub mod colors {
    pub const PRIMARY: &str = "blue-600";
    pub const PRIMARY_HOVER: &str = "blue-700";
    pub const PRIMARY_BG: &str = "blue-600/30";
    pub const PRIMARY_TEXT: &str = "blue-100";
    pub const PRIMARY_TEXT_DARK: &str = "blue-400";
    pub const PRIMARY_BORDER: &str = "blue-500";

    /// Secondary button (inactive outline)
    pub const BTN_SECONDARY_BG: &str = "gray-100";
    pub const BTN_SECONDARY_HOVER: &str = "gray-200";

    pub const SIDEBAR_BG: &str = "slate-900";
    pub const SIDEBAR_BG_VIA: &str = "slate-800";
    pub const SIDEBAR_BORDER: &str = "slate-700/30";
    pub const SIDEBAR_TEXT_PRIMARY: &str = "slate-100";
    pub const SIDEBAR_TEXT_SECONDARY: &str = "slate-300";
    pub const SIDEBAR_TEXT_MUTED: &str = "slate-400";
    pub const SIDEBAR_HOVER_BG: &str = "slate-800/50";
    pub const SIDEBAR_ACTIVE_BG: &str = "slate-700";
    pub const SIDEBAR_INPUT_BG: &str = "slate-800";
    pub const SIDEBAR_INPUT_BORDER: &str = "slate-600";

    pub const CONTENT_BG: &str = "gray-50";
    pub const CONTENT_BG_ACCENT: &str = "indigo-50/30";
    pub const CONTENT_CARD_BG: &str = "white";
    pub const CONTENT_BORDER: &str = "gray-200";
    pub const CONTENT_TEXT_PRIMARY: &str = "gray-900";
    pub const CONTENT_TEXT_SECONDARY: &str = "gray-600";
    pub const CONTENT_TEXT_MUTED: &str = "gray-500";

    pub const SUCCESS: &str = "green-600";
    pub const SUCCESS_HOVER: &str = "green-700";
    pub const SUCCESS_LIGHT: &str = "green-50";
    pub const SUCCESS_TEXT: &str = "green-800";
    pub const SUCCESS_BORDER: &str = "green-200";

    pub const ERROR: &str = "red-600";
    pub const ERROR_HOVER: &str = "red-700";
    pub const ERROR_LIGHT: &str = "red-50";
    pub const ERROR_TEXT: &str = "red-800";
    pub const ERROR_BORDER: &str = "red-200";

    /// Content-area accent (e.g. badges, tags on light background)
    pub const CONTENT_ACCENT_BG: &str = "blue-50";
    pub const CONTENT_ACCENT_TEXT: &str = "blue-700";
    pub const CONTENT_ACCENT_BORDER: &str = "blue-200";

    pub const WARNING: &str = "yellow-600";
    pub const WARNING_LIGHT: &str = "yellow-50";
    pub const WARNING_TEXT: &str = "yellow-800";

    // Composite Tailwind class strings (must be literals for the CSS build to include them).
    pub const SIDEBAR_ASIDE: &str = "bg-gradient-to-b from-slate-900 via-slate-800 to-slate-900 border-r border-slate-700/30 h-screen flex flex-col flex-shrink-0 shadow-xl";
    pub const SIDEBAR_LOGO_BORDER: &str = "px-4 py-3 border-b border-slate-700/30";
    pub const SIDEBAR_BRAND: &str = "text-base font-semibold text-slate-100";
    pub const SIDEBAR_FOOTER: &str = "px-4 py-3 border-t border-slate-700/30";
    pub const SIDEBAR_FOOTER_LINK: &str =
        "flex items-center gap-2 text-xs text-slate-400 hover:text-blue-400 transition-colors";
    pub const SIDEBAR_HIDE_BTN: &str = "absolute top-4 -right-3 w-6 h-6 bg-slate-700 border border-slate-700 rounded-full shadow-lg flex items-center justify-center hover:bg-slate-600 z-30 transition-colors";

    pub const SIDEBAR_ITEM_ACTIVE: &str = "flex items-center gap-2 px-2 py-1.5 text-sm font-medium rounded-md bg-blue-600/30 text-blue-100 border-l-2 border-blue-500";
    pub const SIDEBAR_ITEM_INACTIVE: &str = "flex items-center gap-2 px-2 py-1.5 text-sm font-medium rounded-md text-slate-300 hover:bg-slate-800/50 hover:text-blue-100 transition-colors";

    pub const SIDEBAR_PANEL_BORDER: &str = "mt-4 pt-4 border-t border-slate-700/30";

    pub const SIDEBAR_CONTROL_TITLE: &str = "text-xs font-semibold text-slate-300";
    pub const SIDEBAR_CONTROL_VALUE: &str = "text-xs text-slate-400";
    pub const SIDEBAR_TOGGLE_ON: &str =
        "relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors bg-blue-600";
    pub const SIDEBAR_TOGGLE_OFF: &str =
        "relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors bg-slate-700";
    pub const SIDEBAR_TOGGLE_LABEL: &str = "text-xs text-slate-300";
    pub const SIDEBAR_INPUT: &str = "w-full px-2 py-1 border border-slate-600 bg-slate-800 text-slate-300 rounded text-xs focus:border-blue-500 focus:outline-none";
    pub const SIDEBAR_RESIZE_HOVER: &str = "hover:bg-blue-600/50";
    pub const SIDEBAR_RESIZE_ACTIVE: &str = "bg-blue-600";
}
