# 从这里开始

本节先建立一条可工作的进程连接，再介绍 SQL 模型和分布式执行。

## 选择连接方式

| 方式 | 用途 | 安全边界 | 第一个命令 |
|------|------|----------|------------|
| 进程内、本地 | 启动时启用 Probing | Unix peer credential；有效 UID 必须相同 | `PROBING=1 python train.py` |
| 注入、本地 | 附着到已有 Linux 进程 | Unix peer credential；有效 UID 必须相同 | `probing -t <pid> inject` |
| 远程 TCP | 跨主机或集群查询 | 配置后使用 token；应在 TLS 后部署 | `probing -t <host>:<port> query …` |

TCP 监听绑定到非 loopback 地址前，请先完成[安全检查清单](../operations/security.zh.md)。

## 推荐顺序

1. [安装指南](../installation.zh.md)——安装 wheel 并确认平台支持。
2. [快速开始](../quickstart.zh.md)——连接、抓取 backtrace、执行 SQL。
3. [核心模型](../guide/concepts.zh.md)——理解 catalog、endpoint 和 step 坐标。
4. [运行与运维](../operations/index.zh.md)——配置健康检查、资源限制和认证。

## 完成检查

目标进程能够响应下面的有界查询，即表示基本链路可用：

```bash
probing -t <pid-or-host:port> query "SHOW TABLES"
```

下一步可阅读 [SQL 分析](../guide/sql-analytics.zh.md)或使用
[诊断 Skill](../guide/skills.zh.md)中的预定义工作流。
