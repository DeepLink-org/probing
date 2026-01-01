# SQL 分析接口

Probing 提供强大的 SQL 接口用于分析性能和监控数据。

## 概览

SQL 分析接口将复杂的性能分析转化为直观的数据库查询。所有监控数据都可以通过标准 SQL 操作访问，包括 `SELECT`、`WHERE`、`GROUP BY`、`ORDER BY` 和高级分析函数。

## 基本查询结构

```bash
probing $ENDPOINT query "SELECT columns FROM table WHERE conditions"
```

## 核心表

### 配置和元数据

**`information_schema.df_settings`** - 系统配置和设置

```sql
SELECT * FROM information_schema.df_settings
WHERE name LIKE 'probing.%';
```

### Python 命名空间表

**`python.backtrace`** - 堆栈跟踪信息

```sql
SELECT * FROM python.backtrace LIMIT 10;
```

常用列：

- `ip` - 指令指针（用于原生帧）
- `file` - 源文件名
- `func` - 函数名
- `lineno` - 行号
- `depth` - 堆栈深度
- `frame_type` - 帧类型（'Python' 或 'Native'）

## PyTorch 集成

监控 PyTorch 应用时，可用额外的表：

**`python.torch_trace`** - PyTorch 执行跟踪

```sql
SELECT step, module, stage, duration, allocated
FROM python.torch_trace
WHERE step >= 5
ORDER BY step DESC, seq;
```

常用列：

- `step` - 训练步数
- `seq` - 步内序号
- `module` - 模块名
- `stage` - 执行阶段（forward、backward、step）
- `allocated` - GPU 已分配内存（MB）
- `duration` - 执行时长（秒）

## 高级分析

### 时间序列分析

**内存随时间增长：**

```sql
SELECT
  step,
  stage,
  avg(allocated) as avg_memory_mb,
  max(allocated) as peak_memory_mb
FROM python.torch_trace
WHERE step > (SELECT max(step) - 10 FROM python.torch_trace)
GROUP BY step, stage
ORDER BY step, stage;
```

**滚动平均：**

```sql
SELECT
  step,
  module,
  duration,
  AVG(duration) OVER (
    PARTITION BY module
    ORDER BY step, seq
    ROWS BETWEEN 4 PRECEDING AND CURRENT ROW
  ) as avg_duration_5_samples
FROM python.torch_trace
WHERE step > (SELECT max(step) - 5 FROM python.torch_trace);
```

### 性能分析

**最慢操作排名：**

```sql
SELECT
  module,
  stage,
  count(*) as execution_count,
  avg(duration) as avg_duration,
  max(duration) as max_duration
FROM python.torch_trace
WHERE step > (SELECT max(step) - 10 FROM python.torch_trace)
  AND duration > 0
GROUP BY module, stage
ORDER BY avg_duration DESC
LIMIT 10;
```

## 聚合函数

### 统计函数

```sql
SELECT
  module,
  stage,
  count(*) as total_executions,
  avg(duration) as mean_duration,
  percentile_cont(0.5) WITHIN GROUP (ORDER BY duration) as median_duration,
  percentile_cont(0.95) WITHIN GROUP (ORDER BY duration) as p95_duration,
  min(duration) as min_duration,
  max(duration) as max_duration
FROM python.torch_trace
WHERE duration > 0
GROUP BY module, stage;
```

### 窗口函数

```sql
SELECT
  step,
  allocated,
  LAG(allocated) OVER (ORDER BY step, seq) as prev_memory,
  LEAD(allocated) OVER (ORDER BY step, seq) as next_memory,
  ROW_NUMBER() OVER (ORDER BY allocated DESC) as memory_rank
FROM python.torch_trace
WHERE step > (SELECT max(step) - 5 FROM python.torch_trace);
```

## 数据导出

结果可以导出用于进一步分析：

```bash
# 导出为 JSON
probing $ENDPOINT query "SELECT * FROM python.torch_trace" > torch_traces.json

# 时间序列数据用于绘图
probing $ENDPOINT query "
  SELECT step, stage, avg(duration), avg(allocated)
  FROM python.torch_trace
  GROUP BY step, stage
" > training_metrics.json
```

## 最佳实践

1. **使用基于步数的过滤** - 始终包含步数约束以获得更好的性能
2. **限制结果集** - 对大数据集使用 `LIMIT` 子句
3. **适当聚合** - 使用 `GROUP BY` 获取汇总统计
4. **渐进式测试查询** - 从简单开始，逐步增加复杂度
