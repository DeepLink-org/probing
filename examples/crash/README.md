# Crash

Crash 捕获演示（单进程 / torchrun）。

## 入口

```bash
./examples/crash/run_demo.sh              # record（线程异常，进程不退出）
./examples/crash/run_demo.sh exception    # 主线程崩溃
./examples/crash/run_torchrun.sh          # 多 rank
./examples/crash/run_torchrun.sh exception 1
```

调试时若不想等 grace：`PROBING_CRASH_NO_GRACE=1`。

## 文件

| 文件 | 说明 |
|------|------|
| `crash_demo.py` | 单进程 |
| `crash_torchrun_demo.py` | 多 rank |
| `run_demo.sh` / `run_torchrun.sh` | 入口 |
