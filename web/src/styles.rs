use dioxus::prelude::*;

/// 样式常量定义
pub mod styles {
    // 布局样式
    pub const FLEX_CENTER: &str = "flex items-center justify-center";
    pub const FLEX_COL: &str = "flex flex-col";
    pub const FLEX_ROW: &str = "flex flex-row";
    pub const FLEX_1: &str = "flex-1";
    pub const FULL_SIZE: &str = "w-full h-full";
    pub const ABSOLUTE_FULL: &str = "absolute inset-0";
    
    // 间距样式
    pub const SPACE_Y_2: &str = "space-y-2";
    pub const SPACE_Y_3: &str = "space-y-3";
    pub const SPACE_Y_4: &str = "space-y-4";
    pub const SPACE_Y_6: &str = "space-y-6";
    pub const P_4: &str = "p-4";
    pub const P_6: &str = "p-6";
    pub const PX_6: &str = "px-6";
    pub const PY_2: &str = "py-2";
    pub const PY_4: &str = "py-4";
    pub const PY_8: &str = "py-8";
    
    // 文本样式
    pub const TEXT_SM: &str = "text-sm";
    pub const TEXT_LG: &str = "text-lg";
    pub const TEXT_XL: &str = "text-xl";
    pub const TEXT_2XL: &str = "text-2xl";
    pub const TEXT_3XL: &str = "text-3xl";
    pub const FONT_SEMIBOLD: &str = "font-semibold";
    pub const FONT_BOLD: &str = "font-bold";
    pub const FONT_MONO: &str = "font-mono";
    pub const FONT_MEDIUM: &str = "font-medium";
    
    // 颜色样式
    pub const TEXT_GRAY_600: &str = "text-gray-600";
    pub const TEXT_GRAY_500: &str = "text-gray-500";
    pub const TEXT_GRAY_700: &str = "text-gray-700";
    pub const TEXT_GRAY_900: &str = "text-gray-900";
    pub const TEXT_WHITE: &str = "text-white";
    pub const TEXT_BLUE_600: &str = "text-blue-600";
    pub const TEXT_RED_500: &str = "text-red-500";
    pub const TEXT_GREEN_600: &str = "text-green-600";
    
    // 背景样式
    pub const BG_WHITE: &str = "bg-white";
    pub const BG_GRAY_50: &str = "bg-gray-50";
    pub const BG_GRAY_100: &str = "bg-gray-100";
    pub const BG_GRAY_800: &str = "bg-gray-800";
    pub const BG_GRAY_900: &str = "bg-gray-900";
    pub const BG_BLUE_600: &str = "bg-blue-600";
    pub const BG_RED_50: &str = "bg-red-50";
    
    // 边框样式
    pub const BORDER: &str = "border";
    pub const BORDER_B: &str = "border-b";
    pub const BORDER_GRAY_200: &str = "border-gray-200";
    pub const BORDER_GRAY_300: &str = "border-gray-300";
    pub const BORDER_GRAY_700: &str = "border-gray-700";
    pub const BORDER_RED_200: &str = "border-red-200";
    pub const ROUNDED: &str = "rounded";
    pub const ROUNDED_MD: &str = "rounded-md";
    pub const ROUNDED_LG: &str = "rounded-lg";
    
    // 阴影样式
    pub const SHADOW_SM: &str = "shadow-sm";
    
    // 响应式样式
    pub const GRID_COLS_1: &str = "grid grid-cols-1";
    pub const GRID_COLS_2: &str = "grid grid-cols-1 lg:grid-cols-2";
    pub const GRID_COLS_3: &str = "grid grid-cols-1 lg:grid-cols-3";
    pub const GAP_6: &str = "gap-6";
    
    // 交互样式
    pub const HOVER_BLUE_700: &str = "hover:bg-blue-700";
    pub const TRANSITION_COLORS: &str = "transition-colors";
    pub const CURSOR_POINTER: &str = "cursor-pointer";
    pub const CURSOR_NOT_ALLOWED: &str = "cursor-not-allowed";
    
