---
template: home.html
title: Probing - Dynamic Performance Profiler for Distributed AI
description: A production-grade performance profiler designed specifically for distributed AI workloads. Zero-intrusion, SQL-powered analytics, real-time introspection.
hide: toc
---

<!-- This content is hidden by the home.html template but indexed for search -->

# Probing

**Probing** is a dynamic performance profiler for distributed AI applications.

## 🎯 Why Probing?

### Pain Points of Traditional Profilers

| Problem | Traditional Approach | Probing Solution |
|---------|---------------------|------------------|
| **Code modification required** | Add logging, timers, decorators | ✅ Dynamic injection, zero code changes |
| **Fixed report formats** | Predefined tables and charts | ✅ SQL queries, custom analysis |
| **Service restart needed** | Must stop and restart | ✅ Runtime attachment |
| **High learning curve** | Different syntax per tool | ✅ Familiar SQL + Python |
| **Distributed is hard** | Analyze each node separately | ✅ Unified cross-node view |

### Core Technical Advantages

=== "🔧 Dynamic Probe Injection"

    Professional-grade code injection based on ptrace:

    - No source code modification required
    - Supports x86_64 and aarch64 architectures
    - Complete state save and restore mechanism
    - Production-safe implementation

=== "📊 SQL Query Engine"

    Built on Apache DataFusion:

    - Standard SQL syntax, no new language to learn
    - Millisecond query response
    - Complex aggregations, window functions
    - Plugin-based data source extension

=== "🐍 Remote REPL"

    Execute Python directly in target process:

    - Inspect any variable or object
    - Modify runtime state in real-time
    - No need to stop training jobs
    - Full Python environment access

=== "🌐 Distributed Support"

    Native multi-node support:

    - Unified cross-node queries
    - Automatic process discovery
    - Communication latency analysis
    - Cluster-wide performance view

## 🔄 Comparison with Alternatives

| Feature | Probing | py-spy | Perfetto | torch.profiler |
|:--------|:-------:|:------:|:--------:|:--------------:|
| **Zero Intrusion** | ✅ | ✅ | ❌ | ❌ |
| **Dynamic Injection** | ✅ | ❌ | ❌ | ❌ |
| **SQL Queries** | ✅ | ❌ | ❌ | ❌ |
| **Remote REPL** | ✅ | ❌ | ❌ | ❌ |
| **Distributed Support** | ✅ | ❌ | ✅ | ⚠️ |
| **AI Framework Integration** | ✅ | ❌ | ⚠️ | ✅ |
| **Web UI** | ✅ | ❌ | ✅ | ✅ |

## Key Features

- **Zero Intrusion** - Attach to running processes without code changes
- **SQL Analytics** - Query performance data with standard SQL
- **Live Execution** - Run Python code in target processes
- **Stack Analysis** - Capture call stacks with variable values
- **Distributed Ready** - Monitor processes across multiple nodes

## Quick Start

```bash
# Install
pip install probing

# Inject into running process
probing -t <pid> inject

# Query performance data
probing -t <pid> query "SELECT * FROM python.torch_trace LIMIT 10"

# Remote REPL debugging
probing -t <pid> repl
```

## Use Cases

- **Training Debugging** - Debug training instabilities and hangs
- **Memory Analysis** - Track GPU/CPU memory usage
- **Performance Profiling** - Identify bottlenecks in model execution
- **Production Monitoring** - Monitor AI services without restarts

## Community

- [GitHub Repository](https://github.com/DeepLink-org/probing)
- [Issue Tracker](https://github.com/DeepLink-org/probing/issues)
- [PyPI Package](https://pypi.org/project/probing/)
