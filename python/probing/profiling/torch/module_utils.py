import logging

import torch

from ..types import TensorDef

logger = logging.getLogger(__name__)

NAME_CACHE = {}


def module_name(m, name=None):
    global NAME_CACHE
    mid = id(m)
    if mid in NAME_CACHE:
        return NAME_CACHE[mid]
    elif name is not None:
        NAME_CACHE[mid] = name
        return name
    return None


def try_catch(maxtry=3):
    def decorator(func):
        _maxtry = maxtry

        def wrapper(*args, **kwargs):
            try:
                return func(*args, **kwargs)
            except Exception:
                nonlocal _maxtry
                _maxtry -= 1
                if _maxtry > 0:
                    logger.exception(
                        "module_utils.%s failed; retries left=%s",
                        func.__name__,
                        _maxtry,
                    )

        return wrapper

    return decorator


def module_analysis(m, prefix=""):
    if not isinstance(m, torch.nn.Module):
        return
    # Root uses its class name (e.g. "AlexNet"); children are prefixed with the
    # *root-qualified* path ("AlexNet.features", "AlexNet.features.0", …). Without
    # the root prefix, containers become flamegraph siblings of the root instead
    # of nesting under it, so their durations get added and inflate the phase.
    fallback = prefix if prefix else module_get_fullname(m).split(".")[-1]
    name = module_name(m, fallback)
    for n, s in m.named_children():
        module_analysis(s, f"{name}.{n}")


def _cache(func):
    cache = {}

    def wrapper(m, value=None):
        nonlocal cache
        mid = id(m)
        if value is not None:
            cache[mid] = value
        elif mid not in cache:
            cache[mid] = func(m)
            return cache[mid]
        else:
            return cache[mid]

    return wrapper


@_cache
def module_get_fullname(m):
    return f"{m.__module__}.{m.__class__.__name__}"


@_cache
def module_get_params(m):
    return {k: TensorDef.create(v) for k, v in m.named_parameters(recurse=False)}


@_cache
def module_is_container(m):
    return isinstance(m, torch.nn.Module) and len(list(m.children())) > 0


def get_toplevel_module():
    import gc
    import warnings

    import torch

    # A full gc sweep touches unrelated deprecated torch globals (e.g.
    # ``torch.distributed.reduce_op``) whose access emits a FutureWarning. That
    # noise is not about our modules — silence it just for this scan.
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", FutureWarning)
        objs = [obj for obj in gc.get_objects() if isinstance(obj, torch.nn.Module)]
    is_child = set()
    for obj in objs:
        for child in obj.children():
            is_child.add(id(child))
    return [obj for obj in objs if id(obj) not in is_child]
