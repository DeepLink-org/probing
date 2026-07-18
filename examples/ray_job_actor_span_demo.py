"""Slime-like async RL demo for probing spans inside Ray actors.

This example mirrors the shape of ``slime/train_async.py`` without depending on
slime itself: the driver submits rollout generation one step ahead, waits for
the previous rollout, trains critic/actor workers, and periodically updates
weights, saves checkpoints, and runs evaluation.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import random
import time
from dataclasses import dataclass

import probing
import ray


@ray.remote
class RolloutManager:
    def __init__(self, actor_name: str, trajectories_per_rollout: int):
        from probing.ext.ray import register_current_process

        self.actor_name = actor_name
        self.trajectories_per_rollout = trajectories_per_rollout
        register_current_process("rollout_actor")
        with probing.span("rollout.manager.init", kind="ray.actor", actor_name=actor_name):
            time.sleep(0.05)

    def get_metrics_router_addr(self) -> str:
        from probing.ext.ray import register_current_process

        register_current_process("rollout_actor")
        with probing.span("metrics.router.addr", kind="slime.setup", actor_name=self.actor_name):
            time.sleep(0.01)
        return "http://127.0.0.1:18000"

    def check_weights(self, action: str) -> dict:
        from probing.ext.ray import register_current_process

        register_current_process("rollout_actor")
        with probing.span("weights.check.remote", kind="slime.weights", action=action):
            time.sleep(0.02)
        return {"action": action, "ok": True}

    async def _generate_trajectory(
        self,
        rollout_id: int,
        trajectory_idx: int,
        sleep_seconds: float,
        env_step_sleep_seconds: float,
        turns_per_trajectory: int,
    ) -> dict:
        import probing.rl as rl

        trajectory_id = f"r{rollout_id}-t{trajectory_idx}"
        carrier = rl.bind(
            framework="slime",
            algorithm="grpo",
            rollout_id=rollout_id,
            step_id=rollout_id,
            sample_id=trajectory_id,
            trajectory_id=trajectory_id,
            group_id=f"rollout-{rollout_id}",
            attempt=0,
            actor_role="rollout",
        )
        token_count = 0
        env_observations = []
        with rl.context(**carrier):
            async with rl.async_span(
                "trajectory.sample",
                phase="trajectory",
                kind="rl.trajectory",
                trajectory_index=trajectory_idx,
                turns=turns_per_trajectory,
            ):
                for turn_id in range(turns_per_trajectory):
                    with rl.context(turn_id=turn_id, env_step_id=turn_id):
                        async with rl.async_span(
                            "inference.generate",
                            phase="inference",
                            max_new_tokens=128,
                        ):
                            for value in range(20_000):
                                token_count += (value + rollout_id + trajectory_idx + turn_id) % 13
                            await asyncio.sleep(sleep_seconds / max(turns_per_trajectory, 1))

                        async with rl.async_span(
                            "env.step",
                            phase="env.step",
                            action=f"answer_turn_{turn_id}",
                        ):
                            await asyncio.sleep(env_step_sleep_seconds)
                            env_observations.append(f"obs-{turn_id}")

                        if turn_id + 1 < turns_per_trajectory:
                            async with rl.async_span(
                                "tool.call",
                                phase="tool.call",
                                tool_name="calculator",
                            ):
                                await asyncio.sleep(env_step_sleep_seconds * 0.5)

                async with rl.async_span("reward.compute", phase="reward"):
                    reward = round(random.random() + rollout_id * 0.01, 4)
                    await asyncio.sleep(0.01)

        return {
            "trajectory_id": trajectory_id,
            "tokens": token_count,
            "reward": reward,
            "turns": turns_per_trajectory,
            "env_observations": env_observations,
            "rl_trace": rl.export_context(**carrier),
        }

    async def generate(
        self,
        rollout_id: int,
        sleep_seconds: float,
        env_step_sleep_seconds: float,
        turns_per_trajectory: int,
    ) -> dict:
        from probing.ext.ray import current_process_identity, register_current_process

        register_current_process("rollout_actor")
        with probing.span(
            "custom.generate",
            kind="ray.actor",
            actor_name=self.actor_name,
            rollout_id=rollout_id,
            step_id=rollout_id,
        ):
            start = time.perf_counter()
            trajectories = await asyncio.gather(
                *[
                    self._generate_trajectory(
                        rollout_id,
                        trajectory_idx,
                        sleep_seconds / max(self.trajectories_per_rollout, 1),
                        env_step_sleep_seconds,
                        turns_per_trajectory,
                    )
                    for trajectory_idx in range(self.trajectories_per_rollout)
                ]
            )

        identity = current_process_identity("rollout_actor")
        return {
            "actor_name": self.actor_name,
            "rollout_id": rollout_id,
            "pid": os.getpid(),
            "identity": identity,
            "duration_s": time.perf_counter() - start,
            "trajectories": trajectories,
        }

    def save(self, rollout_id: int) -> dict:
        from probing.ext.ray import register_current_process

        register_current_process("rollout_actor")
        with probing.span(
            "rollout.dataset.save",
            kind="slime.io",
            rollout_id=rollout_id,
            step_id=rollout_id,
        ):
            time.sleep(0.02)
        return {"rollout_id": rollout_id, "saved": True, "pid": os.getpid()}

    def eval(self, rollout_id: int) -> dict:
        from probing.ext.ray import register_current_process

        register_current_process("rollout_actor")
        with probing.span(
            "rollout.eval.remote",
            kind="slime.eval",
            rollout_id=rollout_id,
            step_id=rollout_id,
        ):
            time.sleep(0.04)
        return {"rollout_id": rollout_id, "score": round(0.5 + random.random() * 0.1, 4)}

    def dispose(self) -> dict:
        from probing.ext.ray import register_current_process

        register_current_process("rollout_actor")
        with probing.span("rollout.dispose.remote", kind="slime.cleanup"):
            time.sleep(0.01)
        return {"disposed": True, "pid": os.getpid()}


@ray.remote
class TrainWorker:
    def __init__(self, model_name: str, sleep_seconds: float):
        from probing.ext.ray import register_current_process

        self.model_name = model_name
        self.sleep_seconds = sleep_seconds
        register_current_process(f"{model_name}_trainer")
        with probing.span(f"{model_name}.init", kind="ray.actor", model_name=model_name):
            time.sleep(0.03)

    def async_train(
        self,
        rollout_id: int,
        rollout_data: dict,
        external_data: dict | None = None,
    ) -> dict:
        from probing.ext.ray import current_process_identity, register_current_process

        register_current_process(f"{self.model_name}_trainer")
        batch_id = f"{self.model_name}-batch-{rollout_id}"
        with probing.span(
            f"{self.model_name}.train.remote",
            kind="slime.train",
            model_name=self.model_name,
            rollout_id=rollout_id,
            step_id=rollout_id,
            batch_id=batch_id,
        ):
            loss = 0.0
            trajectories = rollout_data["trajectories"]
            for item in trajectories:
                import probing.rl as rl

                carrier = rl.import_context(
                    item.get("rl_trace"),
                    actor_role=f"{self.model_name}_trainer",
                    batch_id=batch_id,
                )
                with rl.context(**carrier):
                    with rl.span(
                        "batch.prepare",
                        phase="train.prepare",
                        kind="rl.train",
                        model_name=self.model_name,
                    ):
                        loss += (item["tokens"] % 97) * 0.001
                    time.sleep(self.sleep_seconds / max(len(trajectories), 1))
                with rl.context(**carrier):
                    with rl.span(
                        "loss.compute",
                        phase="train.loss",
                        kind="rl.train",
                        model_name=self.model_name,
                    ):
                        loss += item["reward"] * 0.01
            if external_data:
                loss += float(external_data.get("loss", 0.0)) * 0.1
            with probing.span(
                "optimizer.step",
                kind="slime.train",
                model_name=self.model_name,
                rollout_id=rollout_id,
                step_id=rollout_id,
                batch_id=batch_id,
                phase="optimizer.step",
                actor_role=f"{self.model_name}_trainer",
            ):
                time.sleep(0.01)

        return {
            "model_name": self.model_name,
            "rollout_id": rollout_id,
            "pid": os.getpid(),
            "identity": current_process_identity(f"{self.model_name}_trainer"),
            "batch_id": batch_id,
            "loss": round(loss, 4),
        }

    def save_model(self, rollout_id: int, force_sync: bool) -> dict:
        from probing.ext.ray import register_current_process

        register_current_process(f"{self.model_name}_trainer")
        with probing.span(
            f"{self.model_name}.save.remote",
            kind="slime.io",
            model_name=self.model_name,
            rollout_id=rollout_id,
            step_id=rollout_id,
            force_sync=force_sync,
        ):
            time.sleep(0.02)
        return {"model_name": self.model_name, "rollout_id": rollout_id, "saved": True}

    def update_weights(self, rollout_id: int) -> dict:
        from probing.ext.ray import register_current_process

        register_current_process(f"{self.model_name}_trainer")
        with probing.span(
            f"{self.model_name}.weights.export",
            kind="slime.weights",
            model_name=self.model_name,
            rollout_id=rollout_id,
            step_id=rollout_id,
        ):
            time.sleep(0.03)
        return {"model_name": self.model_name, "rollout_id": rollout_id, "updated": True}


@dataclass
class StepResult:
    rollout_id: int
    rollout_pid: int
    actor_pid: int | None = None
    critic_pid: int | None = None
    eval_score: float | None = None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--address", default="auto", help="Ray address for ray.init")
    parser.add_argument("--num-rollouts", type=int, default=4)
    parser.add_argument("--start-rollout-id", type=int, default=0)
    parser.add_argument("--trajectories-per-rollout", type=int, default=3)
    parser.add_argument("--turns-per-trajectory", type=int, default=3)
    parser.add_argument("--generate-sleep-seconds", type=float, default=0.1)
    parser.add_argument("--env-step-sleep-seconds", type=float, default=0.02)
    parser.add_argument("--train-sleep-seconds", type=float, default=0.05)
    parser.add_argument("--use-critic", action="store_true", default=True)
    parser.add_argument("--no-critic", action="store_false", dest="use_critic")
    parser.add_argument("--num-critic-only-steps", type=int, default=1)
    parser.add_argument("--update-weights-interval", type=int, default=2)
    parser.add_argument("--save-interval", type=int, default=2)
    parser.add_argument("--eval-interval", type=int, default=3)
    parser.add_argument("--check-weight-update-equal", action="store_true")
    parser.add_argument(
        "--keep-alive-seconds",
        type=int,
        default=600,
        help="Keep the job alive so probing CLI can query driver and actor PIDs.",
    )
    return parser.parse_args()


def should_run_periodic_action(rollout_id: int, interval: int, final_rollout_id: int) -> bool:
    if interval <= 0:
        return False
    return (rollout_id + 1) % interval == 0 or rollout_id == final_rollout_id


def main() -> None:
    args = parse_args()

    ray.init(
        address=args.address,
        _tracing_startup_hook="probing.ext.ray:setup_tracing",
    )

    from probing.ext.ray import get_ray_timeline, setup_driver

    setup_driver("driver")
    driver_pid = os.getpid()
    final_rollout_id = args.start_rollout_id + args.num_rollouts - 1

    with probing.span("setup", kind="slime.setup"):
        rollout_manager = RolloutManager.remote(
            "rollout-manager",
            args.trajectories_per_rollout,
        )
        router_addr = ray.get(rollout_manager.get_metrics_router_addr.remote())
        actor_model = TrainWorker.remote("actor", args.train_sleep_seconds)
        critic_model = TrainWorker.remote("critic", args.train_sleep_seconds * 0.8)
        with probing.span("weights.update.initial", kind="slime.weights"):
            ray.get(actor_model.update_weights.remote(args.start_rollout_id))

    if args.check_weight_update_equal:
        with probing.span("weights.check", kind="slime.weights"):
            ray.get(rollout_manager.check_weights.remote(action="compare"))

    step_results: list[StepResult] = []
    rollout_data_next_future = None
    rollout_data_prefetched = None
    with probing.span(
        "rollout.submit",
        kind="rollout",
        rollout_id=args.start_rollout_id,
        step_id=args.start_rollout_id,
        submit_step_id=args.start_rollout_id - 1,
        consume_step_id=args.start_rollout_id,
    ):
        rollout_data_next_future = rollout_manager.generate.remote(
            args.start_rollout_id,
            args.generate_sleep_seconds,
            args.env_step_sleep_seconds,
            args.turns_per_trajectory,
        )

    for rollout_id in range(args.start_rollout_id, args.start_rollout_id + args.num_rollouts):
        with probing.span(
            "rollout.step",
            kind="step",
            rollout_id=rollout_id,
            step_id=rollout_id,
        ):
            probing.event("rollout.step.start", attributes=[{"rollout_id": rollout_id, "step_id": rollout_id}])
            with probing.span(
                "rollout.wait",
                kind="slime.rollout",
                rollout_id=rollout_id,
                step_id=rollout_id,
                submit_step_id=rollout_id - 1,
                consume_step_id=rollout_id,
            ):
                if rollout_data_next_future is not None:
                    rollout_data_curr = ray.get(rollout_data_next_future)
                else:
                    rollout_data_curr = rollout_data_prefetched
                    rollout_data_prefetched = None
                if rollout_data_curr is None:
                    raise RuntimeError(f"No rollout data available for rollout_id={rollout_id}")
                step_result = StepResult(
                    rollout_id=rollout_id,
                    rollout_pid=int(rollout_data_curr["pid"]),
                )

            if rollout_id < final_rollout_id:
                with probing.span(
                    "rollout.submit_next",
                    kind="slime.rollout",
                    rollout_id=rollout_id + 1,
                    step_id=rollout_id,
                    submit_step_id=rollout_id,
                    consume_step_id=rollout_id + 1,
                ):
                    rollout_data_next_future = rollout_manager.generate.remote(
                        rollout_id + 1,
                        args.generate_sleep_seconds,
                        args.env_step_sleep_seconds,
                        args.turns_per_trajectory,
                    )

            if args.use_critic:
                actor_trains_this_step = rollout_id >= args.num_critic_only_steps
                with probing.span(
                    "critic.train",
                    kind="slime.train",
                    rollout_id=rollout_id,
                    step_id=rollout_id,
                ):
                    value_ref = critic_model.async_train.remote(rollout_id, rollout_data_curr)
                if actor_trains_this_step:
                    with probing.span(
                        "actor.train",
                        kind="slime.train",
                        rollout_id=rollout_id,
                        step_id=rollout_id,
                    ):
                        critic_result = ray.get(value_ref)
                        step_result.critic_pid = int(critic_result["pid"])
                        actor_result = ray.get(
                            actor_model.async_train.remote(
                                rollout_id,
                                rollout_data_curr,
                                critic_result,
                            )
                        )
                        step_result.actor_pid = int(actor_result["pid"])
                else:
                    with probing.span(
                        "critic.wait",
                        kind="slime.train",
                        rollout_id=rollout_id,
                        step_id=rollout_id,
                    ):
                        critic_result = ray.get(value_ref)
                        step_result.critic_pid = int(critic_result["pid"])
            else:
                with probing.span(
                    "actor.train",
                    kind="slime.train",
                    rollout_id=rollout_id,
                    step_id=rollout_id,
                ):
                    actor_result = ray.get(
                        actor_model.async_train.remote(rollout_id, rollout_data_curr)
                    )
                    step_result.actor_pid = int(actor_result["pid"])

            if should_run_periodic_action(rollout_id, args.save_interval, final_rollout_id):
                with probing.span(
                    "checkpoint.save",
                    kind="slime.io",
                    rollout_id=rollout_id,
                    step_id=rollout_id,
                ):
                    save_refs = [
                        actor_model.save_model.remote(
                            rollout_id,
                            rollout_id == final_rollout_id,
                        )
                    ]
                    if args.use_critic:
                        save_refs.append(
                            critic_model.save_model.remote(
                                rollout_id,
                                rollout_id == final_rollout_id,
                            )
                        )
                    save_refs.append(rollout_manager.save.remote(rollout_id))
                    ray.get(save_refs)

            if (rollout_id + 1) % args.update_weights_interval == 0:
                with probing.span(
                    "weights.update",
                    kind="slime.weights",
                    rollout_id=rollout_id,
                    step_id=rollout_id,
                ):
                    if rollout_data_next_future is not None:
                        rollout_data_prefetched = ray.get(rollout_data_next_future)
                        rollout_data_next_future = None
                    ray.get(actor_model.update_weights.remote(rollout_id))

            if should_run_periodic_action(rollout_id, args.eval_interval, final_rollout_id):
                with probing.span(
                    "eval",
                    kind="slime.eval",
                    rollout_id=rollout_id,
                    step_id=rollout_id,
                ):
                    eval_result = ray.get(rollout_manager.eval.remote(rollout_id))
                    step_result.eval_score = float(eval_result["score"])

            step_results.append(step_result)
            probing.event("rollout.step.end", attributes=[{"rollout_id": rollout_id, "step_id": rollout_id}])

    with probing.span("rollout.dispose", kind="slime.cleanup"):
        ray.get(rollout_manager.dispose.remote())
    with probing.span("tracking.finish", kind="slime.cleanup", router_addr=router_addr):
        time.sleep(0.01)

    timeline = get_ray_timeline()
    actor_pids = sorted(
        {
            pid
            for result in step_results
            for pid in (result.rollout_pid, result.actor_pid, result.critic_pid)
            if pid is not None
        }
    )

    print("=== probing slime-like async RL span demo ===")
    print(f"driver_pid={driver_pid}")
    print(f"actor_pids={','.join(str(pid) for pid in actor_pids)}")
    print(f"ray_timeline_entries={len(timeline)}")
    print("step_results:")
    print(json.dumps([result.__dict__ for result in step_results], indent=2, sort_keys=True))
    print()
    print("Useful commands:")
    print("  probing list")
    print("  open the Traces page and select 'RL Timeline' to inspect rollout samples")
    print(
        "  probing -t "
        f'{driver_pid} query "SELECT actor_id, class_name, worker_id, worker_pid '
        "FROM python.ray_actor ORDER BY timestamp_ns DESC LIMIT 20;\""
    )
    print(
        "  probing -t "
        f'{driver_pid} query "SELECT name, attributes FROM python.trace_event '
        "WHERE record_type = 'span_start' ORDER BY time DESC LIMIT 20;\""
    )
    for pid in actor_pids:
        print(
            "  probing -t "
            f'{pid} query "SELECT name, attributes FROM python.trace_event '
            "WHERE record_type = 'span_start' ORDER BY time DESC LIMIT 20;\""
        )

    if args.keep_alive_seconds > 0:
        print(f"keeping job alive for {args.keep_alive_seconds}s...")
        time.sleep(args.keep_alive_seconds)


if __name__ == "__main__":
    main()
