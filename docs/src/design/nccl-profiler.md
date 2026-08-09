# NCCL Profiler Architecture

The NCCL Profiler is not designed merely to persist callbacks. It must answer a distributed causal
question: when a collective slows down, is time spent waiting for this rank to produce data, for a
peer, for the network, or for device execution? A single timestamp cannot answer that question.
The architecture therefore preserves event lifetimes and lets the query layer align evidence
across ranks.

## Observation boundary: why the collector lives inside NCCL

The PyTorch API layer knows `global_step` and parallel roles, but the return of a synchronous call
or an asynchronous `work.wait()` is a host boundary, not proof that device and proxy work completed.
PyTorch Flight Recorder preserves watchdog ring records and is useful for post-timeout collective
alignment, but it does not continuously decompose waits in normal communication. Only the NCCL
profiler plugin observes Collective, KernelCh, ProxyOp, ProxyStep, and NetPlugin callbacks directly.

The evidence planes remain separate. Python supplies training coordinates, NCCL reconstructs
runtime execution and waits, and Flight Recorder preserves timeout state. They do not call one
another on callback paths; queries correlate them by communicator, sequence, rank, and epoch-ns
windows. This preserves each layer's time semantics and avoids pulling Python state into NCCL
communication threads merely to simplify a later join.

The plugin exports both `ncclProfiler_v4` and `ncclProfiler_v3`, allowing NCCL to negotiate the ABI.
V4 provides GPU globaltimer, per-communicator metadata, and fuller peer-wait evidence. Missing v3
signals degrade explicitly through `timing_source` and sentinel values rather than pretending to
have equal precision.

## Event lifetime: completion comes from child events

![NCCL child events reconstruct the execution window and decompose waits](../assets/architecture/probing-nccl-event-model.svg)

A collective `stopEvent` closes host enqueue while kernels and proxy work may still be running.
Publishing at that point would label launch time as execution time. The plugin therefore keeps the
Collective as a parent of active KernelCh and ProxyOp events; each ProxyOp, in turn, owns ProxyStep
progress. Only the final child close gives the parent a complete window and makes it publishable.

Timing degrades through an evidence hierarchy: GPU globaltimer first, then the KernelCh activity
window, then the ProxyOp envelope, and finally host enqueue. The selected source is stored in
`timing_source`. This is not presentation metadata; it is part of query semantics. Two
`exec_time_ns` values should be compared directly only when their evidence quality is comparable.

ProxyStep is not published as an unbounded detail table. Its transitions accumulate within a
ProxyOp into send-side GPU wait, peer-credit wait, network send, receive, and flush wait. This
trades bounded state for the decomposition needed by diagnosis and prevents message fragmentation
from multiplying storage volume. The waits remain evidence rather than conclusions: high
`send_gpu_wait_ns` implicates local production, while high `recv_wait_ns` implicates a peer or the
network. Culprit/victim attribution still requires the same sequence on other ranks, parallel
topology, and system state.

## Callback concurrency: communication threads never yield to diagnostics

![NCCL callbacks update sharded fixed pools and write completed rows outside the lock](../assets/architecture/probing-nccl-write-path.svg)

Callbacks arrive from host, proxy, NetPlugin, and watchdog threads. A global lock or dynamic growth
inside those callbacks could make the profiler alter communication timing. The plugin uses
fixed-capacity slot pools sharded by communicator hash. A callback normally touches one shard, and
capacity plus worst-case allocation cost are fixed at startup.

A handle contains shard, slot, and generation. Reuse changes the generation, so a late stop cannot
close a newer event that occupies the same slot. Under the shard lock the callback updates parent/
child state and counters and materializes a completed row. MEMT append happens only after releasing
the lock, preventing storage jitter from widening the NCCL critical section.

The watchdog uses `try_lock`. If a shard is busy, it skips and counts that snapshot instead of
waiting for the communication thread. This is an explicit priority decision: an observable data
gap is acceptable; creating a new hang while trying to diagnose one is not.

## Publication model: tables are projections of lifecycle state

Completed communication is published to `nccl.coll_perf` with its reconstructed window,
algorithm, protocol, message size, and `timing_source`. Proxy wait decomposition for the same work
goes to `nccl.proxy_ops`. An operation that never completes cannot produce either completed row, so
the watchdog writes read-only snapshots to `nccl.inflight_ops`. With NetPlugin enabled, QP
completion latency enters `nccl.net_qp` independently rather than being attached to a collective
whose relationship has not been proven.

These are not four competing answers; they are four projections of the event lifecycle. A query
starts with `coll_perf` to locate an anomalous window and uses `proxy_ops` to explain its waits. If
no completion exists it turns to `inflight_ops`; only network-wait evidence justifies joining
`net_qp` and RDMA metrics. Cross-rank queries use `global.nccl.*` to filter locally before merging,
while epoch-ns windows connect NCCL evidence to Python training-step coordinates. Aggregation and
causal inference belong to the query layer and never feed back into the collector.

`nccl.profiler_counters` defines the integrity boundary for all four projections. Pool exhaustion,
stale handles, write failures, and watchdog skips are counted. A diagnosis must inspect these
signals before interpreting an absence of events as an absence of anomalies.

## Failure boundary and implementation constraints

The callback path never waits for a remote node, calls another collector, or changes NCCL control
flow because diagnostics failed. A full pool drops and counts an event, MEMT failures are recorded
outside the shard lock, and watchdog contention skips a snapshot. The evidence can therefore have
an explicit gap while training communication retains its original control flow.

Exact schemas are in the [SQL table reference](../reference/sql-tables.md), capacity and runtime
controls in [Environment variables](../reference/env-vars.md), deployment and query examples in
[Performance analysis](../examples/performance-analysis.md), and cross-rank diagnostic orchestration
in [Diagnostic skills](../guide/skills.md). The implementation lives under
`probing/extensions/nccl-profiler/`.
