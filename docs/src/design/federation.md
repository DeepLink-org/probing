# Federated Query Engine

Product design for cross-rank SQL in probing: ask the whole training cluster from a
coordinator with one query — who is slow, on which step, compute vs network, which machine.

!!! note "Language"
    Full design (scenarios, SQL, execution paths, acceptance bar) is in
    **[中文版 / Chinese](/zh/design/federation/)**.

## Summary

| Path | When | Module |
|------|------|--------|
| Local `probe` | Single-process query | DataFusion |
| **Aggregate pushdown (A)** | Single-table `global.*` + `GROUP BY` / safe aggregates | `aggregate_pushdown.rs` |
| **Federated scan (B)** | Single-table scan, filters, raw rows | `FederatedScanExec` |
| **Broadcast (C)** | JOIN / CTE / multi-table on each rank | `cluster_fanout.rs` |

Federation tags (fixed): `_host`, `_addr`, `_rank`, `_node_rank`, `_local_rank`, `_role`.

Chinese doc covers: diagnostic scenarios (straggler, heatmap, slowdown, topology, hang),
engine behavior spec (routing, rewrite, tags, paths A/B/C), and the wan-scale «five SQL» bar.

## Related

- **[Federation (中文)](/zh/design/federation/)**
- [Distributed](distributed.md)
- [Modularity](modularity.md)
