# 从 Leptos 到 Dioxus 的迁移指南

## 概述

本项目成功将原来的 Leptos Web 界面迁移到了 Dioxus 框架。以下是主要的变更和迁移要点：

## 主要变更

### 1. 框架变更
- **从**: Leptos 0.7.8
- **到**: Dioxus 0.5

### 2. 语法变更

#### Leptos 语法
```rust
view! {
    <div>
        <h1>{title}</h1>
        <p>{content}</p>
    </div>
}
```

#### Dioxus 语法
```rust
rsx! {
    div {
        h1 { "{title}" }
        p { "{content}" }
    }
}
```

### 3. 组件结构变更

#### Leptos 组件
```rust
#[component]
pub fn MyComponent(prop: String) -> impl IntoView {
    view! { <div>{prop}</div> }
}
```

#### Dioxus 组件
```rust
#[component]
pub fn MyComponent(prop: String) -> Element {
    rsx! { div { "{prop}" } }
}
```

### 4. 状态管理变更

#### Leptos 信号
```rust
let count = RwSignal::new(0);
let doubled = Memo::new(move |_| count.get() * 2);
```

#### Dioxus 信号
```rust
let mut count = use_signal(|| 0);
let doubled = use_memo(move || count() * 2);
```

### 5. 路由变更

#### Leptos 路由
```rust
<Router>
    <Routes>
        <Route path="/" view=Home />
        <Route path="/about" view=About />
    </Routes>
</Router>
```

#### Dioxus 路由
```rust
#[derive(Routable, Clone, PartialEq)]
pub enum Route {
    #[route("/")]
    Home {},
    #[route("/about")]
    About {},
}

Router::<Route> {}
```

## 迁移的组件

### 页面组件
- ✅ `overview.rs` - 概览页面
- ✅ `cluster.rs` - 集群页面  
- ✅ `activity.rs` - 活动页面
- ✅ `profiler.rs` - 性能分析页面
- ✅ `timeseries.rs` - 时间序列页面
- ✅ `python.rs` - Python 检查页面

### UI 组件
- ✅ `header_bar.rs` - 顶部导航栏
- ✅ `page_layout.rs` - 页面布局
- ✅ `panel.rs` - 面板组件
- ✅ `card_view.rs` - 卡片视图
- ✅ `table_view.rs` - 表格组件
- ✅ `dataframe_view.rs` - 数据框视图

### 工具模块
- ✅ `api.rs` - API 客户端
- ✅ `error.rs` - 错误处理

## 配置变更

### Cargo.toml
- 移除了 Leptos 相关依赖
- 添加了 Dioxus 相关依赖
- 保持了与 `probing-proto` 的集成

### 构建配置
- 添加了 `Dioxus.toml` 配置文件
- 创建了 `build.sh` 构建脚本
- 添加了 Tailwind CSS 支持

## 优势

1. **更好的性能**: Dioxus 的虚拟 DOM 实现更高效
2. **更简洁的语法**: `rsx!` 宏语法更接近 React
3. **更好的类型安全**: 编译时检查更多错误
4. **更活跃的社区**: Dioxus 社区更活跃，更新更频繁
5. **更好的开发体验**: 热重载和调试工具更完善

## 开发命令

```bash
# 安装 Dioxus CLI
cargo install dioxus-cli

# 开发模式
dx serve

# 构建生产版本
dx build --release
```

## 注意事项

1. Dioxus 的信号系统与 Leptos 略有不同
2. 事件处理语法有所变更
3. 样式系统需要适配新的框架
4. 某些 Leptos 特有的功能可能需要重新实现

## 下一步

1. 测试所有页面的功能
2. 优化性能和用户体验
3. 添加更多交互功能
4. 完善错误处理
5. 添加单元测试
