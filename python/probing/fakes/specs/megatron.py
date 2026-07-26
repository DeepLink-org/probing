"""Scripted Megatron-shaped module tree for macOS / meta-device debugging.

Exposes enough of the Megatron-LM / megatron-core import surface for a
``pretrain_gpt.py``-style entry to boot under ``probing.fakes``:

* ``megatron.core.parallel_state`` / ``megatron.core.mpu``
* ``megatron.core.enums.ModelType``
* ``megatron.core.models.gpt.GPTModel`` (tiny meta ``nn.Module``)
* ``megatron.training``: ``get_args``, ``pretrain``, ``print_rank_0``, …
* ``megatron.training.training.train_step``
* ``megatron.training.global_vars.get_args``

This is **not** Megatron-Core. Real forward/backward on ``meta`` will fail;
``pretrain()`` advances probing step/role coordinates with a scripted loop.
"""

from __future__ import annotations

import sys
import types
from types import SimpleNamespace
from typing import Any, Optional

from ..registry import FakeSpec, register

_STATE: dict[str, Any] = {
    "initialized": False,
    "tp": 0,
    "pp": 0,
    "dp": 0,
    "ep": 0,
    "cp": 0,
    "tp_size": 1,
    "pp_size": 1,
    "dp_size": 1,
    "iteration": 0,
    "micro_batches": 1,
    "train_calls": 0,
}

_ARGS: SimpleNamespace | None = None
_CACHE: dict[str, types.ModuleType] = {}


def default_args(**overrides: Any) -> SimpleNamespace:
    """Megatron-like args namespace used by fake ``get_args()``."""
    base = dict(
        num_layers=2,
        hidden_size=64,
        num_attention_heads=4,
        seq_length=32,
        max_position_embeddings=32,
        micro_batch_size=1,
        global_batch_size=2,
        train_iters=4,
        eval_iters=0,
        save_interval=0,
        tensor_model_parallel_size=1,
        pipeline_model_parallel_size=1,
        context_parallel_size=1,
        data_parallel_size=1,
        seed=1234,
        mock_data=True,
        fp16=False,
        bf16=False,
        tokenizer_type="NullTokenizer",
        vocab_size=1024,
        padded_vocab_size=1024,
        untie_embeddings_and_output_weights=True,
        use_cpu_initialization=True,
        perform_initialization=True,
        iteration=0,
        consumed_train_samples=0,
        consumed_valid_samples=0,
        variable_seq_lengths=False,
        sft=False,
        fim_data=False,
        create_attention_mask_in_dataloader=False,
        dataloader_inter_document_masking=False,
        hybrid_context_parallel=False,
        sequence_parallel=False,
        overlap_moe_expert_parallel_comm=False,
        check_for_nan_in_loss_and_grad=False,
        check_for_spiky_loss=False,
        logits_load_dir=None,
        modelopt_enabled=False,
        rank=0,
        world_size=1,
        local_rank=0,
        distributed_backend="gloo",
        DDP_impl="local",
        # Fields referenced by Megatron-LM pretrain_gpt / gpt_builders.
        yaml_cfg=None,
        transformer_impl="local",
        experimental_attention_variant=None,
        num_experts=None,
        heterogeneous_layers_config_path=None,
        normalization="LayerNorm",
        qk_l2_norm=False,
        mtp_num_layers=None,
        spec=None,
        record_memory_history=False,
        memory_snapshot_path="oom_snapshot.pickle",
        use_legacy_models=False,
    )
    base.update(overrides)
    return SimpleNamespace(**base)


