# 生产部署

Probing 运行在被观测的 Python 进程内部，因此部署方式会直接影响训练进程的可用性和攻击面。

## 仅本地基线

CLI 与目标进程位于同一主机且有效 UID 相同时，优先使用 PID：

```bash
PROBING=1 python train.py
probing -t <pid> query "SHOW TABLES"
```

该路径使用 Unix 控制 socket，无需 token，也不会引入远程 TCP 信任边界。

## 远程 TCP 基线

在命令历史之外生成 token，通过工作负载的 secret 机制注入，并且只通过可信网络或 TLS
代理暴露 listener：

```bash
export PROBING_SERVER_ADDR="'127.0.0.1:8080'"
export PROBING_AUTH_TOKEN="<random-secret>"
PROBING=1 python train.py
```

内层引号用于把值传成运行时配置的字符串字面量。也可以用 `PROBING_PORT=8080` 在所有
interface 上启用 listener。CLI 读取同一个
`PROBING_AUTH_TOKEN` 并发送 Bearer credential。没有 token 时绑定
`0.0.0.0`，非公开路由将没有认证保护；详见[安全](security.zh.md)。

## 健康检查与启动

liveness 使用 `GET /health`，readiness 使用 `GET /ready`。两者应分开处理：仅因 SQL
引擎仍在初始化就重启存活进程，可能中断训练负载。

默认情况下 engine 初始化失败后 HTTP 服务仍可用，`/ready` 返回 `503`。只有当诊断功能
不可用就必须终止目标负载时，才设置 `PROBING_ENGINE_FAIL_FAST=1`。

## 资源边界

| 边界 | 配置 | 默认值 / 行为 |
|------|------|---------------|
| 请求 body | `PROBING_MAX_REQUEST_SIZE` | 5 MiB |
| HTTP 并发连接 | `PROBING_MAX_CONNECTIONS` | fan-out 并发与 128 的较大值 |
| 联邦查询行数 | `PROBING_GLOBAL_SCAN_MAX_ROWS` | 10,000 |
| 联邦 peer/响应字节 | `PROBING_GLOBAL_RESPONSE_MAX_BYTES` | 16 MiB |
| 联邦物化内存 | `PROBING_GLOBAL_MEMORY_MAX_BYTES` | 每次查询累计 128 MiB |
| Peer 查询超时 | `PROBING_REMOTE_QUERY_TIMEOUT_SECS` | 30 秒 |
| Fan-out 并发 | `PROBING_FANOUT_CONCURRENCY` | 128，并受字节预算进一步收紧 |
| Python 表内存 | `PROBING_TABLE_DEFAULT_MB` | 默认每表 20 MiB，可单独覆盖 |

这些配置是 guardrail，并非整个进程的内存配额；查询算子、collector 数量和冷存储会产生额外成本。

## 数据生命周期

MEMT 是有界 mmap 环，新写入最终会复用旧 chunk。可选 MEMC 冷压缩可以延长保留时间，
但仍是诊断存储，不应作为备份。需要保留数据时，应显式配置 `PROBING_DATA_DIR`、冷存储
大小、age 和 TTL。格式与写入语义见[数据层](../design/data-layer.zh.md)。

## 部署检查清单

- 同主机访问尽量使用 Unix socket。
- TCP 使用非空 token，并在 Probing listener 前终止 TLS。
- 限制 bind address、防火墙规则和 `PROBING_ALLOWED_FILE_DIRS`。
- 除非需要干预，否则保持 MCP 写工具关闭。
- 分别探测 `/health` 和 `/ready`。
- 根据工作负载规模设置请求、连接、federation 和存储限制。
- 明确是否接受 federation 部分结果；否则设置 `PROBING_FANOUT_STRICT=1`。
- 修改线上集群前，先在可丢弃训练任务上验证升级与回滚。
