"""
A REPL (Read-Eval-Print Loop) implementation using an in-process IPython kernel.

This module provides a `CodeExecutor` class that encapsulates an IPython kernel
running within the same process. It allows for executing Python code, maintaining
state between executions, and defining custom "magic" commands.

The results of executions are encapsulated in an `ExecutionResult` object,
which can be easily serialized to JSON.
"""

# from jupyter_client.session import Session
from typing import Union, List, Optional, Dict, Type
from dataclasses import dataclass, field, asdict
import json

# Magic class registry
_MAGIC_REGISTRY: Dict[str, Type] = {}


def register_magic(name: Optional[str] = None):
    """
    Decorator to register a magic class.

    Usage:
        @register_magic("custom")
        @magics_class
        class CustomMagic(Magics):
            ...

    If name is not provided, it will be derived from the class name.
    """

    def decorator(cls: Type):
        magic_name = name or cls.__name__
        if magic_name in _MAGIC_REGISTRY:
            import warnings

            warnings.warn(
                f"Magic class '{magic_name}' is already registered. "
                f"Previous registration: {_MAGIC_REGISTRY[magic_name]}",
                UserWarning,
            )
        _MAGIC_REGISTRY[magic_name] = cls
        return cls

    return decorator


def get_registered_magics() -> Dict[str, Type]:
    """Get all registered magic classes."""
    return _MAGIC_REGISTRY.copy()


def unregister_magic(name: str) -> bool:
    """Unregister a magic class by name. Returns True if successful."""
    if name in _MAGIC_REGISTRY:
        del _MAGIC_REGISTRY[name]
        return True
    return False


@dataclass
class ExecutionResult:
    """Encapsulates the result of a code execution.

    >>> res_ok = ExecutionResult(status='ok', output='hello')
    >>> print(res_ok.to_json(indent=2))
    {
      "status": "ok",
      "output": "hello",
      "traceback": []
    }
    >>> res_err = ExecutionResult(status='error', traceback=['line 1', 'line 2'])
    >>> print(res_err.to_json(indent=2))
    {
      "status": "error",
      "output": "",
      "traceback": [
        "line 1",
        "line 2"
      ]
    }
    """

    status: str  # 'ok' or 'error'
    output: str = ""
    traceback: Optional[List[str]] = field(default_factory=list)

    def to_json(self, indent: Optional[int] = None) -> str:
        """Serializes the result to a JSON string."""
        return json.dumps(asdict(self), indent=indent)

    def display(self):
        """Prints the execution result to the console."""
        print(f"Status: {self.status}")
        if self.output:
            print(f"Output:\n{self.output}")
        if self.traceback:
            print("Traceback:")
            for line in self.traceback:
                print(line)


class CodeExecutor:
    """A class that encapsulates an in-process IPython kernel for code execution.

    This class provides a simple interface to execute Python code in a persistent
    IPython kernel running within the same process. It handles the creation,
    communication, and shutdown of the kernel.

    By default, `InProcessKernelManager` uses a singleton `InteractiveShell`
    instance. This means that different `CodeExecutor` instances created in the
    same process will share the same underlying shell and, therefore, the same
    execution state (variables, imports, etc.).

    The executor also supports registering and using custom IPython magic commands.

    Attributes
    ----------
    km : InProcessKernelManager
        The kernel manager instance.
    kc : jupyter_client.inprocess.client.InProcessKernelClient
        The kernel client for communication.

    Examples
    --------
    >>> # Create two executor instances.
    >>> executor1 = CodeExecutor()
    >>> executor2 = CodeExecutor()
    >>> # They are different objects...
    >>> executor1 is executor2
    False
    >>> # ...but they share the same underlying kernel state.
    >>> _ = executor1.execute("my_var = 42")
    >>> res = executor2.execute("print(my_var)")
    >>> res.output
    '42'
    >>> # Clean up the resources.
    >>> executor1.shutdown() # doctest: +ELLIPSIS
    <BLANKLINE>
    Shutting down kernel...
    Kernel shut down.
    >>> executor2.shutdown() # doctest: +ELLIPSIS
    <BLANKLINE>
    Shutting down kernel...
    Kernel shut down.
    """

    def __init__(self):
        from ipykernel.inprocess.manager import InProcessKernelManager
        import sys

        # Save original __main__ before IPython replaces it
        original_main = sys.modules.get("__main__")
        
        self.km = InProcessKernelManager()
        self.km.start_kernel()
        self.kc = self.km.client()
        self.kc.start_channels()

        if not self.km.has_kernel:
            return
            
        shell = self.km.kernel.shell
        
        # Copy original __main__ to kernel namespace
        if original_main:
            shell.user_ns.update(original_main.__dict__)

        # Auto-discover and register magic commands
        import importlib
        import pkgutil

        package = __import__(__name__, fromlist=[""])
        for importer, modname, ispkg in pkgutil.iter_modules(package.__path__):
            if modname.startswith("_") or not modname.endswith("_magic"):
                continue

            try:
                importlib.import_module(f"{__name__}.{modname}")
            except Exception as e:
                import warnings
                warnings.warn(f"Failed to import {modname}: {e}", ImportWarning)

        # Register all magic classes
        for magic_name, magic_class in _MAGIC_REGISTRY.items():
            try:
                shell.register_magics(magic_class(shell=shell))
            except Exception as e:
                import warnings
                warnings.warn(f"Failed to register {magic_name}: {e}", ImportWarning)

    def execute(self, code_or_request: Union[str, dict]) -> ExecutionResult:
        """Executes a string of code or a request dictionary in the kernel.

        This method sends the code to the IPython kernel for execution and waits
        for the result. It captures stdout, stderr, and rich display outputs.

        The state of the kernel is preserved across calls. For example, variables
        or functions defined in one execution can be used in subsequent ones.

        Parameters
        ----------
        code_or_request : str or dict
            The code to execute as a string, or a dictionary conforming to the
            format `{'code': '...'}`.

        Returns
        -------
        ExecutionResult
            An object containing the status of the execution, the captured
            output, and any traceback if an error occurred.

        Examples
        --------
        >>> executor = CodeExecutor()
        >>> # Simple execution
        >>> res = executor.execute("a = 10; a + 5")
        >>> res.display()
        Status: ok
        Output:
        15
        >>> # Using a variable from a previous execution
        >>> res2 = executor.execute("print(f'The value of a is {a}')")
        >>> res2.display()
        Status: ok
        Output:
        The value of a is 10
        >>> # Handling an error
        >>> res3 = executor.execute("print(b)")
        >>> res3.display() # doctest: +ELLIPSIS
        Status: error
        Traceback:
        ...
        >>> executor.shutdown() # doctest: +ELLIPSIS
        <BLANKLINE>
        Shutting down kernel...
        Kernel shut down.
        """
        if isinstance(code_or_request, str):
            request = {"code": code_or_request}
        else:
            request = code_or_request

        # Execute the code, this is a non-blocking call
        self.kc.execute(request["code"], silent=False)

        # Wait for and get the execution result
        # For InProcessKernelClient, we can call get_shell_msg directly
        reply = self.kc.get_shell_msg(timeout=5)

        # Check execution status
        content = reply["content"]
        status = content["status"]

        if status == "error":
            traceback = content["traceback"]
            return ExecutionResult(status="error", traceback=traceback)

        # Get all stdout/stderr output from the IOPub channel
        output = []
        while self.kc.iopub_channel.msg_ready():
            sub_msg = self.kc.get_iopub_msg(timeout=5)
            msg_type = sub_msg["header"]["msg_type"]

            if msg_type == "stream":
                output.append(sub_msg["content"]["text"])
            elif msg_type == "execute_result":
                output.append(sub_msg["content"]["data"].get("text/plain", ""))

        result_text = "".join(output).strip()
        return ExecutionResult(status="ok", output=result_text)

    def shutdown(self):
        """Shuts down the kernel and its communication channels.

        This should be called to clean up resources when the executor is no
        longer needed. It stops the client channels and requests the kernel
        manager to shut down the kernel.
        """
        print("\nShutting down kernel...")
        self.kc.stop_channels()
        self.km.shutdown_kernel()
        print("Kernel shut down.")


