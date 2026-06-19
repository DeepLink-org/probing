# 常见问题

使用 Probing 时的常见问题及解决方案。

## 连接问题

### 无法连接到进程

**症状**：`probing $ENDPOINT inject` 失败或超时。

**解决方案**：

1. **验证进程存在**：
   ```bash
   ps aux | grep $ENDPOINT
   ```

2. **检查 Linux 要求**：
   注入功能仅在 Linux 上可用。在其他平台上，请在启动时启用：
   ```bash
   PROBING=1 python your_script.py
   ```

3. **检查权限**：
   ```bash
   # 可能需要 sudo 进行注入
   sudo probing $ENDPOINT inject
   ```

### 连接被拒绝（远程）

**症状**：无法连接到远程进程。

**解决方案**：

1. **验证服务器正在运行**：
   ```bash
   # 在远程机器上
   netstat -tlnp | grep $PORT
   ```

2. **检查防火墙**：
   ```bash
   # 允许端口
   sudo ufw allow $PORT
   ```

3. **验证端点格式**：
   ```bash
   export ENDPOINT=hostname:port  # 不只是 hostname
   ```

## 查询问题

### 表不存在

**症状**：`Table 'python.torch_trace' not found`

**解决方案**：

1. **检查 PyTorch 分析是否启用**：
   ```bash
   probing $ENDPOINT eval "
   import probing
   print(probing.get_config())"
   ```

2. **启用 PyTorch 追踪**：
   ```bash
   PROBING_TORCH_PROFILING=on python your_script.py
   ```

3. **等待数据写入**：
   表在训练进行时填充。先运行若干训练 step。
   TorchProbe 第一个 step 为 discovery（无行）；必要时使用 `WHERE step > 1`。

### 结果为空

**症状**：查询没有返回行。

**解决方案**：

1. **检查表内容**：
   ```sql
   SELECT COUNT(*) FROM python.torch_trace;
   ```

2. **验证过滤条件**：
   ```sql
   -- 移除过滤器来调试
   SELECT * FROM python.torch_trace LIMIT 5;
   ```

3. **检查步骤范围**：
   ```sql
   SELECT MIN(step), MAX(step) FROM python.torch_trace;
   ```

## Eval 问题

### 代码执行失败

**症状**：`probing eval` 返回错误或意外结果。

**解决方案**：

1. **检查语法**：
   ```bash
   # 使用正确的引号
   probing $ENDPOINT eval "print('hello')"
   ```

2. **处理导入**：
   ```bash
   # 先导入模块
   probing $ENDPOINT eval "import torch; print(torch.__version__)"
   ```

3. **检查变量作用域**：
   ```bash
   # 使用 globals() 查看可用变量
   probing $ENDPOINT eval "print(list(globals().keys())[:10])"
   ```

## 性能问题

### 开销过高

**症状**：启用 Probing 后应用运行变慢。

**解决方案**：

1. **降低 TorchProbe 采样**（无全局 sample_rate 开关）：
   ```bash
   PROBING_TORCH_PROFILING=ordered:0.1 python your_script.py
   # 或运行时：set probing.torch.profiling=ordered:0.1;
   ```

2. **降低 CPU pprof 频率**：
   ```bash
   probing $ENDPOINT config probing.pprof.sample_freq=50
   ```

3. **不需要时关闭 torch profiling**：
   ```bash
   PROBING_TORCH_PROFILING=off python your_script.py
   ```

4. **用 SQL 过滤 step**，而非 warmup schedule：
   ```sql
   SELECT * FROM python.torch_trace WHERE step > 10;
   ```

### 查询超时

**症状**：SQL 查询耗时太长。

**解决方案**：

1. **添加 LIMIT 子句**：
   ```sql
   SELECT * FROM python.torch_trace LIMIT 100;
   ```

2. **使用步骤过滤**：
   ```sql
   WHERE step > (SELECT MAX(step) - 10 FROM python.torch_trace)
   ```

3. **聚合数据**：
   ```sql
   SELECT step, AVG(duration) FROM python.torch_trace GROUP BY step;
   ```

## 获取帮助

如果仍有问题：

1. **检查日志**：
   ```bash
   probing $ENDPOINT eval "
   import logging
   logging.basicConfig(level=logging.DEBUG)"
   ```

2. **报告问题**：
   [GitHub Issues](https://github.com/DeepLink-org/probing/issues)

3. **包含诊断信息**：
   ```bash
   probing --version
   python --version
   uname -a
   ```
