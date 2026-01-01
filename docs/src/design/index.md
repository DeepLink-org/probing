# Design Overview

## Why Probing?

### The Pythonic Advantage

Python's dominance in AI stems from one core principle: **everything feels like Python**. Whether you're using pandas, PyTorch, or NumPy, you can **talk to them pythonically**—the same `print()`, iteration, and attribute access patterns work everywhere.

### How Distributed Systems Break This

As AI models scale to distributed clusters, something fundamental breaks: **distributed systems aren't Pythonic**. Single-machine debugging feels natural—`print(model.parameters())`, `loss.item()`, `torch.cuda.memory_allocated()`—but distributed debugging forces you into system administration tools: `kubectl get nodes`, SSH sessions, log file parsing, monitoring dashboards.

### Probing's Mission

Probing's core mission is simple: **make distributed systems feel Pythonic again**. Your cluster, nodes, and distributed processes become accessible through familiar interfaces. Instead of context-switching between tools, you stay in Python and **talk to your distributed system pythonically**.

## Design Principles

### 🔍 Zero Intrusion

- No code modifications required
- No environment setup changes needed
- No workflow disruptions
- Dynamic probe injection into running processes

### 🎯 Zero Learning Curve

- Standard SQL interface for data analysis
- Familiar database query patterns
- Intuitive command-line tools
- Web-based dashboard for visualization

### 📦 Zero Deployment Burden

- Single binary deployment (Rust-based)
- Static compilation with minimal dependencies
- Linux-first design with query/eval support on other platforms
- Elastic scaling capabilities

## Design Documents

| Document | Description |
|----------|-------------|
| [Architecture](architecture.md) | System structure and components |
| [Profiling](profiling.md) | Performance data collection |
| [Debugging](debugging.md) | Debugging capabilities |
| [Distributed](distributed.md) | Multi-node support |
| [Extensibility](extensibility.md) | Custom tables and metrics |
