# Probing - Dynamic Performance Profiler for Distributed AI

<div align="center">
  <img src="probing.svg" alt="Probing Logo" width="200"/>
  
  <p>
    <a href="README.cn.md">中文</a> | 
    <a href="README.md">English</a>
  </p>
</div>

[![PyPI version](https://badge.fury.io/py/probing.svg)](https://badge.fury.io/py/probing)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
[![Downloads](https://pepy.tech/badge/probing)](https://pepy.tech/project/probing)
[![codecov](https://codecov.io/gh/DeepLink-org/probing/graph/badge.svg?token=IRH3F0OI56)](https://codecov.io/gh/DeepLink-org/probing)

> Uncover the Hidden Truth of AI Performance

Probing is a production-grade performance profiler designed specifically for distributed AI workloads. Built on dynamic probe injection technology, it delivers zero-overhead runtime introspection with SQL-queryable performance metrics and cross-node correlation analysis.

### What probing delivers...

### 🔍 **For AI Researchers & Algorithm Engineers**
- **Debug Training Instabilities** - Real-time insight into why training diverges or hangs
- **Optimize Model Performance** - Identify bottlenecks in forward/backward passes
- **Memory Leak Detection** - Track GPU/CPU memory usage across training steps
- **Live Variable Inspection** - Check tensor values, gradients, and model states without stopping training

### 🛠️ **For Framework & Library Developers**  
- **Runtime Framework Analysis** - Understand how your framework performs in real-world usage
- **Zero-Intrusion Profiling** - Profile framework internals without code modifications
- **Production Debugging** - Debug issues reported by users in their actual environments
- **Performance Benchmarking** - Collect real performance data for optimization decisions

### ⚙️ **For System Engineers & MLOps**
- **Production Monitoring** - Monitor AI services without service restarts
- **Resource Optimization** - Analyze resource usage patterns across the cluster
- **Custom Metrics Collection** - Gather any application-specific performance data
- **Distributed Debugging** - Correlate performance issues across multiple nodes

### 🚀 **Core Technical Capabilities**
- **Dynamic Probe Injection** - Attach to running processes without code changes
- **SQL-Powered Analytics** - Use standard SQL to query performance data
- **Live Code Execution** - Run Python code directly in target processes
- **Real-time Stack Analysis** - Capture execution context with variable values

### In contrast with traditional profilers, probing does not...

- **Require Code Instrumentation** - No need to add logging statements, insert timers, or modify your training scripts
- **Force "Break-Then-Fix" Workflow** - No waiting for issues to occur, then spending days trying to reproduce them
- **Lock You Into Fixed Reports** - No more deciphering pre-formatted tables; use SQL to create custom analysis reports that match your specific needs
- **Disrupt Your Workflow** - Attach to running processes without stopping your training jobs or services
- **Force You to Learn New Tools** - Use familiar SQL syntax and Python code for all your analysis needs

## Getting Started

### Installation

```bash
pip install probing
```

### Quick Start (30 seconds)

```bash
# Enable instrumentation at startup
PROBING=1 python train.py

# Or inject into running process
probing -t <pid> inject

# Real-time stack trace analysis
probing -t <pid> backtrace
```

## Core Features

- **Dynamic Probe Injection** - Runtime instrumentation without target application modification
- **Distributed Performance Aggregation** - Cross-node data collection with unified correlation analysis
- **SQL Analytics Interface** - Apache DataFusion-powered query engine with standard SQL syntax
- **Interactive Python REPL** - Live debugging and variable inspection in running processes
- **Production-Grade Overhead** - Efficient sampling strategies maintaining <1% performance impact
- **Time-Series Storage** - Columnar data storage with configurable compression and retention
- **Real-Time Introspection** - Live performance metrics and runtime stack trace analysis
- **Advanced CLI** - Comprehensive command-line interface with process monitoring and management

## Basic Usage

```bash
# Inject performance monitoring
probing -t <pid> inject

# Real-time stack trace analysis
probing -t <pid> backtrace

# Memory usage profiling
probing -t <pid> memory

# Generate flame graphs
probing -t <pid> flamegraph

# Interactive Python REPL (connect to running process)
probing -t <pid> repl

# RDMA Flow Analysis
probing -t <pid> rdma
```

## Advanced Features

### SQL Analytics Interface
```bash
# Memory usage analysis
probing -t <pid> query "SELECT * FROM memory_usage WHERE timestamp > now() - interval '5 min'"

# Performance hotspot analysis
probing -t <pid> query "
  SELECT operation_name, avg(duration_ms), count(*)
  FROM profiling_data 
  WHERE timestamp > now() - interval '5 minutes'
  GROUP BY operation_name
  ORDER BY avg(duration_ms) DESC
"

# Training progress tracking
probing -t <pid> query "
  SELECT epoch, avg(loss), min(loss), count(*) as steps
  FROM training_logs 
  GROUP BY epoch 
  ORDER BY epoch
"
```

### Interactive Python REPL

Probing provides an interactive Python REPL that connects to running processes, allowing you to inspect variables, execute code, and debug in real-time:

```bash
# Connect to a process via REPL
probing -t <pid> repl

# For remote processes
probing -t <host|ip:port> repl
```

Example REPL session:
```python
>>> import torch
>>> # Inspect torch models in the target process
>>> models = [m for m in gc.get_objects() if isinstance(m, torch.nn.Module)]
```

The REPL provides:
- **Live Variable Inspection**: Access all variables in the target process context
- **Code Execution**: Run arbitrary Python code within the target process
- **Real-time Debugging**: Set breakpoints and inspect state without stopping the process

### Distributed Training Analysis
```bash
# Monitor all cluster nodes
probing cluster attach

# Inter-node communication latency
probing -t <pid> query "SELECT src_rank, dst_rank, avg(latency_ms) FROM comm_metrics"

# Cross-node stack trace comparison
probing -t <pid> query "SELECT * FROM python.backtrace"

# GPU utilization analysis
probing -t <pid> query "SELECT avg(gpu_util) FROM gpu_metrics WHERE timestamp > now() - 60"
```

### Memory Analysis
```bash
# Quick memory usage overview
probing -t <pid> memory

# Memory growth trend analysis
probing -t <pid> query "SELECT hour(timestamp), avg(memory_mb) FROM memory_usage GROUP BY hour(timestamp)"

# Memory leak detection
probing -t <pid> query "
  SELECT function_name, sum(allocated_bytes) as total_alloc
  FROM memory_allocations 
  WHERE timestamp > now() - interval '1 hour'
  GROUP BY function_name
  ORDER BY total_alloc DESC
"
```

### Configuration Options
```bash
# Environment variable configuration
export PROBING_SAMPLE_RATE=0.1      # Set sampling rate
export PROBING_RETENTION_DAYS=7     # Data retention period

# View current configuration
probing -t <pid> config

# Dynamic configuration updates
probing -t <pid> config probing.sample_rate=0.05
probing -t <pid> config probing.max_memory=1GB
probing -t <pid> config "probing.rdma.hca.name='mlx5_cx6_0'"
probing -t <pid> config "probing.rdma.sample.rate='5'"
```

## Development

### Prerequisites

Before building Probing from source, ensure you have the following dependencies installed:

```bash
# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install nightly toolchain (required)
rustup toolchain install nightly
rustup default nightly

# Add WebAssembly target for web UI
rustup target add wasm32-unknown-unknown

# Install Dioxus CLI for building WebAssembly frontend
cargo install dioxus-cli

# Install cross-compilation tools (optional, for distribution builds)
cargo install cargo-zigbuild
pip install ziglang
```

### Building from Source

```bash
# Clone repository
git clone https://github.com/reiase/probing.git
cd probing

# Development build (faster compilation)
make

# Production build with cross-platform compatibility
make ZIG=1

# Build web UI separately (optional)
cd web && dx build --release

# Build and install wheel package
make wheel
pip install dist/probing-*.whl --force-reinstall
```

### Testing

prepare your environment:

```bash
# Install dependencies
cargo install cargo-nextest --locked
```

```bash
# Run all tests
make test

# Test with a simple example
PROBING=1 python examples/test_probing.py

# Advanced testing with variable tracking
PROBING_TORCH_PROFILING="on,exprs=loss@train,acc1@train" PROBE=1 python examples/imagenet.py
```

### Project Structure

- `probing/cli/` - Command-line interface
- `probing/core/` - Core profiling engine  
- `probing/extensions/` - Language-specific extensions (Python, C++)
- `probing/server/` - HTTP API server
- `web/` - Web UI source and build output (Dioxus + WebAssembly)
  - `web/dist/` - Web UI build output directory
- `python/` - Python hooks and integration
- `examples/` - Usage examples and demos

### Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feature-name`
3. Make your changes and add tests
4. Run tests: `make test`
5. Submit a pull request

## License

[Apache License 2.0](LICENSE)
