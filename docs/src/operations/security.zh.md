# 安全

Probing 能读取进程状态、查询遥测、读取允许目录中的文件，并可通过相应 endpoint 执行
Python。应将控制 endpoint 视为对训练进程的特权访问。

## 信任边界

| Surface | 默认鉴权 | 传输安全 | 重要影响 |
|---------|----------|----------|----------|
| 本地 Unix socket | 有效 UID 必须与目标进程相同 | 本机内核 IPC | 同一 UID 下的其他进程被信任 |
| 远程 TCP | `PROBING_AUTH_TOKEN` 非空前无认证 | 明文 HTTP | 不要暴露未配置的 listener |
| MCP 读工具 | 认证连接可用 | 与 endpoint 相同 | 可读取 SQL 和诊断元数据 |
| MCP 写工具 | `PROBING_MCP_ALLOW_WRITE=1` 前禁用 | 与 endpoint 相同 | 可修改配置、执行 Python |
| HTTP eval/config 路由 | 配置 TCP auth 后受其保护 | 与 endpoint 相同 | MCP 写开关不会禁用直接 HTTP 能力 |

## 本地模式

Unix listener 在 HTTP 处理前读取操作系统 peer credential；客户端有效 UID 与 server
有效 UID 不同时拒绝连接。该过程不依赖调用者提供的 UID header 或 token，因此不存在
通过请求字段伪造 UID 的路径。

这个边界默认同一账号下的进程相互信任。互不信任的任务应使用不同操作系统身份或更强的
sandbox。

## 远程 TCP 模式

在 server 启动前设置高熵 token：

```bash
export PROBING_AUTH_TOKEN="<random-secret>"
export PROBING_SERVER_ADDR="'127.0.0.1:8080'"
```

客户端可以发送 `Authorization: Bearer <token>`、以 token 为密码的 HTTP Basic auth，
或 `X-Probing-Token`。程序化客户端优先使用 Bearer；CLI 会自动读取
`PROBING_AUTH_TOKEN`。

token 未设置或为空时，TCP 认证关闭。Probing 本身不终止 TLS；应使用反向代理、service
mesh、SSH tunnel 或等价加密通道。即使配置了 token，也应限制网络可达范围。

## 公开路径

TCP 认证 middleware 有意保留以下公开路径：

- `/health` 与 `/ready`
- `/`、`/index.html`、`/static/*` 和 favicon 路径

不要在公开静态资源或健康响应中放置 secret 与工作负载详情。

## 能力控制

- 仅诊断用途的 Agent 应保持 `PROBING_MCP_ALLOW_WRITE` 未设置。
- 启用 MCP 写能力后可调用 `set_config` 与 `eval_python`，写调用会记录审计日志。
- 直接 HTTP/CLI eval 是独立能力；网络认证控制调用者，但 MCP 开关不是全局 eval 开关。
- 限制 `PROBING_ALLOWED_FILE_DIRS`；file API 还包含内置允许目录。
- `GET /apis/overview` 会过滤 token/secret 形式的环境变量，但该 endpoint 仍应视为敏感。

## Token 运维

- 通过调度系统或 secret manager 分发，不要写入源码或命令行参数。
- 每个信任域使用独立 token，怀疑泄露后立即轮换。
- 运行时修改 `server.auth_token` 会改变 middleware credential，应协调客户端切换。
- 不要将 token 放入 URL query、日志、截图或诊断包。

## 安全检查清单

- [ ] 已确认本地访问不足，确实需要 TCP。
- [ ] 每个非 loopback TCP listener 都配置了非空 token。
- [ ] TLS 与网络策略保护 TCP 链路。
- [ ] 文件目录和 MCP 写能力符合最小权限。
- [ ] 主机可以接受同 UID 进程互相信任。
- [ ] 公开 health/static 路径没有敏感信息。
- [ ] Token 轮换和事件吊销有明确负责人。

Endpoint 与状态码见 [HTTP 与 MCP API](../reference/http-api.zh.md)，精确配置见
[环境变量](../reference/env-vars.zh.md)。
