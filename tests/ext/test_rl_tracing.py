import asyncio

import probing


def test_rl_context_normalizes_sample_and_trajectory_aliases():
    attrs = probing.rl.normalize_attrs(rollout_id=3, sample_id="s1")

    assert attrs["sample_id"] == "s1"
    assert attrs["trajectory_id"] == "s1"
    assert attrs["rollout_id"] == 3


def test_rl_span_merges_context_and_sets_phase():
    with probing.rl.context(run_id="run-a", rollout_id=1, sample_id="sample-a"):
        with probing.rl.span("inference", phase="inference.decode") as span:
            attrs = span.get_attributes()

    assert attrs["run_id"] == "run-a"
    assert attrs["rollout_id"] == 1
    assert attrs["sample_id"] == "sample-a"
    assert attrs["trajectory_id"] == "sample-a"
    assert attrs["phase"] == "inference.decode"


def test_rl_context_export_import_round_trip():
    with probing.rl.context(framework="slime", rollout_id=2, trajectory_id="traj-2"):
        carrier = probing.rl.export_context(turn_id=4)

    imported = probing.rl.import_context(carrier, env_step_id=1)

    assert imported["framework"] == "slime"
    assert imported["rollout_id"] == 2
    assert imported["sample_id"] == "traj-2"
    assert imported["trajectory_id"] == "traj-2"
    assert imported["turn_id"] == 4
    assert imported["env_step_id"] == 1


def test_rl_async_span_keeps_contextvars_context():
    async def run():
        with probing.rl.context(rollout_id=5, sample_id="async-sample"):
            async with probing.rl.async_span("env.step", phase="env.step") as span:
                await asyncio.sleep(0)
                return span.get_attributes()

    attrs = asyncio.run(run())

    assert attrs["rollout_id"] == 5
    assert attrs["sample_id"] == "async-sample"
    assert attrs["phase"] == "env.step"
