"""Pure-Python stand-in for ``megatron.core.datasets.helpers_cpp``.

Megatron's mock/indexed GPT datasets import a pybind11 extension built via
``make`` in ``megatron/core/datasets``. On macOS debug hosts that extension is
often missing; this module covers the symbols needed for small ``--mock-data``
runs without a C++ toolchain.
"""

from __future__ import annotations

import logging
import sys
import types
from typing import Any

import numpy as np

logger = logging.getLogger(__name__)


def build_sample_idx_int32(
    sizes: np.ndarray,
    document_indices: np.ndarray,
    sequence_length: int,
    num_epochs: int,
    tokens_per_epoch: int,
    drop_last_partial_sequence: bool = True,
    add_extra_token_to_sequence: int = 1,
) -> np.ndarray:
    return _build_sample_idx(
        sizes,
        document_indices,
        sequence_length,
        num_epochs,
        tokens_per_epoch,
        drop_last_partial_sequence,
        add_extra_token_to_sequence,
        dtype=np.int32,
    )


def build_sample_idx_int64(
    sizes: np.ndarray,
    document_indices: np.ndarray,
    sequence_length: int,
    num_epochs: int,
    tokens_per_epoch: int,
    drop_last_partial_sequence: bool = True,
    add_extra_token_to_sequence: int = 1,
) -> np.ndarray:
    return _build_sample_idx(
        sizes,
        document_indices,
        sequence_length,
        num_epochs,
        tokens_per_epoch,
        drop_last_partial_sequence,
        add_extra_token_to_sequence,
        dtype=np.int64,
    )


def _build_sample_idx(
    sizes: np.ndarray,
    document_indices: np.ndarray,
    sequence_length: int,
    num_epochs: int,
    tokens_per_epoch: int,
    drop_last_partial_sequence: bool,
    add_extra_token_to_sequence: int,
    *,
    dtype: Any,
) -> np.ndarray:
    """Port of ``helpers.cpp`` ``build_sample_idx`` for small mock datasets."""
    sizes = np.asarray(sizes)
    document_idx = np.asarray(document_indices)
    seq_length = int(sequence_length)
    extra = int(add_extra_token_to_sequence)
    epochs = int(num_epochs)
    tokens = int(tokens_per_epoch)

    if drop_last_partial_sequence:
        num_samples = (epochs * tokens - extra) // seq_length
    else:
        num_samples = int(np.ceil(float(epochs * tokens - extra) / seq_length))
    num_samples = max(int(num_samples), 0)

    sample_idx = np.zeros((num_samples + 1, 2), dtype=dtype)
    sample_idx_index = 0
    document_idx_index = 0
    doc_offset = 0
    sample_idx[0, 0] = document_idx_index
    sample_idx[0, 1] = doc_offset
    sample_idx_index = 1

    while sample_idx_index <= num_samples:
        remaining_seq_length = seq_length + extra
        while remaining_seq_length != 0:
            document_index = int(document_idx[document_idx_index])
            document_length = int(sizes[document_index]) - doc_offset
            remaining_seq_length -= document_length
            if remaining_seq_length <= 0:
                doc_offset += remaining_seq_length + document_length - extra
                remaining_seq_length = 0
            else:
                if document_idx_index == document_idx.shape[0] - 1:
                    doc_offset = (
                        int(sizes[int(document_idx[document_idx_index])]) - extra
                    )
                    break
                document_idx_index += 1
                doc_offset = 0
        sample_idx[sample_idx_index, 0] = document_idx_index
        sample_idx[sample_idx_index, 1] = doc_offset
        sample_idx_index += 1

    return sample_idx


def build_mapping(*_args: Any, **_kwargs: Any) -> np.ndarray:
    return np.zeros((0, 3), dtype=np.int64)


def build_blocks_mapping(*_args: Any, **_kwargs: Any) -> np.ndarray:
    return np.zeros((0, 4), dtype=np.int64)


def build_blending_indices(
    dataset_index: np.ndarray,
    dataset_sample_index: np.ndarray,
    weights: np.ndarray,
    num_datasets: int,
    size: int,
    _verbose: bool = False,
) -> None:
    del weights, num_datasets
    for i in range(size):
        dataset_index[i] = 0
        dataset_sample_index[i] = i


def build_exhaustive_blending_indices(
    dataset_index: np.ndarray,
    dataset_sample_index: np.ndarray,
    sizes: np.ndarray,
    num_datasets: int,
) -> None:
    idx = 0
    for d in range(num_datasets):
        for s in range(int(sizes[d])):
            dataset_index[idx] = d
            dataset_sample_index[idx] = s
            idx += 1


def install_helpers_cpp_fallback() -> bool:
    """Register this module as ``megatron.core.datasets.helpers_cpp`` if missing."""
    name = "megatron.core.datasets.helpers_cpp"
    if name in sys.modules:
        return False
    try:
        import megatron.core.datasets  # noqa: F401
    except Exception as exc:
        logger.debug("helpers_cpp fallback skipped (no megatron datasets): %s", exc)
        return False

    mod = types.ModuleType(name)
    mod.build_sample_idx_int32 = build_sample_idx_int32  # type: ignore[attr-defined]
    mod.build_sample_idx_int64 = build_sample_idx_int64  # type: ignore[attr-defined]
    mod.build_mapping = build_mapping  # type: ignore[attr-defined]
    mod.build_blocks_mapping = build_blocks_mapping  # type: ignore[attr-defined]
    mod.build_blending_indices = build_blending_indices  # type: ignore[attr-defined]
    mod.build_exhaustive_blending_indices = (  # type: ignore[attr-defined]
        build_exhaustive_blending_indices
    )
    mod.__probing_fake__ = True  # type: ignore[attr-defined]
    sys.modules[name] = mod
    logger.info("probing.fakes: installed pure-Python helpers_cpp fallback")
    return True
