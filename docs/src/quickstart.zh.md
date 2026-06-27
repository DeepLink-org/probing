# 快速开始

本指南带你使用 Probing 检查运行中的训练进程。假定你已完成[安装](installation.zh.md)。
读完这篇，你将能够附着到进程、抓取调用栈、并对实时性能数据执行 SQL 查询。

## 找到并附着到进程

首先找到你想检查的 Python 进程：

```bash
# 查找训练进程
pgrep -f "python.*train"
# → 27891

# 或者带更多上下文
ps aux | grep python | grep -v grep
```

设置端点环境变量以便后续命令更简洁：

```bash
export ENDPOINT=27891
```

在 Linux 上，可以直接附着到运行中的进程：

```bash
probing $ENDPOINT inject
```

macOS 或 Windows 不支持 injection——需要以 `PROBING=1 python train.py` 的方式启动
进程。`inject` 是 Probing 中唯一 Linux 专属的命令，其余所有操作（query、eval、
backtrace、skill）在服务器运行后全平台通用。

## 首次诊断

服务器已在目标进程中运行后，抓取调用栈看看主线程在做什么：

```bash
probing $ENDPOINT backtrace
```

backtrace 会填充 `python.backtrace`——主线程当前调用栈的瞬时视图，混合 Python
和原生帧。用 SQL 查询它：

```bash
probing $ENDPOINT query "
  SELECT func, file, lineno, depth, frame_type
  FROM python.backtrace
  ORDER BY depth LIMIT 10
"
```

你会看到函数名、源文件路径，以及每帧是 Python 还是原生代码。深度 0 是最内层帧
（当前正在执行的位置）。

如需查看调用栈之外的实时状态，用 `eval` 在目标进程中执行任意 Python：

```bash
probing $ENDPOINT eval "import torch; print(f'CUDA 可用: {torch.cuda.is_available()}')"
probing $ENDPOINT eval "
  import gc, torch
  gc.collect()
  alloc = torch.cuda.memory_allocated() / 1024**2
  reserved = torch.cuda.memory_reserved() / 1024**2
  print(f'GPU: 已分配 {alloc:.0f}MB, 已预留 {reserved:.0f}MB')
"
```

## 三个常见工作流

以下是你会经常用到的真实场景。

### 训练卡住了

最常见的场景：训练突然停止进展。进程还活着但什么也不做。

先用 backtrace 看主线程状况：

```bash
probing $ENDPOINT backtrace
probing $ENDPOINT query "SELECT func, file, lineno FROM python.backtrace ORDER BY depth LIMIT 5"
```

最内层帧（depth 0）精确显示卡在哪里。如果调用栈显示 `ncclAllReduce` 或
`torch.distributed` 调用，这是通信 hang。如果是模型 Python 代码，则是计算瓶颈。

接着检查线程状态——collective 可能卡住了但其他线程正常：

```bash
probing $ENDPOINT eval "
  import threading
  for t in threading.enumerate():
      print(f'{t.name}: alive={t.is_alive()}, daemon={t.daemon}')
"
```

### GPU 内存在增长

内存随 step 不断增长——泄漏或累积。查询 torch_trace 表看每步分配趋势：

```bash
probing $ENDPOINT query "
  SELECT local_step, AVG(allocated) as avg_mb, MAX(allocated_delta) as max_delta_mb
  FROM python.torch_trace
  GROUP BY local_step
  ORDER BY local_step
"
```

关注 `allocated_delta` 值不会在 step 间归零——说明迭代之间内存未被释放。
配合 `eval` 强制 GC 并检查当前状态：

```bash
probing $ENDPOINT eval "import gc, torch; gc.collect(); torch.cuda.empty_cache()"
```

如果 GC 和 cache 清理后内存回落，问题是 Python 侧引用环。否则是 CUDA 侧
泄漏或不断增长的 workspace。

### 找出最慢的模块和操作

定位模型中哪个部分最耗时：

```bash
probing $ENDPOINT query "
  SELECT module, stage, AVG(duration) as avg_duration, COUNT(*) as calls
  FROM python.torch_trace
  WHERE stage IN ('post forward', 'post step')
  GROUP BY module, stage
  ORDER BY avg_duration DESC
  LIMIT 10
"
```

过滤到 `post forward` 和 `post step` stage 可获得执行耗时（`pre` 行携带时间锚点，
`post` 行携带实际 duration）。结果告诉你哪个模块、哪个方向最耗时。

分布式训练时，加 `global.` 前缀跨 rank 对比：

```bash
probing -t <master> query "
  SELECT _rank, module, stage, AVG(duration) as avg_duration
  FROM global.python.torch_trace
  WHERE stage IN ('post forward', 'post step')
  GROUP BY _rank, module, stage
  ORDER BY avg_duration DESC
  LIMIT 20
"
```

`_rank` 是查询时附加的联邦标签——工作原理见[核心概念](guide/concepts.zh.md)。

## 下一步

三个命令——`backtrace`、`eval`、`query`——覆盖了日常诊断的大部分场景。详细文档
见 [API 参考](api-reference.zh.md)。

更深入的分析模式（跨表 JOIN、时间分桶、统计聚合）见 [SQL 分析](guide/sql-analytics.zh.md)。
多节点调试从[分布式](design/distributed.zh.md)开始。
