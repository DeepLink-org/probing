# Job tracker

作业开始 / 结束 hook 示例（集群作业元数据上报骨架）。

## 入口

```bash
./examples/job-tracker/run.sh
./examples/job-tracker/run.sh via-init
```

## 文件

| 文件 | 说明 |
|------|------|
| `job_tracker.py` | HTTP 上报示例 |
| `job_tracker_via_init.py` | 仅本地打印的轻量版 |
