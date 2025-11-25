from typing import Any, Iterable, List, Optional, Union


def _get_ray():
    """Lazy import ray module.

    Centralized here so extensions can share the same helper.
    """
    try:
        import ray

        return ray
    except ImportError as exc:  # pragma: no cover - environment dependent
        raise ImportError(
            "Ray is not installed. Please install it with: pip install ray"
        ) from exc


def _get_attr(obj, key, default=None):
    """Get attribute from mapping or object, trying multiple keys.

    Examples
    --------
    Single key on dict:

    >>> _get_attr({"a": 1}, "a")
    1

    Multiple keys on dict (first hit wins):

    >>> _get_attr({"b": 2}, ["a", "b", "c"])
    2

    Missing keys on dict falls back to default:

    >>> _get_attr({"a": 1}, ["x", "y"], default=0)
    0

    Single attribute on object:

    >>> class Obj:
    ...     foo = 10
    >>> _get_attr(Obj(), "foo")
    10

    Multiple attributes on object:

    >>> class Obj2:
    ...     bar = 20
    >>> _get_attr(Obj2(), ["foo", "bar"])
    20

    Parameters
    ----------
    obj : Any
        Source object or mapping.
    key : str | list[str]
        Single key/name or a list of candidates to try in order.
    default : Any, optional
        Fallback value if none of the keys are found.
    """
    keys = key if isinstance(key, (list, tuple)) else [key]

    for k in keys:
        if isinstance(obj, dict):
            if k in obj:
                return obj[k]
        else:
            if hasattr(obj, k):
                return getattr(obj, k)

    return default
