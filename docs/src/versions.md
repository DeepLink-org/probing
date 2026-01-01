# Version Compatibility

This page documents Probing version compatibility and changelog.

## Current Version

**Probing v0.6.x** (Latest)

## System Requirements

### Python Version

| Probing Version | Python Support |
|-----------------|----------------|
| 0.6.x | Python 3.9 - 3.12 |
| 0.5.x | Python 3.8 - 3.11 |

### PyTorch Version

| Probing Version | PyTorch Support |
|-----------------|-----------------|
| 0.6.x | PyTorch 2.0+ |
| 0.5.x | PyTorch 1.13+ |

### Operating Systems

- **Linux**: Full support (recommended for production)
- **macOS**: Full support (Intel and Apple Silicon)
- **Windows**: Experimental (WSL2 recommended)

## Changelog

### v0.6.0

**New Features**

- SQL query engine based on DataFusion
- Mermaid diagram support in documentation
- Improved distributed debugging support
- New `torch_trace` table for PyTorch profiling

**Breaking Changes**

- Deprecated `probing.trace()` API, use `probing.enable_torch_profiling()` instead
- Configuration format changed from JSON to TOML

**Bug Fixes**

- Fixed memory leak in long-running sessions
- Improved error messages for invalid SQL queries

### v0.5.0

**New Features**

- Initial PyTorch profiling support
- Memory analysis capabilities
- Basic SQL query support

**Bug Fixes**

- Various stability improvements

## Upgrade Guide

### From v0.5.x to v0.6.x

1. Update Python to 3.9+ if needed
2. Update PyTorch to 2.0+ if needed
3. Update Probing:

```bash
pip install --upgrade probing
```

4. Update configuration files (if using custom config):

```python
# Old format (v0.5.x)
probing.trace(enabled=True)

# New format (v0.6.x)
probing.enable_torch_profiling()
```

## Deprecation Policy

- Major version changes may include breaking changes
- Minor version changes maintain backward compatibility
- Deprecated features show warnings for at least one minor version before removal

## Feature Support Matrix

| Feature | v0.5.x | v0.6.x |
|---------|--------|--------|
| Basic Profiling | ✅ | ✅ |
| SQL Queries | Partial | ✅ |
| PyTorch Tracing | Basic | Full |
| Memory Analysis | Basic | Full |
| Distributed Support | ❌ | ✅ |
| Custom Tables | ❌ | ✅ |
| Web UI | ❌ | Beta |

## Reporting Issues

For bugs and feature requests, please use the [GitHub Issue Tracker](https://github.com/DeepLink-org/probing/issues).

When reporting issues, please include:

- Probing version (`pip show probing`)
- Python version (`python --version`)
- PyTorch version (if applicable)
- Operating system
- Minimal reproduction example
