"""Bottom-layer patches applied before importing real Megatron."""

from __future__ import annotations

import logging
import os

logger = logging.getLogger(__name__)


def _noop_compile(fn=None, *args, **kwargs):  # noqa: ANN001
    """Drop-in for ``torch.compile`` that never invokes dynamo."""
    del args, kwargs
    if fn is not None:
        return fn
    return lambda f: f


def apply_bottom_patches() -> None:
    """Disable torch.compile / dynamo paths that break on fake Triton + macOS."""
    os.environ.setdefault("TORCHDYNAMO_DISABLE", "1")
    os.environ.setdefault("TORCH_COMPILE_DISABLE", "1")

    try:
        import torch

        if hasattr(torch, "compile"):
            torch.compile = _noop_compile  # type: ignore[method-assign, assignment]
    except Exception as exc:
        logger.debug("torch.compile patch skipped: %s", exc)

    try:
        import triton

        if not getattr(triton, "__version__", None):
            triton.__version__ = "3.0.0+probing.fakes"
    except Exception as exc:
        logger.debug("triton version stamp skipped: %s", exc)


def skip_dataset_cpp_helpers() -> None:
    """Prefer a pure-Python ``helpers_cpp`` when the pybind11 extension is absent.

    Real indexed datasets should still compile ``helpers.so`` via Megatron's Makefile;
    this path is for ``--mock-data`` macOS smoke runs.
    """
    try:
        from probing.fakes.helpers_cpp import install_helpers_cpp_fallback

        install_helpers_cpp_fallback()
    except Exception as exc:
        logger.debug("helpers_cpp fallback skipped: %s", exc)

    try:
        from megatron.core.datasets import utils as ds_utils

        def _noop_compile_helpers() -> None:
            logger.info(
                "probing.fakes: skipping megatron dataset C++ helpers compile "
                "(using pure-Python helpers_cpp fallback)"
            )

        ds_utils.compile_helpers = _noop_compile_helpers  # type: ignore[method-assign]
    except Exception as exc:
        logger.debug("compile_helpers patch skipped: %s", exc)
