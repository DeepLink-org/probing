# RL Dashboard

Probing exposes framework-neutral RL telemetry through relational tables and the
`/rl/overview` Web page. Aggregate trainer trends use `rl.metric`; per-sample
execution paths continue to use `python.trace_event`.

## XTuner

Start the training process with Probing enabled, then enable the XTuner adapter:

```bash
PROBING=1 <xtuner training command>
probing -t <pid> config python.enabled=probing.ext.xtuner
```

### What each XTuner version can report

The adapter hooks whichever of XTuner's methods the installed version has, and
what reaches the dashboard follows from that. Check it with:

```python
import probing.ext.xtuner as adapter
adapter.install()
adapter.status()   # {'enabled': ..., 'installed_hooks': [...], 'error': ...}
```

An empty `installed_hooks` with a non-empty `error` means the installed XTuner
exposes none of the hook points, and no RL telemetry will appear. This is worth
checking first when the Overview page stays empty, since every collection path is
fail-open and an API mismatch otherwise looks the same as an idle trainer.

Releases that expose `_log_step` and an experiment tracker report the full metric
vocabulary below. Releases built around `RLTrainer._save_trajectories` alone —
XTuner 0.2.0 among them — report what trajectories carry: `rl.sample` rows with
reward and task, the pass-rate gauges derived from them (`sampler.avg_pass`,
`sampler.pass_zero_ratio`, `sampler.pass_one_ratio`), token counts, and the
environment gauges. Trainer internals such as `policy.entropy`,
`policy.pg_loss`, `policy.grad_norm`, the wall-clock breakdown, and throughput
live inside the training workers there and are not published to the trainer, so
those charts stay empty. The pass-rate distribution, data-source table, and batch
composition all derive from `rl.sample` and work either way.

The adapter hooks XTuner's existing experiment writer. It records every numeric
source scalar and also maps stable fields to canonical names such as
`reward.mean`, `policy.entropy`, `policy.train_infer_kl`, and `time.step_s`.
Coverage spans the reward and advantage distribution, ratio clipping
(`policy.clip_frac_high`), sampler-versus-trainer divergence
(`policy.train_infer_ppl_ratio`), the step wall-clock breakdown
(`time.onload_s`, `time.save_ckpt_s`), rollout straggler spread
(`sampler.task_p99_p50_ratio`), and accelerator memory
(`hardware.max_memory_gb`). `probing.ext.rl_data.protocol.CANONICAL_METRICS` is
the full vocabulary with a one-line meaning for each name.

When the trainer knows its planned step count, the adapter also emits
`progress.total_steps`, `progress.completed_ratio`, and `progress.eta_s`, which
the Overview header renders as `Step 42/1000 (4.2%) · ETA 1h 12m`.

Trajectory saves additionally populate `rl.sample` and derive grouped pass-rate,
token-count, staleness, and environment gauges (`environment.active`,
`environment.sandbox_total`). Offline `eval/*` scalars become `rl.benchmark`
rows; online pass rate is also published as `online/avg_pass`.

Set `PROBING_RL_RUN_ID` before launch when the experiment directory should not
be used as the run identifier.

The adapter is fail-open: table allocation, conversion, or write failures do not
propagate into the trainer.

### Trying it on a small job

`examples/rl/xtuner_gsm8k_smoke_config.py` is a two-step GRPO GSM8K config that
reads `WORK_DIR`, `MODEL_PATH`, `DATA_PATH`, and `EVAL_DATA_PATH` from the
environment, so it carries no paths of its own. Launch it through
`examples/rl/xtuner_smoke_entry.py`, which installs the adapter and then hands the
remaining arguments to XTuner's own `train.cli.rl`:

```bash
WORK_DIR=/tmp/xtuner-smoke MODEL_PATH=... DATA_PATH=... EVAL_DATA_PATH=... \
  PROBING=1 python examples/rl/xtuner_smoke_entry.py \
  --config examples/rl/xtuner_gsm8k_smoke_config.py --num-workers 4
```

Installing from the entry wrapper reaches the driver process, which is where the
trainer and every hook point live. It does not reach rollout workers: those are
Ray actors, and Ray starts them as fresh processes that re-import their modules,
so nothing patched in the driver's memory carries over. Enable the adapter inside
a rollout worker, if ever needed, through Ray's `runtime_env` rather than from
here.

To see the pages without a GPU, `examples/rl/dashboard_demo.py` writes the same
tables from synthetic telemetry.

### What the LMDeploy rollout path needs

The GRPO example drives rollout through LMDeploy, and two things there are
independent of Probing but decide whether the job starts at all.

XTuner imports `SpeculativeConfig` from `lmdeploy.messages`, which only exists in
builds that carry speculative decoding. On an older LMDeploy the rollout worker
fails at server-config time; upgrade LMDeploy, or guard that import inside
XTuner. Patching it from a launcher does not work, for the process reason above.

Weight sync can also fail inside `LMDeployIPCBackendAdapter`: when the payload
reuses a cached IPC tensor it carries metadata without a `flattened_tensor`
entry, and LMDeploy's `serialize_state_dict` tries to `reduce_tensor()` those
non-tensor fields. Both were patched locally in XTuner to get the smoke job
running, so the telemetry reported here was collected against a patched XTuner
tree rather than a stock one.

