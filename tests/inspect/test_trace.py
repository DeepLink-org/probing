"""Unit tests for probing.inspect.trace module.

This module tests the core tracing functionality including:
- probe decorator
- ProbingTracer class
- trace/untrace functions
- show_trace function
- list_traceable function
"""

import json
import sys
import pytest
from unittest.mock import patch, MagicMock
from types import FrameType

from probing.inspect.trace import (
    probe,
    ProbingTracer,
    trace,
    untrace,
    show_trace,
    list_traceable,
    ProbingTensor,
    traced_functions,
)


class TestProbeDecorator:
    """Test the probe decorator functionality."""

    def test_probe_as_decorator_with_args(self):
        """Test probe decorator with arguments."""

        @probe(watch=["x"], depth=2)
        def test_func(x):
            return x * 2

        result = test_func(5)
        assert result == 10

    def test_probe_as_decorator_without_args(self):
        """Test probe decorator without arguments."""

        @probe
        def test_func(x):
            return x * 2

        result = test_func(5)
        assert result == 10

    def test_probe_as_function(self):
        """Test probe as a function call."""

        def test_func(x):
            return x * 2

        wrapped = probe(test_func, watch=["x"], depth=1)
        result = wrapped(5)
        assert result == 10

    def test_probe_preserves_function_metadata(self):
        """Test that probe preserves function metadata."""

        @probe
        def test_func(x):
            """Test function docstring."""
            return x * 2

        assert test_func.__name__ == "test_func"
        assert test_func.__doc__ == "Test function docstring."


class TestProbingTracer:
    """Test ProbingTracer class."""

    def test_tracer_initialization(self):
        """Test tracer initialization."""
        tracer = ProbingTracer(depth=2, watch=["x", "y"])
        assert tracer.depth == 2
        assert tracer.watch == ["x", "y"]
        assert tracer.count_calls == 0
        assert tracer.count_returns == 0
        assert tracer.watch_impl == {}

    def test_tracer_default_values(self):
        """Test tracer with default values."""
        tracer = ProbingTracer()
        assert tracer.depth == 1
        assert tracer.watch == []
        assert tracer.count_calls == 0
        assert tracer.count_returns == 0

    def test_on_call(self):
        """Test on_call method."""
        tracer = ProbingTracer()
        assert tracer.count_calls == 0
        tracer.on_call()
        assert tracer.count_calls == 1
        tracer.on_call()
        assert tracer.count_calls == 2

    def test_on_return(self):
        """Test on_return method."""
        tracer = ProbingTracer()
        assert tracer.count_returns == 0
        tracer.on_return()
        assert tracer.count_returns == 1
        tracer.on_return()
        assert tracer.count_returns == 2

    def test_outof_depth(self):
        """Test _outof_depth method."""
        tracer = ProbingTracer(depth=2)

        # Initially not out of depth
        assert not tracer._outof_depth()

        # Simulate call depth
        tracer.count_calls = 1
        tracer.count_returns = 0
        assert not tracer._outof_depth()  # depth = 1, not > 2

        tracer.count_calls = 3
        tracer.count_returns = 0
        assert tracer._outof_depth()  # depth = 3 > 2

        tracer.count_calls = 3
        tracer.count_returns = 1
        assert not tracer._outof_depth()  # depth = 2, not > 2

    def test_is_internal_frame(self):
        """Test _is_internal_frame method."""
        tracer = ProbingTracer()

        # Create a mock frame
        mock_frame = MagicMock(spec=FrameType)
        mock_frame.f_code.co_filename = "/path/to/probing/trace.py"

        # This depends on internal_directories, so we test the logic
        result = tracer._is_internal_frame(mock_frame)
        # Result depends on actual internal_directories value
        assert isinstance(result, bool)

    def test_tracer_context_manager(self):
        """Test tracer as context manager."""
        original_trace = sys.gettrace()

        try:
            with ProbingTracer() as tracer:
                assert sys.gettrace() == tracer.trace
                # Tracer should be active
                assert sys.gettrace() is not None

            # Tracer should be restored after context exit
            assert sys.gettrace() == original_trace
        finally:
            # Ensure we restore original trace
            sys.settrace(original_trace)

    def test_tracer_context_manager_with_exception(self):
        """Test tracer context manager with exception."""
        original_trace = sys.gettrace()

        try:
            try:
                with ProbingTracer() as tracer:
                    assert sys.gettrace() == tracer.trace
                    raise ValueError("Test exception")
            except ValueError:
                pass

            # Tracer should still be restored after exception
            assert sys.gettrace() == original_trace
        finally:
            sys.settrace(original_trace)


