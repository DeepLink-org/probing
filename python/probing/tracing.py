"""Python wrapper for tracing functionality.

This module provides Python-friendly wrappers around the Rust tracing implementation.
"""

import functools
from typing import Callable, Optional, Any

# Import from the internal Rust module
from probing._tracing import Span, current_span, _span_raw as span_raw


def span(*args, **kwargs):
    """Create a span context manager or decorator.

    This function can be used in three ways:

    1. As a context manager:
       ```python
       with span("operation_name") as s:
           # do work
           pass
       ```

    2. As a context manager with attributes:
       ```python
       with span("operation_name", attr1=value1, attr2=value2) as s:
           # access attributes via s.attr1, s.attr2
           pass
       ```

    3. As a decorator:
       ```python
       @span("operation_name")
       def my_function():
           pass

       @span
       def my_function2():
           pass
       ```

    Args:
        *args: First argument is the span name (string), or a callable for @span decorator
        **kwargs: Additional keyword arguments. Special keys:
            - kind: Optional span kind
            - code_path: Optional code path
            - All other kwargs become span attributes

    Returns:
        Either a SpanWrapper (for context manager) or a decorator function
    """
    # Extract special parameters
    kind = kwargs.pop("kind", None)
    code_path = kwargs.pop("code_path", None)

    # Handle @span (without arguments) - no args and no kwargs
    if len(args) == 0 and not kwargs:

        def decorator(func: Callable) -> Callable:
            @functools.wraps(func)
            def wrapper(*wargs, **wkwargs):
                with span_raw(func.__name__, kind=kind, code_path=code_path) as s:
                    return func(*wargs, **wkwargs)

            return wrapper

        return decorator

    # Handle @span(func) - first arg is a callable
    if len(args) == 1 and callable(args[0]):
        func = args[0]

        @functools.wraps(func)
        def wrapper(*wargs, **wkwargs):
            with span_raw(func.__name__, kind=kind, code_path=code_path) as s:
                return func(*wargs, **wkwargs)

        return wrapper

    # Handle @span("name") or with span("name")
    if len(args) == 1 and isinstance(args[0], str):
        name = args[0]

        # Create a wrapper that supports both decorator and context manager usage
        class SpanWrapper:
            def __init__(
                self,
                name: str,
                kind: Optional[str],
                code_path: Optional[str],
                attrs: dict,
            ):
                self.name = name
                self.kind = kind
                self.code_path = code_path
                self.attrs = attrs
                self._span = None

            def __call__(self, func: Callable) -> Callable:
                """This is @span("name") - used as decorator"""

                @functools.wraps(func)
                def wrapper(*wargs, **wkwargs):
                    # Create span with attributes set during creation
                    if self.attrs:
                        # Use span() function which handles attributes during creation
                        with span(
                            self.name,
                            kind=self.kind,
                            code_path=self.code_path,
                            **self.attrs,
                        ) as s:
                            return func(*wargs, **wkwargs)
                    else:
                        with span_raw(
                            self.name, kind=self.kind, code_path=self.code_path
                        ) as s:
                            return func(*wargs, **wkwargs)

                return wrapper

            def __enter__(self):
                """This is with span("name") - used as context manager"""
                # Get current span for parent relationship
                parent = current_span()

                if parent:
                    self._span = Span.new_child(
                        parent, self.name, kind=self.kind, code_path=self.code_path
                    )
                else:
                    self._span = Span(
                        self.name, kind=self.kind, code_path=self.code_path
                    )

                # Set initial attributes during creation (before __enter__)
                if self.attrs:
                    # Convert Python dict to dict that can be passed to _set_initial_attrs
                    attrs_dict = dict(self.attrs)
                    if hasattr(self._span, "_set_initial_attrs"):
                        try:
                            self._span._set_initial_attrs(attrs_dict)
                        except Exception as e:
                            # If setting attributes fails, log but continue
                            import warnings

                            warnings.warn(f"Failed to set initial attributes: {e}")

                # Enter the span context manager
                # This pushes the span to the thread-local stack
                # We need to call __enter__ to push it to the stack, but we can return the span directly
                entered = self._span.__enter__()
                # __enter__ returns PyRef<Span> which should be automatically converted
                # But to be safe, we return the span object directly
                return self._span

            def __exit__(self, *args):
                """Exit context manager"""
                if self._span:
                    return self._span.__exit__(*args)
                return False

        return SpanWrapper(name, kind, code_path, kwargs)

    # Default: use as context manager with first arg as name
    if len(args) > 0:
        name = args[0]
        if not isinstance(name, str):
            raise TypeError("span() requires a string name as the first argument")

        # Get current span for parent relationship
        parent = current_span()

        if parent:
            span_obj = Span.new_child(parent, name, kind=kind, code_path=code_path)
        else:
            span_obj = Span(name, kind=kind, code_path=code_path)

        # Set initial attributes during creation
        if kwargs:
            attrs_dict = dict(kwargs)
            if hasattr(span_obj, "_set_initial_attrs"):
                span_obj._set_initial_attrs(attrs_dict)

        return span_obj

    raise TypeError("span() requires at least one argument")


# Add convenience methods to Span class
def _span_with(name: str, kind: Optional[str] = None, code_path: Optional[str] = None):
    """Create a span using Span.with_() - convenience method.

    Example:
        ```python
        with Span.with_("my_operation") as s:
            pass
        ```
    """
    parent = current_span()
    if parent:
        return Span.new_child(parent, name, kind=kind, code_path=code_path)
    else:
        return Span(name, kind=kind, code_path=code_path)


def _span_decorator(
    name: Optional[str] = None,
    kind: Optional[str] = None,
    code_path: Optional[str] = None,
):
    """Create a decorator using Span.decorator() - convenience method.

    Example:
        ```python
        @Span.decorator(name="my_function", kind="server_op")
        def my_function():
            pass
        ```
    """

    def decorator(func: Callable) -> Callable:
        @functools.wraps(func)
        def wrapper(*wargs, **wkwargs):
            span_name = name or func.__name__
            with span_raw(span_name, kind=kind, code_path=code_path) as s:
                return func(*wargs, **wkwargs)

        return wrapper

    return decorator


# Monkey-patch Span class with convenience methods
Span.with_ = staticmethod(_span_with)
Span.decorator = staticmethod(_span_decorator)


def add_event(name: str, *, attributes: Optional[list] = None):
    """Add an event to the current active span.

    This is a convenience function that adds an event to the current span
    without needing to explicitly get the span object.

    Args:
        name: The name of the event
        attributes: Optional list of attribute dictionaries or tuples.
                   Each attribute can be:
                   - A dict: {"key": "value"}
                   - A tuple: ("key", value)

    Raises:
        RuntimeError: If there is no active span in the current context.

    Example:
        ```python
        with span("operation") as s:
            add_event("step1")
            add_event("step2", attributes=[{"key": "value"}])
        ```
    """
    current = current_span()
    if current is None:
        raise RuntimeError("No active span in current context. Cannot add event.")

    current.add_event(name, attributes=attributes)