## Tables

- `rl.run`: run identity and latest phase/step snapshot
- `rl.metric`: numeric trainer series in long-table form
- `rl.sampler`: accepted/trained/filtered/failed pipeline snapshots
- `rl.sample`: rollout outcome, reward, token counts, category, and staleness
- `rl.benchmark`: versioned evaluation results with harness and aggregation
- `rl.notice`: operator-authored notes explaining interventions

Examples:

```sql
SELECT step, value
FROM rl.metric
WHERE run_id = '<run>' AND name = 'reward.mean'
ORDER BY step;

SELECT status, count(*), avg(reward)
FROM rl.sample
WHERE run_id = '<run>'
GROUP BY status;
```

## HTTP API

- `GET /apis/rl/runs`
- `GET /apis/rl/status?run_id=...`
- `GET /apis/rl/tags?run_id=...`
- `GET /apis/rl/series?run_id=...&names=reward.mean,time.step_s&limit=2000`
- `GET /apis/rl/series?run_id=...&names=reward.mean,time.step_s&buckets=192`
- `GET /apis/rl/samples?run_id=...&limit=500`
- `GET /apis/rl/sampler?run_id=...&limit=40`
- `GET /apis/rl/composition?run_id=...&dimension=task|category|status|filter_reason&limit=2000`
- `GET /apis/rl/datasets?run_id=...&limit=2000`
- `GET /apis/rl/pass_histogram?run_id=...&limit=2000`
- `GET /apis/rl/staleness?run_id=...&limit=2000`
- `GET /apis/rl/benchmarks?run_id=...&limit=2000`
- `GET /apis/rl/events?run_id=...&limit=40`
- `GET /apis/rl/about?run_id=...`

The series endpoint takes either a row budget or a bucket count. `limit` shares
one budget across every name requested, so asking for eighteen metrics over a
long run returns only the most recent steps of each and quietly drops the rest
of the history. `buckets` instead summarises each metric into that many points
server-side, carrying each bucket's mean, its `low`/`high`, its `spread`
(standard deviation), and the bucket width in `bucket_steps`. Response size then
no longer grows with the run, so a chart covers the whole run however long it
gets. The overview page uses the bucketed form for its trend charts.

Draw `spread` around the line and report `low`/`high` as text. A bucket's
extremes widen as buckets get coarser — a noisy metric bucketed 150 steps at a
time has almost certainly touched both ends of its range within every bucket —
so a min/max band grows until it covers the whole plot and hides the line it was
meant to annotate. The standard deviation measures how much the metric actually
moves, which does not inflate with bucket width.

The composition endpoint returns both an aggregate breakdown and a per-step
one, so a dashboard can stack the batch mix over time and show the change in
share against the previous step. Adapters set `rl.sample.category` to group data
sources; when they do not, the writer falls back to the task's namespace.

The datasets endpoint pivots the same samples the other way, one row per data
source with its completed, filtered, failed, and in-flight counts plus the pass
rate over the samples a judge actually scored. Sources are ordered by volume.
The pass rate is absent rather than zero when nothing has been judged yet, so a
dataset waiting on its judge is distinguishable from one failing every check.

The pass-histogram endpoint groups rollouts by prompt and step, then spreads the
prompts across nine buckets by pass rate. The two ends are exact rather than
rounded, so bucket 0 and bucket 8 agree with the `sampler.pass_zero_ratio` and
`sampler.pass_one_ratio` gauges; the seven buckets between them split the open
interval evenly. Prompts at either end contribute no gradient signal and are
what dynamic sampling discards, so the shape matters as much as the mean: a run
where every prompt is middling and one where half are trivial and half
impossible report the same average pass rate.

Canonical overview gauges also include spend and reliability fields such as
`cost.usd_total`, `cost.usd_per_hour`, and `hardware.restart_count`. XTuner can
inject them through `PROBING_RL_COST_USD`, `PROBING_RL_COST_USD_PER_HOUR`,
`PROBING_RL_RESTARTS`, and `PROBING_RL_SANDBOX_TOTAL`.

The series endpoint accepts at most 96 metric names and 20,000 rows per request.
The Web page polls these bounded endpoints every five seconds.

Overview presents up to two runs side-by-side (primary + compare) with spend,
pass-rate delta, pinned evaluation, and a live notice feed from
`/apis/rl/events` (operator notices, restarts, sampler failures, and recently
published benchmarks).

## Operator notices

Synthesized events report that a run restarted; they cannot report *why*. File a
note so the reason survives into the post-mortem:

```python
from probing.ext.rl_data import notice

notice(
    "Restarted after node drain; steps 120-128 replayed.",
    run_id="<run>",
    step=128,
    level="warning",
    kind="restart",
    author="oncall",
)
```

Notices appear in `/apis/rl/events` and on the Overview page alongside
synthesized events. An empty `run_id` makes the note apply to every run, which
suits cluster-wide incidents. The table is created on first write, so
`/apis/rl/events` works unchanged before any notice exists.

## Adding another framework

Implement the `RlFrameworkAdapter` protocol from
`probing.ext.rl_data.protocol`, map framework fields into the same tables, and
keep framework imports inside the adapter module. The API and Web UI must not
depend on XTuner or Slime class names.