class TestTraceFunction:
    """Test trace function."""

    def setup_method(self):
        """Clear traced_functions before each test."""
        traced_functions.clear()

    def test_trace_function_by_name(self):
        """Test tracing a function by name."""

        def test_func(x):
            return x * 2

        # Add function to a module for testing
        import __main__

        __main__.test_func = test_func

        # Trace the function
        trace("__main__.test_func", watch=["x"], depth=1)

        # Check that function is traced
        assert "__main__.test_func" in traced_functions

        # Call the traced function
        result = test_func(5)
        assert result == 10

    def test_trace_function_already_traced(self, capsys):
        """Test tracing a function that is already traced."""

        def test_func(x):
            return x * 2

        import __main__

        __main__.test_func = test_func

        # Trace first time
        trace("__main__.test_func")
        assert "__main__.test_func" in traced_functions

        # Try to trace again
        trace("__main__.test_func")

        # Should print warning message
        captured = capsys.readouterr()
        assert "already being traced" in captured.out

    def test_trace_function_not_found(self, capsys):
        """Test tracing a function that doesn't exist."""
        trace("nonexistent.module.function")

        # Should print error message
        captured = capsys.readouterr()
        assert "not found" in captured.out

    def test_trace_function_with_non_string(self):
        """Test trace with non-string function name."""

        def test_func(x):
            return x * 2

        with pytest.raises(NotImplementedError):
            trace(test_func)  # Should raise NotImplementedError


class TestUntraceFunction:
    """Test untrace function."""

    def setup_method(self):
        """Clear traced_functions before each test."""
        traced_functions.clear()

    def test_untrace_function(self):
        """Test untracing a function."""

        def test_func(x):
            return x * 2

        import __main__

        __main__.test_func = test_func

        # Trace the function
        trace("__main__.test_func")
        assert "__main__.test_func" in traced_functions

        # Untrace the function
        untrace("__main__.test_func")
        assert "__main__.test_func" not in traced_functions

    def test_untrace_function_not_traced(self, capsys):
        """Test untracing a function that is not traced."""
        untrace("nonexistent.function")

        # Should print warning message
        captured = capsys.readouterr()
        assert "not being traced" in captured.out

    def test_untrace_function_not_found(self, capsys):
        """Test untracing a function that doesn't exist."""
        # Add to traced_functions but function doesn't exist
        traced_functions["nonexistent.function"] = lambda x: x

        untrace("nonexistent.function")

        # Should print error message
        captured = capsys.readouterr()
        assert "not found" in captured.out

    def test_untrace_function_with_non_string(self):
        """Test untrace with non-string function name."""

        def test_func(x):
            return x * 2

        with pytest.raises(NotImplementedError):
            untrace(test_func)  # Should raise NotImplementedError


class TestShowTrace:
    """Test show_trace function."""

    def setup_method(self):
        """Clear traced_functions before each test."""
        traced_functions.clear()

    def test_show_trace_empty(self):
        """Test show_trace with no traced functions."""
        result = show_trace()
        traced = json.loads(result)
        assert traced == []

    def test_show_trace_with_functions(self):
        """Test show_trace with traced functions."""
        traced_functions["func1"] = lambda x: x
        traced_functions["func2"] = lambda y: y

        result = show_trace()
        traced = json.loads(result)

        assert len(traced) == 2
        assert "func1" in traced
        assert "func2" in traced


class TestListTraceable:
    """Test list_traceable function."""

    def test_list_traceable_no_prefix(self):
        """Test list_traceable without prefix."""
        result = list_traceable()
        functions = json.loads(result)

        # Should return a list
        assert isinstance(functions, list)
        # Should contain some functions
        assert len(functions) > 0

    def test_list_traceable_with_prefix(self):
        """Test list_traceable with prefix."""
        result = list_traceable(prefix="__main__")
        functions = json.loads(result)

        # Should return a list
        assert isinstance(functions, list)
        # All functions should contain the prefix (after the type marker)
        for func in functions:
            assert "__main__" in func

    def test_list_traceable_with_nonexistent_prefix(self):
        """Test list_traceable with nonexistent prefix."""
        result = list_traceable(prefix="nonexistent_prefix_xyz")
        functions = json.loads(result)

        # Should return empty list or list with no matching functions
        assert isinstance(functions, list)


