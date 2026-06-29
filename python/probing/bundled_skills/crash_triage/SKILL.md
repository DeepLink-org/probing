---
name: crash_triage
description: >
  Crash triage: grep PROBING CRASH in rank logs; read spill JSON under
  {PROBING_DATA_DIR}/crash/<pid>/latest.json
category: triage
tags: [crash, error, exception, startup, 报错, 崩溃]
keywords:
  en: ['crash', 'error', 'exception', 'failed', 'startup failure']
  zh: ['崩溃', '报错', '异常', '启动失败', '挂了']
---

# Crash triage

训练崩溃或启动失败时：

1. **日志**：在各 rank stderr / launcher 日志中搜索 ``PROBING CRASH``
2. **Spill**：``{PROBING_DATA_DIR}/crash/<pid>/latest.json``（与 memtable 同根目录，默认 Linux ``/dev/shm/probing``，macOS ``$TMPDIR/probing``）
3. **Signal crash**：同目录下 ``signal-latest.json``

torchrun 级联退出后 memtable 不可用——以日志 + spill 为准。

## Related skills

- 进程仍存活但无进展 → skill: training_hang
