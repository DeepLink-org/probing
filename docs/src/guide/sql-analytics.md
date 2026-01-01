# SQL Analytics Interface

Probing provides a powerful SQL interface for analyzing performance and monitoring data.

## Overview

The SQL analytics interface transforms complex performance analysis into intuitive database queries. All monitoring data is accessible through standard SQL operations including `SELECT`, `WHERE`, `GROUP BY`, `ORDER BY`, and advanced analytical functions.

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

**`python.torch_trace`** - PyTorch execution traces

```sql
SELECT step, module, stage, duration, allocated
FROM python.torch_trace
WHERE step >= 5
ORDER BY step DESC, seq;
```

Common columns:

- `step` - Training step number
- `seq` - Sequence number within step
- `module` - Module name
- `stage` - Execution stage (forward, backward, step)
- `allocated` - GPU memory allocated (MB)
- `duration` - Execution duration (seconds)

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

# Time-series data for plotting
probing $ENDPOINT query "
  SELECT step, stage, avg(duration), avg(allocated)
  FROM python.torch_trace
  GROUP BY step, stage
" > training_metrics.json
```

## Best Practices

1. **Use step-based filtering** - Always include step constraints for better performance
2. **Limit result sets** - Use `LIMIT` clauses for large datasets
3. **Aggregate appropriately** - Use `GROUP BY` for summary statistics
4. **Test queries incrementally** - Start simple and add complexity gradually
