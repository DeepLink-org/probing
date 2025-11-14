// 统一颜色系统定义
// 使用 Tailwind CSS 类名，确保整个应用的颜色一致性
// 
// 设计原则：
// - 侧边栏：深色 slate 背景 + blue 强调色（专业、沉稳）
// - 主内容区：浅色 gray/indigo 背景（清晰、易读）
// - 强调色：blue（与侧边栏一致，保持视觉统一）

pub mod colors {
    pub const PRIMARY: &str = "blue-600";
    pub const PRIMARY_BG: &str = "blue-600/30";
    pub const PRIMARY_TEXT: &str = "blue-100";
    pub const PRIMARY_TEXT_DARK: &str = "blue-400";
    pub const PRIMARY_BORDER: &str = "blue-500";
    
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
    pub const SUCCESS_LIGHT: &str = "green-50";
    pub const SUCCESS_TEXT: &str = "green-800";
    pub const SUCCESS_BORDER: &str = "green-200";
    
    pub const ERROR: &str = "red-600";
    pub const ERROR_LIGHT: &str = "red-50";
    pub const ERROR_TEXT: &str = "red-800";
    pub const ERROR_BORDER: &str = "red-200";
    
    pub const WARNING: &str = "yellow-600";
    pub const WARNING_LIGHT: &str = "yellow-50";
    pub const WARNING_TEXT: &str = "yellow-800";
    
}
