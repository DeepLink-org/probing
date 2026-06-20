# SQL Analytics Interface

Probing provides a powerful SQL interface for analyzing performance and monitoring data.

## Overview

The SQL analytics interface transforms complex performance analysis into intuitive database queries. All monitoring data is accessible through standard SQL operations including `SELECT`, `WHERE`, `GROUP BY`, `ORDER BY`, and advanced analytical functions.

**Table schemas:** [SQL Tables](../reference/sql-tables.md). **Terminology:** [Core Concepts](concepts.md).

## Basic Query Structure

```bash
probing $ENDPOINT query "SELECT columns FROM table WHERE conditions"
```

## Core Tables

### Configuration and Metadata

**`information_schema.df_settings`** - System configuration and settings

```sql
SELECT * FROM information_schema.df_settings
WHERE name LIKE 'probing.%';
```

### Python Namespace Tables

**`python.backtrace`** - Stack trace information

```sql
SELECT * FROM python.backtrace LIMIT 10;
```

Common columns:

- `ip` - Instruction pointer (for native frames)
- `file` - Source file name
- `func` - Function name
- `lineno` - Line number
- `depth` - Stack depth
- `frame_type` - Frame type ('Python' or 'Native')

## PyTorch Integration

When monitoring PyTorch applications, additional tables become available:

**`python.torch_trace`** — TorchProbe module hooks (long-running sampled telemetry).

```sql
SELECT step, module, stage, duration, allocated
FROM python.torch_trace
WHERE step > 1 AND duration > 0
ORDER BY step DESC, seq;
```

The first training step is discovery (no rows). Skip cold-start steps with `WHERE step > N`.

Common columns:

- `step` — training step (aligned with optimizer steps)
- `seq` — hook order within the step
- `module` — module name
- `stage` — `pre forward`, `post forward`, `pre step`, `post step` (not `forward`/`backward` literals; backward not collected by default)
- `allocated` — GPU memory allocated (MB); CUDA only
- `duration` — stage duration (seconds); use post rows (`stage LIKE 'post %'`) for timings

Sampling (`PROBING_TORCH_PROFILING`):

- `ordered:rate` — `rate` = probability each step is sampled; one module rotates per sampled step
- `random:rate` — every step sampled; `rate` = per-hook probability after the offset-0 anchor

Also stamped on each row: `global_step`, `rank`, `world_size`, `role` (parallel placement key).
See [SQL Tables — torch_trace](../reference/sql-tables.md#python-torch_trace).

## Collective communication (`python.comm_collective`) {#python-comm_collective}

Records `torch.distributed` collectives with wall-clock `duration_ms` (no NCCL plugin required).

```sql
SELECT global_step, rank, role, op, duration_ms, bytes
FROM python.comm_collective
WHERE global_step > (SELECT max(global_step) - 20 FROM python.comm_collective)
ORDER BY duration_ms DESC
LIMIT 20;
```

Join with module work on the same rank and role:

```sql
SELECT c.global_step, c.role, c.op, c.duration_ms AS comm_ms,
       t.module, t.duration AS module_sec
FROM python.comm_collective c
JOIN python.torch_trace t
  ON c.global_step = t.global_step AND c.rank = t.rank AND c.role = t.role
WHERE c.duration_ms > 5 AND t.stage LIKE 'post %' AND t.duration > 0
LIMIT 50;
```

Run built-in diagnostics: `probing $ENDPOINT skill run slow_rank` or `comm_bottleneck`.

## Federated queries (`global.*`)

On a cluster master endpoint, query **`global.<schema>.<table>`** to fan out to registered peers.
Each row gets `_host`, `_addr`, `_rank`, and `_role` (see [Distributed](../design/distributed.md)).

**Slow rank by parallel role** (align ranks that share the same TP/PP/DP placement):

```sql
SELECT _role, _rank, avg(duration_ms) AS avg_ms, max(duration_ms) AS max_ms
FROM global.python.comm_collective
WHERE global_step > (SELECT max(global_step) - 50 FROM global.python.comm_collective)
GROUP BY _role, _rank
ORDER BY avg_ms DESC;
```

CLI equivalent:

```bash
probing -t rank0:8080 cluster query "
SELECT _role, _rank, avg(duration_ms) AS avg_ms
FROM global.python.comm_collective
GROUP BY _role, _rank
ORDER BY avg_ms DESC"
```

## Advanced Analytics

### Time-Series Analysis

**Memory growth over time:**

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

**Rolling averages:**

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

### Performance Analysis

**Top slowest operations:**

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

## Aggregation Functions

### Statistical Functions

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

### Window Functions

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

## Data Export

Results can be exported for further analysis:

```bash
# Export to JSON
probing $ENDPOINT query "SELECT * FROM python.torch_trace" > torch_traces.json

# 时间序列数据用于绘图
probing $ENDPOINT query "
  SELECT step, stage, avg(duration), avg(allocated)
  FROM python.torch_trace
  GROUP BY step, stage
" > step_metrics.json
```

## Best Practices

1. **Use step-based filtering** - Always include step constraints for better performance
2. **Limit result sets** - Use `LIMIT` clauses for large datasets
3. **Aggregate appropriately** - Use `GROUP BY` for summary statistics
4. **Test queries incrementally** - Start simple and add complexity gradually
