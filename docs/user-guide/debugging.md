<style type="text/css" rel="stylesheet">
body {
    counter-reset: h2
}
h1 {
  counter-reset: h2;
}
h2 {
  counter-reset: h3;
}
h2::before {
  counter-increment: h2;
  content: counter(h2) ". ";
}
h3::before {
  counter-increment: h3;
  content: counter(h2) "." counter(h3) ". ";
}
</style>

# Debugging Code with Probing

This document introduces how to debug Python application code using Probing. For Probing's overall architecture design, please refer to [Architecture Overview](../advanced/architecture.md).

System debugging has always been a challenge in distributed system development and optimization, especially for heterogeneous distributed training systems, which require comprehensive localization of errors and problems across multiple layers from hardware, system, framework, to model. Debugging requirements for distributed systems mainly focus on the following aspects:

1. **Breakpoints**: Breakpoints are the most basic means of debugging programs. They can be used to observe program state and variable values, helping with BUG analysis and resolution. Breakpoints are divided into two types:
   1. **Location breakpoints**: Users specify a specific function or specific location in code to interrupt program execution and enter the debugger.
   2. **Conditional breakpoints**: Users specify a variable or memory address, and when it changes, program execution is interrupted.
2. **Instrumentation**: Usually inserting logs at target code locations to view variable values or system state.
3. **Crash site capture**: When exceptions occur, capture the crash site immediately or trigger breakpoints for further debugging.

## Debug Methodology

### CPU Debug Implementation

This section discusses CPU-side debugger implementation methods for reference.

#### How to Control Target Processes

When debugging a process, the first step is to obtain control permissions over the target process to pause or resume its execution. In Linux systems, this is mainly achieved through the ptrace system call. The following is the function prototype of the ptrace system call:

```C
#include <sys/ptrace.h>

long ptrace(enum __ptrace_request op, pid_t pid,
            void *addr, void *data);
```

`ptrace` provides a method to control target process execution, allowing the debugger to interact with the target process to achieve debugging functionality. Common values for `__ptrace_request` are:

- `PTRACE_ATTACH`: Attach to target process, making it the current process's tracee.
- `PTRACE_INTERRUPT`: Pause target tracee.
- `PTRACE_CONT`: Resume target process execution.
- `PTRACE_DETACH`: Release target tracee.
- `PTRACE_GETREGS/PTRACE_SETREGS`: Read/write target process registers.
- `PTRACE_PEEKDATA/PTRACE_POKEDATA`: Read/write target process memory, one WORD at a time.
- `/proc/<pid>/mem`: Read/write large blocks of memory.

A typical debugger workflow is as follows:
1. Attach to target process.
2. Insert breakpoints by reading/writing the target process's TEXT segment.
3. Resume target process execution and use `waitpid` to wait for the target process to pause at breakpoint.
4. When target process pauses, view information by reading/writing memory.

#### CPU Breakpoint Debugging

##### Software Breakpoints

X86 processors support a special interrupt instruction (INT 3, 0xCC). When the CPU executes this instruction, it triggers an interrupt for the debugger to capture. Inserting breakpoints requires directly modifying the target process's code segment.

##### Hardware Breakpoints

X86 processors provide a series of debug registers (DR0-DR3)[^dr0_3] that can monitor addresses pointed to by registers without modifying memory. When the target address is accessed or executed, it triggers a CPU interrupt, notifying GDB. Register definitions are as follows[^x86_regs]:

- DR0-DR3: Virtual addresses of breakpoints
- DR6: Status register, stores DEBUG-related status indicator bits.
- DR7: Control register, controls breakpoint-related behavior.

### Python Debug Implementation

#### Trace Mechanism

Python's own debugger is mainly implemented through the trace mechanism[^pytrace]:

```python
sys.settrace(tracefunc)
```

`tracefunc` receives 5 types of events from the Python interpreter:

- call: Execute function call.
- line: Execute a line of code.
- return: Function returns.
- exception: An exception occurs.
- opcode: Execute a bytecode instruction. Since opcode trace has high performance overhead, `f_trace_opcodes` must be set to enable it.

