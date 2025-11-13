// 统一颜色系统定义
// 使用 Tailwind CSS 类名，确保整个应用的颜色一致性
// 
// 设计原则：
// - 侧边栏：深色 slate 背景 + blue 强调色（专业、沉稳）
// - 主内容区：浅色 gray/indigo 背景（清晰、易读）
// - 强调色：blue（与侧边栏一致，保持视觉统一）

pub mod colors {
    // ============================================================================
    // 主题色 - Blue（蓝色）
    // 作为主要强调色，用于按钮、链接、激活状态等
    // ============================================================================
    pub const PRIMARY: &str = "blue-600";
    pub const PRIMARY_LIGHT: &str = "blue-50";
    pub const PRIMARY_DARK: &str = "blue-700";
    pub const PRIMARY_HOVER: &str = "blue-700";
    pub const PRIMARY_ACCENT: &str = "blue-500";
    pub const PRIMARY_BG: &str = "blue-600/30";  // 半透明背景
    pub const PRIMARY_TEXT: &str = "blue-100";   // 深色背景上的文字
    pub const PRIMARY_TEXT_DARK: &str = "blue-400"; // 深色背景上的次要文字
    pub const PRIMARY_BORDER: &str = "blue-500"; // 边框色
    
    // ============================================================================
    // 侧边栏深色主题 - Slate（石板灰）
    // 用于侧边栏背景和深色区域
    // ============================================================================
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
    
    // ============================================================================
    // 主内容区浅色主题 - Gray（灰色）
    // 用于主内容区背景和浅色区域
    // ============================================================================
    pub const CONTENT_BG: &str = "gray-50";
    pub const CONTENT_BG_ACCENT: &str = "indigo-50/30"; // 渐变背景
    pub const CONTENT_CARD_BG: &str = "white";
    pub const CONTENT_BORDER: &str = "gray-200";
    pub const CONTENT_TEXT_PRIMARY: &str = "gray-900";
    pub const CONTENT_TEXT_SECONDARY: &str = "gray-600";
    pub const CONTENT_TEXT_MUTED: &str = "gray-500";
    pub const CONTENT_HOVER_BG: &str = "gray-50";
    
    // ============================================================================
    // 功能色
    // ============================================================================
    
    // 成功 - Green
    pub const SUCCESS: &str = "green-600";
    pub const SUCCESS_LIGHT: &str = "green-50";
    pub const SUCCESS_TEXT: &str = "green-800";
    pub const SUCCESS_BORDER: &str = "green-200";
    pub const SUCCESS_HOVER: &str = "green-700";
    
    // 错误 - Red
    pub const ERROR: &str = "red-600";
    pub const ERROR_LIGHT: &str = "red-50";
    pub const ERROR_TEXT: &str = "red-800";
    pub const ERROR_BORDER: &str = "red-200";
    
    // 警告 - Yellow/Orange
    pub const WARNING: &str = "yellow-600";
    pub const WARNING_LIGHT: &str = "yellow-50";
    pub const WARNING_TEXT: &str = "yellow-800";
    
    // ============================================================================
    // 中性色 - Gray（通用）
    // ============================================================================
    pub const GRAY_50: &str = "gray-50";
    pub const GRAY_100: &str = "gray-100";
    pub const GRAY_200: &str = "gray-200";
    pub const GRAY_300: &str = "gray-300";
    pub const GRAY_400: &str = "gray-400";
    pub const GRAY_600: &str = "gray-600";
    pub const GRAY_700: &str = "gray-700";
    pub const GRAY_800: &str = "gray-800";
    pub const GRAY_900: &str = "gray-900";
    
    // ============================================================================
    // 便捷函数：生成完整的 Tailwind 类名
    // ============================================================================
    
    /// 生成背景色类名
    pub fn bg(color: &str) -> String {
        format!("bg-{}", color)
    }
    
    /// 生成文字色类名
    pub fn text(color: &str) -> String {
        format!("text-{}", color)
    }
    
    /// 生成边框色类名
    pub fn border(color: &str) -> String {
        format!("border-{}", color)
    }
    
    /// 生成 hover 背景色类名
    pub fn hover_bg(color: &str) -> String {
        format!("hover:bg-{}", color)
    }
    
    /// 生成 hover 文字色类名
    pub fn hover_text(color: &str) -> String {
        format!("hover:text-{}", color)
    }
}
