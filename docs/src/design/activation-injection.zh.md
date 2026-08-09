# 启用、注入与运行时控制

Probing 有两条进入目标进程的路径：在 CPython 启动时通过 `.pth` 自动加载，或者在 Linux
进程运行期间使用 ptrace 注入。两条路径最终加载同一个 `libprobing`，随后由同一套服务端、
查询引擎和扩展注册流程接管。

这两种方式改变的是**何时加载动态库**，不会改变 Probing 的数据表、查询接口或采集器边界。

## 两条启用路径

![启动加载与运行时注入最终加载同一动态库](../assets/architecture/probing-activation-paths.svg)

| 路径 | 适用场景 | 触发方式 | 平台 |
|------|----------|----------|------|
| 启动加载 | 能控制训练进程的启动环境 | `PROBING=1 python train.py` | Linux、macOS、Windows |
| 运行时注入 | 任务已经运行且不能重启 | `probing -t <pid> inject` | Linux |

两条路径都在目标进程原有的用户和权限上下文中运行。`inject` 不提供权限绕过；是否允许
ptrace 由同 UID、父子关系、Yama `ptrace_scope` 和 `CAP_SYS_PTRACE` 等内核策略决定。

## 启动路径：`.pth` 如何获得执行机会

![CPython 处理 pth 文件后加载 Probing](../assets/architecture/probing-pth-bootstrap.svg)

Python 启动时由 `site` 模块处理安装目录中的 `.pth` 文件。Probing wheel 安装的
`probing.pth` 导入 `probing_hook.py`，后者再进入 `site_hook.py`：

1. 解析 `PROBING`，保存原值并检查当前进程是否需要启用；
2. 跳过不应启动服务的进程，例如用户配置排除的子进程；
3. 导入 `probing`，从而加载包含 Rust 运行时的 `_core` Python 模块；
4. Rust 动态库构造函数准备服务端和扩展；
5. Python 侧安装 torch、异常处理和可选框架集成。

`.pth` 只负责在解释器正常启动流程中获得一次导入机会。是否启动采集和网络监听仍由
`PROBING` 配置、进程过滤和后续初始化状态决定。

## 注入路径：可回滚的远程函数调用

![ptrace 注入使用可回滚跳板执行 dlopen](../assets/architecture/probing-ptrace-injection.svg)

注入器不会把完整的动态加载逻辑写成 shellcode。它只写入一个与目标架构匹配的极小调用
跳板，用 tracer 控制寄存器和 ABI 参数，依次调用目标进程中的 `setenv`、`malloc`、
`dlopen` 和 `free`。

关键步骤如下：

1. **停止并取得控制。** attach 主线程并枚举 `/proc/<pid>/task`，确保注入期间线程不会在
   被覆盖的指令区域继续执行。
2. **保存现场。** 保存目标地址的原始代码字节和被选线程的寄存器。
3. **换算函数地址。** 根据 injector 与 target 的共享库基址换算目标 libc/libdl 中的符号地址，
   不能把本进程虚拟地址直接写给目标进程。
4. **写入调用跳板。** x86_64 使用间接 `call` 和 `INT3`，AArch64 使用 `BLR` 和 `BRK`；
   参数、栈对齐和返回值遵守各自 ABI。
5. **校验结果。** 只接受预期 trap，并检查 `dlopen` 返回值。目标进程退出、收到其他 signal
   或动态库加载失败都会显式报错。
6. **恢复与 detach。** 恢复原始字节和寄存器，再逐个 detach 已附着线程。错误路径也必须
   尽力恢复现场。

恢复现场不等于卸载动态库：`dlopen` 成功后，库及其构造函数产生的运行时状态会保留。
因此注入是一次控制操作，不应作为频繁采样机制。

## 动态库加载后如何接受控制

![动态库构造函数、监听入口与 Engine 就绪过程](../assets/architecture/probing-runtime-control.svg)

动态库加载成功和查询引擎可用是两个不同状态：

1. 构造函数尽快创建本地 Unix socket 或配置的 TCP 监听入口；
2. Engine 在后台完成 catalog、扩展和数据源注册；
3. readiness 从 `claimed` / `in-progress` 进入 `ready`；
4. CLI、Web 和 MCP 通过公开 HTTP 接口查询或控制；
5. 训练 hook 只写本地表，不等待远端控制请求，也不参与 fan-out。

监听入口提前存在，可以让调用方区分“进程尚未注入”“动态库已经加载但 Engine 尚未就绪”
和“服务可以查询”三种状态，而不是把所有失败都表现为连接拒绝。

## 控制入口与 CLI 结构

动态库进入 `ready` 后，CLI 只是公开协议的客户端，不拥有另一套控制逻辑：

![CLI 通过公开传输访问目标进程中的 Server、Engine 与 Extensions](../assets/architecture/probing-cli-control-surface.svg)

命令保持单层调用：`probing [-v] [-t TARGET] <command> ...`；根帮助按 Processes、Analyze、
Diagnose、Runtime、Agent 分组，但分组不改变协议或模块边界。`skill`、`mcp` 等具有自身动作的
命令可以保留二级子命令。

当前仍保留 `cluster query` 与 `cluster nodes`。目标收敛方向是分别并入 `query --global` 和
顶层 `nodes`；在实现完成前，文档不能把目标命令写成现有接口。命令注册与单命令说明位于
`probing/cli/src/cli/commands.rs`，根帮助分组位于 `help.rs`。新增命令必须先判断它属于进程
控制、查询、诊断还是 Agent 入口，并继续通过公开 HTTP/proto 契约访问服务端。

## 关键不变量

- 启动加载和运行时注入最终进入同一个 composition root，不维护两套服务端实现。
- 注入器只负责取得一次 `dlopen` 执行机会；采集和查询逻辑不放进 shellcode。
- 目标进程的训练线程不能因 Engine 初始化或远端查询失败而退出。
- 训练回调不执行网络请求；采集写路径与 HTTP 控制面解耦。
- CLI、Web 和 MCP 只使用公开接口，不链接或调用采集器内部实现。
- 注入失败必须可观察，不能通过伪造“空数据”表现为成功。

## 相关实现

| 关注点 | 位置 |
|--------|------|
| `.pth` 与启动过滤 | `python/probing.pth`、`python/probing_hook.py`、`python/probing/site_hook.py` |
| inject CLI 与 ptrace | `probing/cli/` 中的 inject/ctrl 实现 |
| Rust/Python 动态库入口 | 根 crate `src/lib.rs` 与 `probing._core` |
| 服务端 composition root | `probing/server/src/engine.rs` |
| 公开接口 | `probing/server/API.md`、`probing/proto/` |

使用方式见[安装指南](../installation.zh.md)和[核心模型](../guide/concepts.zh.md)；服务端与模块
边界见[模块化与边界](modularity.zh.md)。