import code


class DebugConsole(code.InteractiveConsole):
    def __init__(self):
        try:
            self.code_executor = CodeExecutor()
        except Exception as e:
            import warnings
            warnings.warn(f"Failed to initialize CodeExecutor: {e}", ImportWarning)
            self.code_executor = None
        super().__init__()

    def sync_main_namespace(self, namespace=None):
        """Sync namespace to kernel. Use sync_main_namespace(globals()) to sync current script.
        
        Args:
            namespace: Dict to sync. If None, uses sys.modules['__main__'].
        """
        if not self.code_executor:
            return
            
        import sys
        if namespace is None:
            namespace = sys.modules.get("__main__", {})
            if not namespace:
                return
            namespace = namespace.__dict__
        
        shell = self.code_executor.km.kernel.shell
        for key, value in namespace.items():
            if not key.startswith("_"):
                shell.user_ns[key] = value

    def runsource(self, source):
        if self.code_executor is None:
            # Fallback to parent class behavior if CodeExecutor is not available
            return super().runsource(source)

        try:
            code = self.compile(source, "<input>", "single")
        except (OverflowError, SyntaxError, ValueError):
            # Compilation failed - might be incomplete code or magic command
            # Let IPython kernel handle it (it understands magic commands)
            retval = self.code_executor.execute(source)
            self.resetbuffer()
            return retval

        if code is None:  # incomplete code
            return None

        retval = self.code_executor.execute(source)
        self.resetbuffer()
        return retval

    def push(self, code: str):
        """Pushes code to the executor and executes it.

        Examples
        --------
        >>> console = DebugConsole()
        >>> console.push("x = 10")
        '{"status": "ok", "output": "", "traceback": []}'
        >>> console.push("x")
        '{"status": "ok", "output": "10", "traceback": []}'
        >>> result = console.push("print(y)")
        >>> '"status": "error"' in result
        True
        >>> '"traceback":' in result
        True
        """
        if self.code_executor is None:
            # Fallback: try to execute using parent class, but still return JSON format
            try:
                self.buffer.append(code)
                source = "\n".join(self.buffer)
                result = super().runsource(source)
                if result is None:
                    # Incomplete code
                    return json.dumps({"status": "incomplete"})
                # Code executed successfully
                return json.dumps(
                    {"status": "ok", "output": str(result) if result else ""}
                )
            except Exception as e:
                return json.dumps(
                    {"status": "error", "output": "", "traceback": [str(e)]}
                )

        try:
            self.buffer.append(code)
            source = "\n".join(self.buffer)
            retval = self.runsource(source)
            if retval is not None:
                return retval.to_json()
            return json.dumps({})
        except Exception as e:
            import traceback

            traceback.print_exc()


debug_console = DebugConsole()