    // 特殊样式
    pub const BREAK_ALL: &str = "break-all";
    pub const TEXT_CENTER: &str = "text-center";
    pub const MIN_H_120: &str = "min-h-[120px]";
    pub const MAX_W_MD: &str = "max-w-md";
    pub const OPACITY_50: &str = "opacity-50";
    pub const LAST_BORDER_B_0: &str = "last:border-b-0";
}

/// 样式组合器
pub struct StyleBuilder {
    classes: Vec<&'static str>,
}

impl StyleBuilder {
    pub fn new() -> Self {
        Self { classes: Vec::new() }
    }
    
    pub fn add(mut self, class: &'static str) -> Self {
        self.classes.push(class);
        self
    }
    
    pub fn build(self) -> String {
        self.classes.join(" ")
    }
}

/// 常用样式组合
pub mod combinations {
    use super::styles::*;
    
    // 卡片样式
    pub const CARD: &str = "bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700";
    pub const CARD_HEADER: &str = "px-6 py-4 border-b border-gray-200 dark:border-gray-700";
    pub const CARD_TITLE: &str = "text-lg font-semibold text-gray-900 dark:text-white";
    pub const CARD_CONTENT: &str = "p-6";
    
    // 按钮样式
    pub const BUTTON_PRIMARY: &str = "px-6 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 transition-colors font-medium";
    pub const BUTTON_DISABLED: &str = "opacity-50 cursor-not-allowed";
    
    // 输入框样式
    pub const INPUT: &str = "w-full font-mono text-sm p-3 rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800";
    pub const TEXTAREA: &str = "w-full min-h-[120px] font-mono text-sm p-3 rounded border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800";
    
    // 布局样式
    pub const PAGE_CONTAINER: &str = "space-y-6";
    pub const PAGE_HEADER: &str = "mb-8";
    pub const PAGE_TITLE: &str = "text-3xl font-bold text-gray-900 dark:text-white";
    pub const PAGE_SUBTITLE: &str = "mt-2 text-gray-600 dark:text-gray-400";
    pub const SECTION_CONTAINER: &str = "space-y-6";
    pub const SECTION_SUBTITLE: &str = "text-sm text-gray-600";
    
    // 列表样式
    pub const LIST_ITEM: &str = "flex justify-between items-center py-2 border-b border-gray-200 last:border-b-0";
    pub const LIST_ITEM_LABEL: &str = "font-medium text-gray-700";
    pub const LIST_ITEM_VALUE: &str = "font-mono text-sm bg-gray-100 px-2 py-1 rounded break-all";
    
    // 状态样式
    pub const LOADING: &str = "text-center py-8 text-gray-500";
    pub const ERROR: &str = "text-red-500 p-4 bg-red-50 border border-red-200 rounded";
    pub const EMPTY: &str = "text-center py-8 text-gray-500";
    
    // 表格样式
    pub const TABLE_CONTAINER: &str = "w-full overflow-x-auto border border-gray-200 dark:border-gray-700 rounded-lg";
    pub const TABLE: &str = "w-full border-collapse table-auto";
    pub const TABLE_HEADER_ROW: &str = "bg-gray-50 dark:bg-gray-800 border-b border-gray-200 dark:border-gray-700";
    pub const TABLE_HEADER_CELL: &str = "px-4 py-2 text-left font-semibold text-gray-700 dark:text-gray-200 border-r border-gray-200 dark:border-gray-700";
    pub const TABLE_ROW_EVEN: &str = "bg-white";
    pub const TABLE_ROW_ODD: &str = "bg-gray-50";
    pub const TABLE_CELL: &str = "px-4 py-2 text-gray-700 dark:text-gray-200 border-r border-gray-200 dark:border-gray-700";
}

/// 条件样式组合器
pub fn conditional_class(condition: bool, true_class: &'static str, false_class: &'static str) -> &'static str {
    if condition { true_class } else { false_class }
}

/// 响应式样式组合器
pub fn responsive_class(base: &'static str, sm: Option<&'static str>, lg: Option<&'static str>) -> String {
    let mut classes: Vec<String> = vec![base.to_string()];
    if let Some(sm_class) = sm {
        classes.push(format!("sm:{}", sm_class));
    }
    if let Some(lg_class) = lg {
        classes.push(format!("lg:{}", lg_class));
    }
    classes.join(" ")
}
