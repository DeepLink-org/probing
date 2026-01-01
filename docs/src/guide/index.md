# User Guide

Welcome to the Probing User Guide. This section covers the core features and usage patterns.

## Overview

Probing provides three core capabilities for analyzing and debugging your AI applications:

| Capability | Command | Description |
|------------|---------|-------------|
| **eval** | `probing $ENDPOINT eval "..."` | Execute Python code in target process |
| **query** | `probing $ENDPOINT query "..."` | Query performance data with SQL |
| **backtrace** | `probing $ENDPOINT backtrace` | Capture execution stack with variables |

## Getting Started

If you're new to Probing, we recommend starting with these guides in order:

1. **[SQL Analytics](sql-analytics.md)** - Learn the powerful SQL query interface
2. **[Memory Analysis](memory-analysis.md)** - Debug memory leaks and usage patterns
3. **[Debugging](debugging.md)** - Master stack analysis and live debugging
4. **[Troubleshooting](troubleshooting.md)** - Common issues and solutions

## Core Concepts

### Target Endpoint

All Probing commands require a target endpoint, which can be:

- **Process ID**: Local process (e.g., `12345`)
- **Remote Address**: Network endpoint (e.g., `host:8080`)

```bash
# Set target endpoint
export ENDPOINT=12345  # or host:8080
```

### Data Tables

Probing exposes performance data through SQL tables:

| Table | Description |
|-------|-------------|
| `python.backtrace` | Stack trace information |
| `python.torch_trace` | PyTorch execution traces |
| `python.variables` | Variable tracking |
| `information_schema.df_settings` | Configuration settings |

### Workflow Patterns

**Debugging Workflow:**
```bash
# 1. Capture current state
probing $ENDPOINT backtrace

# 2. Inspect specific values
probing $ENDPOINT eval "print(my_variable)"

# 3. Query historical data
probing $ENDPOINT query "SELECT * FROM python.torch_trace"
```

## Advanced Topics

- **[Architecture](../design/architecture.md)** - System design and internals
- **[Distributed](../design/distributed.md)** - Multi-node monitoring
- **[Extensibility](../design/extensibility.md)** - Custom tables and metrics
