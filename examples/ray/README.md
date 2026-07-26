# Ray

Ray 任务 / Actor 自动 tracing 与 async RL 骨架 demo。

## 依赖

`ray`

## 入口

```bash
./examples/ray/run_tracing.sh
PROBING_PORT=8080 ./examples/ray/run_job_actor.sh
```

## 文件

| 文件 | 说明 |
|------|------|
| `ray_tracing_example.py` | `_tracing_startup_hook` |
| `ray_job_actor_span_demo.py` | slime 风格 actor spans |
