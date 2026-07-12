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
   probing $ENDPOINT config probing.torch.profiling
   probing $ENDPOINT tables
   ```

2. **Enable PyTorch tracing**:
   ```bash
   PROBING_TORCH_PROFILING=on python your_script.py
   ```

3. **Wait for data collection**:
   Tables are populated as operations occur. Run training steps first.
   The first TorchProbe step is discovery only (no rows); use `WHERE local_step > 1` if needed.

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
   SELECT MIN(local_step), MAX(local_step) FROM python.torch_trace;
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

1. **Reduce TorchProbe sampling** (not a global sample_rate knob):
   ```bash
   PROBING_TORCH_PROFILING=0.1 python your_script.py
   # or at runtime: set probing.torch.profiling=0.1;
   ```

2. **Lower CPU pprof frequency**:
   ```bash
   probing $ENDPOINT config probing.pprof.sample_freq=50
   ```

3. **Disable torch profiling when not needed**:
   ```bash
   PROBING_TORCH_PROFILING=off python your_script.py
   ```

4. **Use SQL step filters** instead of a warmup schedule:
   ```sql
   SELECT * FROM python.torch_trace WHERE local_step > 10;
   ```

### Query Timeout

**Symptom**: SQL queries take too long.

**Solutions**:

1. **Add LIMIT clause**:
   ```sql
   SELECT * FROM python.torch_trace LIMIT 100;
   ```

2. **Use step filtering**:
   ```sql
   WHERE local_step > (SELECT MAX(local_step) - 10 FROM python.torch_trace)
   ```

3. **Aggregate data**:
   ```sql
   SELECT local_step, AVG(duration) FROM python.torch_trace GROUP BY local_step;
   ```

## Data Issues

### Missing Data

**Symptom**: Expected data not appearing in tables.

**Solutions**:

1. **Verify tables exist and have rows**:
   ```bash
   probing $ENDPOINT tables
   probing $ENDPOINT query "SELECT COUNT(*) AS n FROM python.torch_trace"
   probing $ENDPOINT config probing.torch.profiling
   ```

2. **Confirm training has progressed** — rows append when hooks fire; step 1 of TorchProbe
   is discovery-only (use `WHERE local_step > 1` if needed).

3. **Check TorchProbe is not off**:
   ```bash
   PROBING_TORCH_PROFILING=on python your_script.py
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
