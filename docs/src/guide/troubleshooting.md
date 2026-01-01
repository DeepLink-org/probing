# Troubleshooting

Common issues and their solutions when using Probing.

## Connection Issues

### Cannot Connect to Process

**Symptom**: `probing $ENDPOINT inject` fails or times out.

**Solutions**:

1. **Verify process exists**:
   ```bash
   ps aux | grep $ENDPOINT
   ```

2. **Check Linux requirement**:
   Injection only works on Linux. On other platforms, start your process with:
   ```bash
   PROBING=1 python your_script.py
   ```

3. **Check permissions**:
   ```bash
   # May need sudo for injection
   sudo probing $ENDPOINT inject
   ```

### Connection Refused (Remote)

**Symptom**: Cannot connect to remote process.

**Solutions**:

1. **Verify server is running**:
   ```bash
   # On remote machine
   netstat -tlnp | grep $PORT
   ```

2. **Check firewall**:
   ```bash
   # Allow port
   sudo ufw allow $PORT
   ```

3. **Verify endpoint format**:
   ```bash
   export ENDPOINT=hostname:port  # Not just hostname
   ```

## Query Issues

### Table Not Found

**Symptom**: `Table 'python.torch_trace' not found`

**Solutions**:

1. **Check if PyTorch profiling is enabled**:
   ```bash
   probing $ENDPOINT eval "
   import probing
   print(probing.get_config())"
   ```

2. **Enable PyTorch tracing**:
   ```bash
   PROBING_TORCH_PROFILING=on python your_script.py
   ```

3. **Wait for data collection**:
   Tables are populated as operations occur. Run some training steps first.

### Empty Results

**Symptom**: Query returns no rows.

**Solutions**:

1. **Check table contents**:
   ```sql
   SELECT COUNT(*) FROM python.torch_trace;
   ```

2. **Verify filter conditions**:
   ```sql
   -- Remove filters to debug
   SELECT * FROM python.torch_trace LIMIT 5;
   ```

3. **Check step range**:
   ```sql
   SELECT MIN(step), MAX(step) FROM python.torch_trace;
   ```

## Eval Issues

### Code Execution Fails

**Symptom**: `probing eval` returns error or unexpected result.

**Solutions**:

1. **Check syntax**:
   ```bash
   # Use proper quoting
   probing $ENDPOINT eval "print('hello')"
   ```

2. **Handle imports**:
   ```bash
   # Import modules first
   probing $ENDPOINT eval "import torch; print(torch.__version__)"
   ```

3. **Check variable scope**:
   ```bash
   # Use globals() to see available variables
   probing $ENDPOINT eval "print(list(globals().keys())[:10])"
   ```

### Import Errors

**Symptom**: `ModuleNotFoundError` in eval.

**Solutions**:

1. **Check if module is loaded**:
   ```bash
   probing $ENDPOINT eval "import sys; print('torch' in sys.modules)"
   ```

2. **Use try-except**:
   ```bash
   probing $ENDPOINT eval "
   try:
       import torch
       print(torch.__version__)
   except ImportError:
       print('torch not available')"
   ```

## Performance Issues

### High Overhead

**Symptom**: Application runs slower with Probing.

**Solutions**:

1. **Reduce sampling rate**:
   ```bash
   probing $ENDPOINT config probing.sample_rate=0.01
   ```

2. **Disable unused features**:
   ```bash
   PROBING_TORCH_PROFILING=off python your_script.py
   ```

3. **Use targeted profiling**:
   Only enable profiling for specific modules or operations.

### Query Timeout

**Symptom**: SQL queries take too long.

**Solutions**:

1. **Add LIMIT clause**:
   ```sql
   SELECT * FROM python.torch_trace LIMIT 100;
   ```

2. **Use step filtering**:
   ```sql
   WHERE step > (SELECT MAX(step) - 10 FROM python.torch_trace)
   ```

3. **Aggregate data**:
   ```sql
   SELECT step, AVG(duration) FROM python.torch_trace GROUP BY step;
   ```

## Data Issues

### Missing Data

**Symptom**: Expected data not appearing in tables.

**Solutions**:

1. **Verify profiling is active**:
   ```bash
   probing $ENDPOINT eval "
   import probing
   print(probing.is_profiling_active())"
   ```

2. **Check data retention**:
   ```bash
   probing $ENDPOINT config | grep retention
   ```

3. **Force data flush**:
   ```bash
   probing $ENDPOINT eval "
   import probing
   probing.flush()"
   ```

### Incorrect Values

**Symptom**: Data values seem wrong.

**Solutions**:

1. **Verify units**:
   - Memory is typically in MB
   - Duration is in seconds

2. **Check for aggregation**:
   ```sql
   -- Sum vs individual values
   SELECT SUM(allocated) vs SELECT allocated
   ```

3. **Validate manually**:
   ```bash
   probing $ENDPOINT eval "
   import torch
   print(torch.cuda.memory_allocated() / 1024**2)"  # MB
   ```

## Platform-Specific Issues

### Linux

- **ptrace errors**: May need `CAP_SYS_PTRACE` capability
- **SELinux**: May need to adjust policies

### macOS

- **Injection not supported**: Use `PROBING=1` at startup
- **SIP restrictions**: May affect some features

### Windows

- **Limited support**: Only query/eval with pre-enabled processes

## Getting Help

If you're still stuck:

1. **Check logs**:
   ```bash
   probing $ENDPOINT eval "
   import logging
   logging.basicConfig(level=logging.DEBUG)"
   ```

2. **Report issue**:
   [GitHub Issues](https://github.com/DeepLink-org/probing/issues)

3. **Include diagnostics**:
   ```bash
   probing --version
   python --version
   uname -a
   ```
