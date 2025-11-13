// 统一颜色系统定义
// 使用 Tailwind CSS 类名，确保整个应用的颜色一致性
// 主题色：Indigo（靛蓝色）- 专业、科技感强，适合性能分析工具

pub mod colors {
    // 主色调 - Indigo（靛蓝色）
    // 选择 Indigo 作为主题色，因为它：
    // 1. 比蓝色更有特色和识别度
    // 2. 保持专业感和科技感
    // 3. 适合性能分析、调试工具的品牌形象
    pub const PRIMARY: &str = "indigo-600";
    pub const PRIMARY_LIGHT: &str = "indigo-50";
    pub const PRIMARY_TEXT: &str = "indigo-700";
    pub const PRIMARY_HOVER: &str = "indigo-700";
    pub const PRIMARY_DARK: &str = "indigo-800";
    pub const PRIMARY_ACCENT: &str = "indigo-500";
    
    // 成功 - Green
    pub const SUCCESS: &str = "green-600";
    pub const SUCCESS_LIGHT: &str = "green-50";
    pub const SUCCESS_TEXT: &str = "green-800";
    pub const SUCCESS_BORDER: &str = "green-200";
    
    // 错误 - Red
    pub const ERROR: &str = "red-600";
    pub const ERROR_LIGHT: &str = "red-50";
    pub const ERROR_TEXT: &str = "red-800";
    pub const ERROR_BORDER: &str = "red-200";
    
    // 警告 - Yellow/Orange
    pub const WARNING: &str = "yellow-600";
    pub const WARNING_LIGHT: &str = "yellow-50";
    pub const WARNING_TEXT: &str = "yellow-800";
    
    // 中性色 - Gray
    pub const GRAY_50: &str = "gray-50";
    pub const GRAY_100: &str = "gray-100";
    pub const GRAY_200: &str = "gray-200";
    pub const GRAY_300: &str = "gray-300";
    pub const GRAY_600: &str = "gray-600";
    pub const GRAY_700: &str = "gray-700";
    pub const GRAY_900: &str = "gray-900";
    
    // 背景色
    pub const BG_WHITE: &str = "white";
    pub const BG_GRAY: &str = "gray-50";
    
    // 文本色
    pub const TEXT_PRIMARY: &str = "gray-900";
    pub const TEXT_SECONDARY: &str = "gray-600";
    pub const TEXT_MUTED: &str = "gray-500";
}

