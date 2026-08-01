# HTTP 与 MCP API

本页将公开 surface 映射到权威契约，不重复维护每个 endpoint 字段。

## Surface 总览

| Surface | 入口 | 契约 |
|---------|------|------|
| SQL | `POST /query`、`POST /query/dto` | `probing-proto` DTO 与 API spec |
| Server 资源 | `/apis/*` | 显式 Axum route |
| Extension 资源 | `/apis/<extension>/*` | `ProbeExtension` / `@ext_handler` |
| 配置 | `/config/{key}` 与 SQL `SET` | Extension option |
| WebSocket REPL | `GET /ws` | CLI REPL client |
| MCP | `/mcp` | Streamable HTTP tools 与 resources |
| 健康检查 | `GET /health`、`GET /ready` | Liveness 与 engine readiness JSON |

面向人的 endpoint 清单、认证形式、响应 header 和状态码维护在
[`probing/server/API.md`](https://github.com/DeepLink-org/probing/blob/main/probing/server/API.md)。
机器可读 SSOT 是
[`tests/regression/spec/api_spec.json`](https://github.com/DeepLink-org/probing/blob/main/tests/regression/spec/api_spec.json)，
并由契约测试约束。

## 认证

本地 Unix 连接使用相同有效 UID 的 peer credential。只有 `PROBING_AUTH_TOKEN` 非空时，
TCP route 才受认证保护；health 和静态路径仍公开。启用远程访问前请阅读[安全](../operations/security.zh.md)。

## MCP 能力模型

读工具包括 `query`、`describe_tables`、skill 规划/执行、节点列表和 cluster query。
`set_config` 与 `eval_python` 在 `PROBING_MCP_ALLOW_WRITE=1` 前不可用。

Schema resources：

| URI | 内容 |
|-----|------|
| `probing://schema/catalog` | 所有已注册表与列文档 |
| `probing://schema/{schema}/{table}` | 单表文档 |

## 错误与部分结果语义

- 非法请求使用 HTTP `4xx`；engine 不可用和契约规定的集群不完整结果使用 `503`。
- 集群部分结果保留 body，供客户端检查失败节点与丢弃 peer batch；consumer 必须同时检查
  HTTP 状态和响应 metadata。
- Federation broadcast 查询默认要求静态有界的顶层 `LIMIT`，coordinator 物化行数受
  `PROBING_GLOBAL_SCAN_MAX_ROWS` 限制。
- peer/最终响应 body 受 `PROBING_GLOBAL_RESPONSE_MAX_BYTES` 限制，累计 protocol/Arrow 物化受
  `PROBING_GLOBAL_MEMORY_MAX_BYTES` 限制。预算耗尽返回 `503`；远端耗尽可以作为显式标记的 partial
  result 返回。
- 分层 fan-out 通过 `X-Probing-Response-Max-Bytes` 和 `X-Probing-Memory-Max-Bytes` 传播有效预算；
  接收端只允许这些 header 收紧本地限制。

CLI 与 Python 签名不属于 wire protocol，见 [CLI 与 Python API](../api-reference.zh.md)。
