---
template: home.html
title: Probing - Dynamic Performance Profiler for Distributed AI
description: A production-grade performance profiler designed specifically for distributed AI workloads. Zero-intrusion, SQL-powered analytics, real-time introspection.
hide: toc
---

<!-- This content is hidden by the home.html template but indexed for search -->

# Probing

**Probing** is a dynamic performance profiler for distributed AI applications.

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