def set_args(args: SimpleNamespace) -> SimpleNamespace:
    global _ARGS
    _ARGS = args
    _STATE["iteration"] = int(getattr(args, "iteration", 0) or 0)
    _STATE["tp_size"] = max(1, int(getattr(args, "tensor_model_parallel_size", 1) or 1))
    _STATE["pp_size"] = max(
        1, int(getattr(args, "pipeline_model_parallel_size", 1) or 1)
    )
    _STATE["dp_size"] = max(1, int(getattr(args, "data_parallel_size", 1) or 1))
    # Ranks default to 0 for single-process macOS debug.
    _STATE["tp"] = int(getattr(args, "tensor_model_parallel_rank", 0) or 0)
    _STATE["pp"] = int(getattr(args, "pipeline_model_parallel_rank", 0) or 0)
    _STATE["dp"] = int(getattr(args, "data_parallel_rank", 0) or 0)
    gbs = int(getattr(args, "global_batch_size", 1) or 1)
    mbs = max(1, int(getattr(args, "micro_batch_size", 1) or 1))
    _STATE["micro_batches"] = max(1, gbs // mbs)
    return args


def get_args() -> SimpleNamespace:
    global _ARGS
    if _ARGS is None:
        _ARGS = default_args()
        set_args(_ARGS)
    _ARGS.iteration = int(_STATE["iteration"])
    return _ARGS


def reset_state(
    *,
    tp: int = 0,
    pp: int = 0,
    dp: int = 0,
    ep: int = 0,
    cp: int = 0,
    iteration: int = 0,
    micro_batches: int = 1,
) -> None:
    _STATE.update(
        {
            "initialized": False,
            "tp": int(tp),
            "pp": int(pp),
            "dp": int(dp),
            "ep": int(ep),
            "cp": int(cp),
            "iteration": int(iteration),
            "micro_batches": max(1, int(micro_batches)),
            "train_calls": 0,
        }
    )


def get_state() -> dict[str, Any]:
    return dict(_STATE)


def set_iteration(iteration: int) -> None:
    _STATE["iteration"] = int(iteration)
    if _ARGS is not None:
        _ARGS.iteration = int(iteration)
    try:
        from probing.fakes.journal import record_fake_event

        record_fake_event("set_iteration", f"iteration={iteration}")
    except Exception:
        pass


def set_micro_batches(n: int) -> None:
    _STATE["micro_batches"] = max(1, int(n))


def _mark(module: types.ModuleType) -> types.ModuleType:
    module.__probing_fake__ = True  # type: ignore[attr-defined]
    return module


def _make_parallel_state() -> types.ModuleType:
    ps = _mark(types.ModuleType("megatron.core.parallel_state"))

    def model_parallel_is_initialized() -> bool:
        return bool(_STATE["initialized"])

    def is_initialized() -> bool:
        return bool(_STATE["initialized"])

    def get_tensor_model_parallel_rank() -> int:
        return int(_STATE["tp"])

    def get_pipeline_model_parallel_rank() -> int:
        return int(_STATE["pp"])

    def get_data_parallel_rank() -> int:
        return int(_STATE["dp"])

    def get_tensor_model_parallel_world_size() -> int:
        return int(_STATE["tp_size"])

    def get_pipeline_model_parallel_world_size() -> int:
        return int(_STATE["pp_size"])

    def get_data_parallel_world_size() -> int:
        return int(_STATE["dp_size"])

    def is_pipeline_first_stage(*_a: Any, **_k: Any) -> bool:
        return int(_STATE["pp"]) == 0

    def is_pipeline_last_stage(*_a: Any, **_k: Any) -> bool:
        return int(_STATE["pp"]) == int(_STATE["pp_size"]) - 1

    def get_tensor_model_parallel_group() -> None:
        return None

    def get_tensor_model_parallel_src_rank() -> int:
        return 0

    def get_context_parallel_group() -> None:
        return None

    def get_hybrid_data_context_parallel_groups(*_a: Any, **_k: Any) -> None:
        return None

    def initialize_model_parallel(*args: Any, **kwargs: Any) -> None:
        if "tensor_model_parallel_size" in kwargs:
            _STATE["tp_size"] = max(1, int(kwargs["tensor_model_parallel_size"]))
        if "pipeline_model_parallel_size" in kwargs:
            _STATE["pp_size"] = max(1, int(kwargs["pipeline_model_parallel_size"]))
        del args
        _STATE["initialized"] = True
        try:
            from probing.fakes.journal import record_fake_event

            record_fake_event(
                "init_parallel",
                "initialize_model_parallel",
                attrs={
                    "tp_size": _STATE["tp_size"],
                    "pp_size": _STATE["pp_size"],
                    "tp": _STATE["tp"],
                    "pp": _STATE["pp"],
                    "dp": _STATE["dp"],
                },
            )
        except Exception:
            pass

    def destroy_model_parallel() -> None:
        _STATE["initialized"] = False

    ps.model_parallel_is_initialized = model_parallel_is_initialized
    ps.is_initialized = is_initialized
    ps.get_tensor_model_parallel_rank = get_tensor_model_parallel_rank
    ps.get_pipeline_model_parallel_rank = get_pipeline_model_parallel_rank
    ps.get_data_parallel_rank = get_data_parallel_rank
    ps.get_tensor_model_parallel_world_size = get_tensor_model_parallel_world_size
    ps.get_pipeline_model_parallel_world_size = get_pipeline_model_parallel_world_size
    ps.get_data_parallel_world_size = get_data_parallel_world_size
    ps.is_pipeline_first_stage = is_pipeline_first_stage
    ps.is_pipeline_last_stage = is_pipeline_last_stage
    ps.get_tensor_model_parallel_group = get_tensor_model_parallel_group
    ps.get_tensor_model_parallel_src_rank = get_tensor_model_parallel_src_rank
    ps.get_context_parallel_group = get_context_parallel_group
    ps.get_hybrid_data_context_parallel_groups = get_hybrid_data_context_parallel_groups
    if int(_STATE["ep"]):
        ps.get_expert_model_parallel_rank = lambda: int(_STATE["ep"])
    if int(_STATE["cp"]):
        ps.get_context_parallel_rank = lambda: int(_STATE["cp"])
    ps.initialize_model_parallel = initialize_model_parallel
    ps.destroy_model_parallel = destroy_model_parallel
    return ps


def _make_num_microbatches() -> types.ModuleType:
    mod = _mark(types.ModuleType("megatron.core.num_microbatches_calculator"))

    def get_num_microbatches() -> int:
        return int(_STATE["micro_batches"])

    mod.get_num_microbatches = get_num_microbatches
    return mod


def _make_enums() -> types.ModuleType:
    mod = _mark(types.ModuleType("megatron.core.enums"))

    class ModelType:
        encoder_or_decoder = 1
        encoder_and_decoder = 2
        retro_encoder = 3
        retro_decoder = 4

    mod.ModelType = ModelType
    return mod


def _make_gpt_model() -> types.ModuleType:
    import torch.nn as nn

    mod = _mark(types.ModuleType("megatron.core.models.gpt"))
    mod.__path__ = []  # type: ignore[attr-defined]  # package: gpt_layer_specs, …

    class GPTModel(nn.Module):
        """Tiny stand-in; lives on the remapped fake device (usually meta)."""

        def __init__(self, *args: Any, **kwargs: Any) -> None:
            super().__init__()
            del args
            hidden = int(kwargs.get("hidden_size", get_args().hidden_size))
            vocab = int(kwargs.get("vocab_size", get_args().padded_vocab_size))
            self.vp_stage = kwargs.get("vp_stage")
            self.embed = nn.Embedding(vocab, hidden)
            self.lm_head = nn.Linear(hidden, vocab, bias=False)

        def forward(
            self, tokens=None, position_ids=None, attention_mask=None, **kwargs
        ):
            del position_ids, attention_mask, kwargs
            if tokens is None:
                return self.lm_head(self.embed.weight[:1])
            hidden = self.embed(tokens)
            return self.lm_head(hidden)

        def build_schedule_plan(self, *args: Any, **kwargs: Any) -> None:
            del args, kwargs
            return None

    mod.GPTModel = GPTModel
    return mod


def _make_package_info() -> types.ModuleType:
    mod = _mark(types.ModuleType("megatron.core.package_info"))
    mod.__version__ = "0.0.0+probing.fakes"
    return mod


def _make_global_vars() -> types.ModuleType:
    mod = _mark(types.ModuleType("megatron.training.global_vars"))
    mod._probing_args = None  # type: ignore[attr-defined]
    mod.get_args = get_args
    return mod


def _make_training() -> types.ModuleType:
    mod = _mark(types.ModuleType("megatron.training.training"))

    def train_step(*args: Any, **kwargs: Any) -> dict[str, float]:
        del args, kwargs
        _STATE["train_calls"] = int(_STATE["train_calls"]) + 1
        try:
            from probing.fakes.journal import record_fake_event

            record_fake_event("train_step", "train_step")
        except Exception:
            pass
        return {"loss": 1.0}

    def update_seqlen_stats_from_cu_seqlens(*_a: Any, **_k: Any) -> None:
        return None

    mod.train_step = train_step
    mod.update_seqlen_stats_from_cu_seqlens = update_seqlen_stats_from_cu_seqlens
    return mod


def _make_training_pkg(
    *,
    training_mod: types.ModuleType,
    global_vars: types.ModuleType,
) -> types.ModuleType:
    pkg = _mark(types.ModuleType("megatron.training"))
    pkg.__path__ = []  # type: ignore[attr-defined]
    pkg.training = training_mod
    pkg.global_vars = global_vars
    pkg.get_args = get_args

    def print_rank_0(msg: Any) -> None:
        if int(getattr(get_args(), "rank", 0) or 0) == 0:
            print(msg)

    class _Timers:
        def __call__(self, *_a: Any, **_k: Any) -> _Timers:
            return self

        def start(self) -> None:
            return None

        def stop(self) -> None:
            return None

    def get_timers() -> _Timers:
        return _Timers()

    def set_startup_timestamps(**_k: Any) -> None:
        return None

    def pretrain(*args: Any, **kwargs: Any):
        from probing.fakes.pretrain import run_pretrain

        model_provider = kwargs.get("model_provider")
        forward_step = kwargs.get("forward_step_func") or kwargs.get("forward_step")
        datasets_provider = kwargs.get("train_valid_test_datasets_provider")

        # Megatron-LM current: (full_config, datasets_provider, model_type, forward_step, ...)
        # Older: (datasets_provider, model_provider, model_type, forward_step, ...)
        if (
            model_provider is None
            and datasets_provider is None
            and len(args) >= 4
            and callable(args[1])
            and callable(args[3])
            and not callable(args[0])
        ):
            datasets_provider = args[1]
            forward_step = forward_step or args[3]
        else:
            if datasets_provider is None and args and callable(args[0]):
                datasets_provider = args[0]
            if model_provider is None and len(args) >= 2 and callable(args[1]):
                model_provider = args[1]
            if forward_step is None and len(args) >= 4 and callable(args[3]):
                forward_step = args[3]

        return run_pretrain(
            model_provider=model_provider,
            forward_step_func=forward_step,
            train_valid_test_datasets_provider=datasets_provider,
        )

    class _InprocessRestart:
        @staticmethod
        def maybe_wrap_for_inprocess_restart(fn: Any):
            return fn, None

    pkg.print_rank_0 = print_rank_0
    pkg.get_timers = get_timers
    pkg.set_startup_timestamps = set_startup_timestamps
    pkg.pretrain = pretrain
    pkg.inprocess_restart = _InprocessRestart()
    return pkg


def build_megatron_tree(
    *,
    tp: int = 0,
    pp: int = 0,
    dp: int = 0,
    iteration: int = 0,
    micro_batches: int = 1,
    initialized: bool = True,
    args: SimpleNamespace | None = None,
) -> dict[str, types.ModuleType]:
    """Build a full megatron module dict (also usable outside the finder)."""
    reset_state(tp=tp, pp=pp, dp=dp, iteration=iteration, micro_batches=micro_batches)
    _STATE["initialized"] = initialized
    if args is not None:
        set_args(args)
    else:
        set_args(
            default_args(
                iteration=iteration,
                global_batch_size=micro_batches,
                micro_batch_size=1,
                tensor_model_parallel_rank=tp,
                pipeline_model_parallel_rank=pp,
                data_parallel_rank=dp,
            )
        )

    ps = _make_parallel_state()
    num_calc = _make_num_microbatches()
    enums = _make_enums()
    gpt_mod = _make_gpt_model()
    pkg_info = _make_package_info()
    training_mod = _make_training()
    global_vars = _make_global_vars()
    training_pkg = _make_training_pkg(
        training_mod=training_mod, global_vars=global_vars
    )

    models_pkg = _mark(types.ModuleType("megatron.core.models"))
    models_pkg.__path__ = []  # type: ignore[attr-defined]
    models_pkg.gpt = gpt_mod

    core = _mark(types.ModuleType("megatron.core"))
    core.__path__ = []  # type: ignore[attr-defined]
    core.parallel_state = ps
    core.mpu = ps  # historical alias
    core.num_microbatches_calculator = num_calc
    core.enums = enums
    core.models = models_pkg
    core.package_info = pkg_info

    megatron = _mark(types.ModuleType("megatron"))
    megatron.__path__ = []  # type: ignore[attr-defined]
    megatron.core = core
    megatron.training = training_pkg

    tree = {
        "megatron": megatron,
        "megatron.core": core,
        "megatron.core.mpu": ps,
        "megatron.core.parallel_state": ps,
        "megatron.core.num_microbatches_calculator": num_calc,
        "megatron.core.enums": enums,
        "megatron.core.models": models_pkg,
        "megatron.core.models.gpt": gpt_mod,
        "megatron.core.package_info": pkg_info,
        "megatron.training": training_pkg,
        "megatron.training.training": training_mod,
        "megatron.training.global_vars": global_vars,
    }
    _CACHE.clear()
    _CACHE.update(tree)
    _wire_training_cli_stubs(tree)
    _CACHE.update(tree)
    return tree


def ensure_sys_modules(
    *,
    tp: int | None = None,
    pp: int | None = None,
    dp: int | None = None,
    args: SimpleNamespace | None = None,
) -> dict[str, types.ModuleType]:
    """Materialize the fake tree into ``sys.modules``."""
    state = get_state()
    tree = build_megatron_tree(
        tp=state["tp"] if tp is None else tp,
        pp=state["pp"] if pp is None else pp,
        dp=state["dp"] if dp is None else dp,
        iteration=state["iteration"],
        micro_batches=state["micro_batches"],
        initialized=True,
        args=args or get_args(),
    )
    for name, mod in tree.items():
        sys.modules[name] = mod
    return tree


def _ensure_tree() -> dict[str, types.ModuleType]:
    if "megatron" not in _CACHE:
        build_megatron_tree()
    return _CACHE


class StubModule(types.ModuleType):
    """Package-like module that synthesizes missing attributes on demand.

    Lets real Megatron-LM scripts (``pretrain_gpt.py``, ``gpt_builders.py``)
    import deep ``megatron.*`` paths under ``probing.fakes`` force mode.
    """

    def __init__(self, name: str):
        super().__init__(name)
        self.__probing_fake__ = True  # type: ignore[attr-defined]
        self.__path__ = []  # type: ignore[attr-defined]

    def __getattr__(self, name: str) -> Any:
        if name.startswith("__") and name.endswith("__"):
            raise AttributeError(name)
        # CapWords → lightweight type (HybridModel, PackedSeqParams, …)
        if name[:1].isupper():
            cls = type(
                name,
                (),
                {
                    "__init__": lambda self, *a, **k: None,
                    "__enter__": lambda self: self,
                    "__exit__": lambda self, *a: None,
                    "__call__": lambda self, *a, **k: self,
                },
            )
            setattr(self, name, cls)
            return cls

        def stub(*args: Any, **kwargs: Any) -> Any:
            del args, kwargs
            return None

        stub.__name__ = name
        stub.__qualname__ = name
        setattr(self, name, stub)
        return stub


def _wire_training_cli_stubs(tree: dict[str, types.ModuleType]) -> None:
    """Concrete stubs for Megatron-LM argument / config import sites."""

    def _apply_argv_overrides(args: SimpleNamespace) -> SimpleNamespace:
        """Best-effort map of common Megatron-LM flags from ``sys.argv``."""
        mapping = {
            "--num-layers": ("num_layers", int),
            "--hidden-size": ("hidden_size", int),
            "--num-attention-heads": ("num_attention_heads", int),
            "--seq-length": ("seq_length", int),
            "--max-position-embeddings": ("max_position_embeddings", int),
            "--micro-batch-size": ("micro_batch_size", int),
            "--global-batch-size": ("global_batch_size", int),
            "--train-iters": ("train_iters", int),
            "--eval-iters": ("eval_iters", int),
            "--vocab-size": ("vocab_size", int),
            "--seed": ("seed", int),
            "--tensor-model-parallel-size": ("tensor_model_parallel_size", int),
            "--pipeline-model-parallel-size": ("pipeline_model_parallel_size", int),
        }
        argv = list(sys.argv[1:])
        i = 0
        while i < len(argv):
            tok = argv[i]
            if tok in mapping and i + 1 < len(argv):
                attr, caster = mapping[tok]
                try:
                    setattr(args, attr, caster(argv[i + 1]))
                except Exception:
                    pass
                if attr == "vocab_size":
                    args.padded_vocab_size = int(getattr(args, "vocab_size", 1024))
                i += 2
                continue
            if tok == "--mock-data":
                args.mock_data = True
            i += 1
        return set_args(args)

    def parse_and_validate_args(*args: Any, **kwargs: Any) -> SimpleNamespace:
        del args, kwargs
        return _apply_argv_overrides(default_args())

    def core_transformer_config_from_args(args: Any = None, **kwargs: Any) -> Any:
        del kwargs
        return args if args is not None else get_args()

    def core_transformer_config_from_yaml(*args: Any, **kwargs: Any) -> Any:
        del args, kwargs
        return get_args()

    def gpt_config_from_args(args: Any = None, **kwargs: Any) -> Any:
        del kwargs
        return args if args is not None else get_args()

    def pretrain_cfg_container_from_args(
        args: Any, model_cfg: Any = None, **kwargs: Any
    ) -> Any:
        del model_cfg, kwargs
        return args

    def import_module(*args: Any, **kwargs: Any) -> Any:
        del args, kwargs
        return None

    def get_torch_version() -> str:
        try:
            import torch

            return str(torch.__version__)
        except Exception:
            return "unknown"

    def get_te_version() -> str:
        return "none+fakes"

    class StragglerDetector:
        def __init__(self, *args: Any, **kwargs: Any) -> None:
            del args, kwargs

        def __call__(self, *args: Any, **kwargs: Any) -> StragglerDetector:
            del args, kwargs
            return self

        def __enter__(self) -> StragglerDetector:
            return self

        def __exit__(self, *args: Any) -> None:
            return None

    args_mod = StubModule("megatron.training.arguments")
    args_mod.parse_and_validate_args = parse_and_validate_args
    args_mod.core_transformer_config_from_args = core_transformer_config_from_args
    tree["megatron.training.arguments"] = args_mod

    arg_utils = StubModule("megatron.training.argument_utils")
    arg_utils.gpt_config_from_args = gpt_config_from_args
    arg_utils.pretrain_cfg_container_from_args = pretrain_cfg_container_from_args
    tree["megatron.training.argument_utils"] = arg_utils

    yaml_args = StubModule("megatron.training.yaml_arguments")
    yaml_args.core_transformer_config_from_yaml = core_transformer_config_from_yaml
    tree["megatron.training.yaml_arguments"] = yaml_args

    utils = StubModule("megatron.core.utils")
    utils.StragglerDetector = StragglerDetector
    utils.get_torch_version = get_torch_version
    utils.get_te_version = get_te_version
    utils.get_attr_wrapped_model = lambda model, name, *a, **k: getattr(
        model, name, None
    )
    utils.get_batch_on_this_tp_rank = lambda batch, **k: batch
    utils.get_batch_on_this_cp_rank = lambda batch, **k: batch
    utils.flatten_batch_for_packed_sequences = lambda batch, **k: batch
    tree["megatron.core.utils"] = utils

    spec_utils = StubModule("megatron.core.transformer.spec_utils")
    spec_utils.import_module = import_module
    tree["megatron.core.transformer.spec_utils"] = spec_utils

    # Attach under parents already in the tree when present.
    training = tree.get("megatron.training")
    if training is not None:
        training.arguments = args_mod
        training.argument_utils = arg_utils
        training.yaml_arguments = yaml_args
    core = tree.get("megatron.core")
    if core is not None:
        core.utils = utils


def megatron_factory(fullname: str) -> types.ModuleType:
    tree = _ensure_tree()
    if fullname in tree:
        return tree[fullname]
    mod = StubModule(fullname)
    tree[fullname] = mod
    parent_name, _, child = fullname.rpartition(".")
    parent = tree.get(parent_name) or sys.modules.get(parent_name)
    if parent is not None and child:
        try:
            setattr(parent, child, mod)
        except Exception:
            pass
    return mod


register(
    FakeSpec(
        name="megatron",
        prefixes=("megatron",),
        factory=megatron_factory,
        description="Scripted Megatron surface for pretrain_gpt-style macOS debug",
    )
)