class TestProbingTensor:
    """Test ProbingTensor functionality."""

    @pytest.mark.skipif(
        not pytest.importorskip("torch", reason="torch not available"),
        reason="torch not available",
    )
    def test_probing_tensor_creation(self):
        """Test ProbingTensor creation."""
        import torch

        # Create a ProbingTensor from a regular tensor
        regular_tensor = torch.tensor([1, 2, 3])
        probing_tensor = ProbingTensor(regular_tensor)

        # Should be a tensor
        assert isinstance(probing_tensor, torch.Tensor)
        # Should have the same values
        assert torch.equal(probing_tensor, regular_tensor)

    @pytest.mark.skipif(
        not pytest.importorskip("torch", reason="torch not available"),
        reason="torch not available",
    )
    def test_probing_tensor_format(self):
        """Test ProbingTensor __format__ method."""
        import torch

        regular_tensor = torch.tensor([1.5])
        probing_tensor = ProbingTensor(regular_tensor)

        # Test formatting
        formatted = f"{probing_tensor:.2f}"
        assert isinstance(formatted, str)
        assert "1.50" in formatted or "1.5" in formatted


class TestIntegration:
    """Integration tests for trace functionality."""

    def setup_method(self):
        """Clear traced_functions before each test."""
        traced_functions.clear()

    def test_trace_untrace_cycle(self):
        """Test complete trace/untrace cycle."""

        def test_func(x):
            return x * 2

        import __main__

        __main__.test_func = test_func

        # Trace
        trace("__main__.test_func")
        assert "__main__.test_func" in traced_functions

        # Call traced function
        result = test_func(5)
        assert result == 10

        # Untrace
        untrace("__main__.test_func")
        assert "__main__.test_func" not in traced_functions

        # Function should still work
        result = test_func(5)
        assert result == 10

    def test_probe_with_watch(self):
        """Test probe decorator with watch variables."""

        @probe(watch=["x", "y"], depth=1)
        def test_func(x, y):
            z = x + y
            return z * 2

        result = test_func(3, 4)
        assert result == 14

    def test_probe_with_depth(self):
        """Test probe decorator with depth control."""

        @probe(depth=2)
        def outer_func(x):
            return inner_func(x)

        def inner_func(x):
            return x * 2

        result = outer_func(5)
        assert result == 10

    def test_multiple_traced_functions(self):
        """Test tracing multiple functions."""

        def func1(x):
            return x * 2

        def func2(x):
            return x + 1

        import __main__

        __main__.func1 = func1
        __main__.func2 = func2

        # Trace both functions
        trace("__main__.func1")
        trace("__main__.func2")

        # Both should be traced
        result = show_trace()
        traced = json.loads(result)
        assert "__main__.func1" in traced
        assert "__main__.func2" in traced
        assert len(traced) == 2

        # Untrace one
        untrace("__main__.func1")
        result = show_trace()
        traced = json.loads(result)
        assert "__main__.func1" not in traced
        assert "__main__.func2" in traced
        assert len(traced) == 1


class TestEdgeCases:
    """Test edge cases and error handling."""

    def setup_method(self):
        """Clear traced_functions before each test."""
        traced_functions.clear()

    def test_trace_empty_string(self, capsys):
        """Test trace with empty string."""
        trace("")
        captured = capsys.readouterr()
        # Should handle gracefully
        assert "not found" in captured.out or len(captured.out) == 0

    def test_untrace_empty_string(self, capsys):
        """Test untrace with empty string."""
        untrace("")
        captured = capsys.readouterr()
        # Should handle gracefully
        assert "not being traced" in captured.out or len(captured.out) == 0

    def test_show_trace_after_clear(self):
        """Test show_trace after clearing traced_functions."""
        traced_functions["test"] = lambda x: x
        assert len(json.loads(show_trace())) > 0

        traced_functions.clear()
        result = show_trace()
        traced = json.loads(result)
        assert traced == []

    def test_list_traceable_with_empty_prefix(self):
        """Test list_traceable with empty prefix."""
        result = list_traceable(prefix="")
        functions = json.loads(result)
        # Should return all functions (empty prefix matches everything)
        assert isinstance(functions, list)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
