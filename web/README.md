# Probing Web Interface

这是使用 Dioxus 重写的 Probing 项目 Web 界面，替代了原来的 Leptos 版本。

## 功能特性

- 🚀 基于 Dioxus 的现代化 Web 界面
- 📊 进程信息监控
- 🧵 线程活动追踪
- 📈 性能分析工具
- 🔍 Python 对象检查
- 📉 时间序列数据可视化
- 🎨 响应式设计，支持深色模式

## 项目结构

```
web/
├── src/
│   ├── main.rs              # 应用入口
│   ├── app.rs               # 路由和应用配置
│   ├── components/          # 可复用组件
│   │   ├── header_bar.rs    # 顶部导航栏
│   │   ├── page_layout.rs   # 页面布局
│   │   ├── panel.rs         # 面板组件
│   │   ├── card_view.rs     # 卡片视图
│   │   ├── table_view.rs    # 表格组件
│   │   └── dataframe_view.rs # 数据框视图
│   ├── pages/               # 页面组件
│   │   ├── overview.rs      # 概览页面
│   │   ├── cluster.rs       # 集群页面
│   │   ├── activity.rs      # 活动页面
│   │   ├── profiler.rs      # 性能分析页面
│   │   ├── timeseries.rs    # 时间序列页面
│   │   └── python.rs        # Python 检查页面
│   └── utils/                # 工具模块
│       ├── api.rs           # API 客户端
│       └── error.rs         # 错误处理
├── assets/                  # 静态资源
│   └── tailwind.css         # 样式文件
├── Cargo.toml               # Rust 依赖配置
├── Dioxus.toml              # Dioxus 配置
├── index.html               # HTML 模板
└── build.sh                 # 构建脚本
```

## 开发环境设置

### 前置要求

- Rust 1.70+
- Node.js (用于开发服务器)
- Dioxus CLI

### 安装 Dioxus CLI

```bash
cargo install dioxus-cli
```

### 开发模式

```bash
# 启动开发服务器
dx serve
```

### 构建生产版本

```bash
# 使用构建脚本
./build.sh

# 或手动构建
dx build --release
```

## 主要改进

相比原来的 Leptos 版本，Dioxus 版本具有以下优势：

1. **更好的性能**: Dioxus 的虚拟 DOM 实现更高效
2. **更简洁的语法**: 使用 `rsx!` 宏，语法更接近 React
3. **更好的类型安全**: 编译时检查更多错误
4. **更活跃的社区**: Dioxus 社区更活跃，更新更频繁
5. **更好的开发体验**: 热重载和调试工具更完善

## 组件说明

### 页面布局 (PageLayout)
提供统一的页面布局，包括顶部导航栏和内容区域。

### 面板组件 (Panel)
用于显示信息的容器组件，支持标题和内容。

### 表格组件 (TableView)
用于显示结构化数据的表格组件。

### 卡片视图 (CardView)
用于显示进程和线程信息的卡片组件。

## API 集成

Web 界面通过 HTTP API 与后端服务通信：

- `/apis/overview` - 获取进程概览信息
- `/apis/cluster` - 获取集群信息
- `/apis/activity` - 获取活动信息
- `/apis/activity/{tid}` - 获取特定线程的活动信息

## 部署

构建完成后，`dist/` 目录包含所有静态文件，可以部署到任何 Web 服务器。

推荐使用 Nginx 或 Apache 作为 Web 服务器。