Using the `trace` mechanism, a watch method for Python variables can be implemented:

```python
def trace(self, frame: FrameType, event: AnyStr, arg: Any):
    if event != "line": return self.trace  # Ignore events other than line

    for k, v in self.watch.items():  # Iterate watch list
        if k in frame.f_locals and id(frame.f_locals[k]) != v:  # Detect variable id changes
            print(f"variable update {k} = {frame.f_locals[k]}")
            self.watch[k] = id(frame.f_locals[k])
    return self.trace  # Continue tracing, must return a new trace function each time
```

#### Execution Event Monitoring Mechanism

Since Python 3.12, Python introduced the Execution event monitoring mechanism[^monitoring] to provide Python interpreter internal execution events to various tools. This mechanism is mainly targeted at Debugger, Profiler, and Optimizer developers. Let's look at a simple example of capturing exceptions during Python execution using `sys.monitoring`:

```python
import sys

def hook(*args, **kwargs):  # Define event hook
    print("=== hook", args, kwargs)

# Declare debugging tool
sys.monitoring.use_tool_id(sys.monitoring.DEBUGGER_ID, "debugging")

# Enable RAISE event for exceptions
sys.monitoring.set_events(sys.monitoring.DEBUGGER_ID, sys.monitoring.events.RAISE)

# Register hook for exception event
sys.monitoring.register_callback(
    sys.monitoring.DEBUGGER_ID,
    sys.monitoring.events.RAISE,
    hook,
)

# Test code
def foo(a):
    b = 2
    bar(a, b)

def bar(a, b):
    c = a+b
    raise Exception('error')

foo(1)
```

When executing the above code, when an exception is triggered in the `bar` function, the following message will be printed:
```
=== hook (<code object bar at 0x104bbcb70, file "b.py", line 24>, 32, Exception('error')) {}
```

##### Registering and Using Tools

A core concept in monitoring is **tool**, used to distinguish different tools and avoid conflicts. **Tool**-related APIs are as follows:
```
sys.monitoring.use_tool_id(tool_id: int, name: str, /) → None
Declare use of **tool**, must be declared before use.

sys.monitoring.free_tool_id(tool_id: int, /) → None
Release **tool**

sys.monitoring.get_tool(tool_id: int, /) → str | None
Get **tool**
```

**Tool** can be defined with any integer ID. For convenience, the system predefines several tool IDs:
```
sys.monitoring.DEBUGGER_ID = 0
sys.monitoring.COVERAGE_ID = 1
sys.monitoring.PROFILER_ID = 2
sys.monitoring.OPTIMIZER_ID = 5
```

##### Events

Defines which execution phase events will be sent to **tool**. Currently supported events include:
- **BRANCH**(sys.monitoring.events.BRANCH)
- **CALL**: A call in Python code (event occurs before the call).
- **C_RAISE**: An exception raised from any callable, except for Python functions (event occurs after the exit).
- **C_RETURN**: Return from any callable, except for Python functions (event occurs after the return).
- **EXCEPTION_HANDLED**: An exception is handled.
- **INSTRUCTION**: A VM instruction is about to be executed.
- **JUMP**: An unconditional jump in the control flow graph is made.
- **LINE**: An instruction is about to be executed that has a different line number from the preceding instruction.
- **PY_RESUME**: Resumption of a Python function (for generator and coroutine functions), except for throw() calls.
- **PY_RETURN**: Return from a Python function (occurs immediately before the return, the callee's frame will be on the stack).
- **PY_START**: Start of a Python function (occurs immediately after the call, the callee's frame will be on the stack)
- **PY_THROW**: A Python function is resumed by a throw() call.
- **PY_UNWIND**: Exit from a Python function during exception unwinding.
- **PY_YIELD**: Yield from a Python function (occurs immediately before the yield, the callee's frame will be on the stack).
- **RAISE**: An exception is raised, except those that cause a STOP_ITERATION event.
- **RERAISE**: An exception is re-raised, for example at the end of a finally block.
- **STOP_ITERATION**: An artificial StopIteration is raised; see the STOP_ITERATION event.

Events support boolean operations, such as `PY_RETURN | PY_START` representing handling both events simultaneously.

##### Registering Callback Functions

```python
sys.monitoring.register_callback(tool_id: int, event: int, func: Callable | None, /) → Callable | None
```
Used to register callback functions. The return value is the old callback function. To unregister a callback function, set the `func` parameter to `None`.

##### Enabling Events for Specific Functions

In addition to globally enabling events, events can also be enabled for specific functions:
```python
sys.monitoring.set_local_events(tool_id: int, code: CodeType, event_set: int, /) → None
```
Here the `code` parameter can be a function. Locally enabling events can effectively narrow the scope of event impact and reduce performance impact.

### Exception Capture

```python
sys.excepthook(type, value, traceback)
```

By registering `excepthook`, uncaught exceptions in Python processes can be captured, for example:

```python
import sys

def handle(t, v, tb):
    print("=== exception handler, ", t, v, tb)

def foo(a):
    b = 2
    bar(a, b)

def bar(a, b):
    raise Exception('error')

sys.excepthook = handle

foo(1)
```

`excepthook` can capture the stack, but cannot capture the crash site and local variables.

## Challenges in Distributed Training Debugging

### Typical Scenarios

Distributed heterogeneous training typically faces the following scenarios:

- **Data Parallelism**: Multiple nodes simultaneously process different data subsets and synchronize gradients in parameter servers or ring communication.
- **Model Parallelism**: Split the model into different parts, running on different nodes.
- **Heterogeneous Training**: Training uses heterogeneous computing devices like GPUs, executing asynchronously with CPUs on each node.
- **Fault Recovery**: In distributed training, node failures may occur frequently, requiring effective fault localization and recovery mechanisms.

Distributed debugging requirements include:
1. Distributed breakpoints to observe execution status of each process.
2. Distributed variable observation to monitor key variables of each process.
3. Distributed hooks: Focus on values and changes of key variables or tensors.
4. Distributed backtrace: Observe execution stack of each process.

### Main Challenges

1. **Distributed Breakpoint Debugging**:
   - Role differences exist between multiple nodes (e.g., different TP/PP/DP roles).
   - Breakpoints in collective communication scenarios affect overall execution flow.
   - Traditional breakpoint debugging tools are difficult to adapt to distributed environments.
   - Need to design lightweight and semantically-aware observation mechanisms.

2. **Distributed Variable Observation**:
   - High overhead for variable acquisition and serialization.
   - Limited network transmission bandwidth.
   - Complex data aggregation and display.
   - Trade-off between real-time performance and system performance.

3. **Distributed Hooks and Traceback**:
   - Capture key actions or variable updates, especially synchronization primitives and asynchronous logic in communication libraries.
   - Obtain execution stack of each process and aggregate analysis.

4. **Scale Scalability**:
   - Log data volume increases dramatically with number of nodes.
   - Difficult to aggregate and analyze debugging information.
   - Storage and query performance bottlenecks.
   - Large visualization challenges.

## Probing's Distributed Debug Solution

Probing currently provides mechanisms that can well support the development of distributed debug capabilities:

1. **Distributed Probes**:
   - Probing's probes can serve the role of C/C++ ptrace and Python Trace.
   - Probing's probes support remote control and can be used to control all probes in a cluster.
   - Can leverage sys.monitoring to implement lightweight tracing, minimizing performance impact.

2. **Data Processing**:
   - Probing has a built-in Query engine, good at processing large amounts of local data.
   - The Query engine's distributed capabilities can help Probing automatically manage cluster-level distributed data processing.
   - The Query engine internally implements efficient data compression.

3. **Automation**:
   - The Query engine provides programming capabilities.
   - Standardized SQL query statements can leverage large models to automatically generate SQL.

### High-Performance Tracer

For Python 3.12 and above, efficient tracers can be implemented using `sys.monitoring`. In tracer implementation, the minimum scope principle should be followed, i.e., control the Python code affected by the tracer to be as small as possible.

1. **Control Effective Scope**: `sys.monitoring` supports bytecode-based tracing, i.e., using `sys.monitoring.set_local_events` to trace only bytecode of specific functions.
2. **Control Event Types**: Finer-grained events have higher trace overhead. Therefore, when selecting events, the following principles should be followed:
   1. Prefer trigger events, such as `RAISE` exception throwing events.
   2. Next consider function-level events like `PY_START` or `RETURN`.
   3. Finally consider fine-grained events like `LINE`.

For Python versions before 3.12, the `trace` function can be used, but its impact scope needs to be manually controlled:
1. Use `with` statements to control trace scope, ensuring tracer only affects specific function calls:

```python
def probe(func=None):
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        tracer = ProbingTracer(depth, watch)
        with tracer:
            return func(*args, **kwargs)

    return wrapper
```

2. Control trace depth and close trace in time:

```python
# Inject trace guard into call stack
frame.f_locals["__trace_guard__"] = TracerGuard()

class TracerGuard:
    def __init__(self, callback=None):
        self.trace = sys.gettrace()  # Save trace function
        sys.settrace(None)           # Disable trace

    def __del__(self):
        sys.settrace(self.trace)     # Restore trace
```

### Variable Observation and Hooks

For tracking variable changes, the following approach can be implemented:
1. Create a Tensor subclass.
2. Use torch's dispatch mechanism to dispatch tensor modification computations to `__torch_function__`.
3. Use tracer to replace tensors of interest in the call stack with `HookedTensor`.

```python
class HookedTensor(torch.Tensor, FakeProbingTensor):
    def __format__(self, format_spec):
        return f"{self.item().__format__(format_spec)}"

    @classmethod
    def __torch_function__(cls, func, types, args=(), kwargs=None):
        if kwargs is None:
            kwargs = {}
        if (
            func is not torch.Tensor.__repr__
            # and func is not torch.Tensor.__format__
            and func is not torch.Tensor.__str__
            and func.__name__.endswith("_")
            and not func.__name__.startswith("__")
        ):
            old_val = f"{args}"
            ret = super().__torch_function__(func, types, args, kwargs)
            ret_val = f"{args}"
            print(
                f"probing: tensor update with {func.__name__}: {old_val} => {ret_val}"
            )
            return ret
        return super().__torch_function__(func, types, args, kwargs)
```

## Practical Debugging Cases

### Case 1: Memory Leak Debugging

Use Probing's memory monitoring capabilities to locate memory leaks in Python applications:

```python
# Enable memory tracking
import probing

# Monitor memory usage in training loop
with probing.span("memory_usage"):
    for epoch in range(num_epochs):
        train_model(epoch)
        
# Analyze memory usage patterns through SQL queries
```

### Case 2: Distributed Training Synchronization Issues

Monitor communication and synchronization issues in distributed training:

```python
# Monitor collective communication operations
with probing.span("collective_ops"):
    torch.distributed.all_reduce(tensor)
    
# Analyze execution time differences across different nodes
```

## Best Practices

1. **Minimize Performance Impact**: Only enable tracing in necessary code segments.
2. **Choose Event Types Appropriately**: Select appropriate monitoring events based on debugging needs.
3. **Leverage SQL Analysis**: Use Probing's SQL interface for efficient data analysis.
4. **Distributed Coordination**: Pay attention to synchronization and coordination issues in distributed environments.

## References

- [Memory Analysis Guide](memory-analysis.md)
- [Distributed Training Analysis](distributed.md)
- [SQL Analytics Interface](sql-analytics.md)

[^ptrace]: https://www.man7.org/linux/man-pages/man2/ptrace.2.html
[^dr0_3]: https://sandpile.org/x86/drx.htm
[^x86_regs]: https://wiki.osdev.org/CPU_Registers_x86#DR0_-_DR3
[^pytrace]: https://docs.python.org/3/library/sys.html
[^monitoring]: https://docs.python.org/3/library/sys.monitoring.html#module-sys.monitoring
