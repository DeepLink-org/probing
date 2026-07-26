"""Correlate ``python.fake_event`` ground truth with probing observations."""

from __future__ import annotations

import dataclasses
from dataclasses import dataclass
from typing import Any, Optional


@dataclass
class VerifyIssue:
    code: str
    message: str
    seq: int = -1
    expected: Any = None
    observed: Any = None


@dataclass
class VerifyReport:
    ok: bool
    checked: int
    issues: list[VerifyIssue]

    def raise_if_failed(self) -> None:
        if self.ok:
            return
        detail = "; ".join(f"{i.code}: {i.message}" for i in self.issues[:8])
        raise AssertionError(
            f"probing.fakes verify failed ({len(self.issues)} issues): {detail}"
        )


def _rows(table_cls, limit: int = 10_000) -> list[dict[str, Any]]:
    try:
        table_cls.init_table()
    except Exception:
        pass
    try:
        raw = table_cls.take(limit)
    except Exception:
        return []
    fields = [f.name for f in dataclasses.fields(table_cls)]
    out: list[dict[str, Any]] = []
    for item in raw:
        data = item[1] if isinstance(item, tuple) and len(item) == 2 else item
        if isinstance(data, dict):
            out.append(data)
        else:
            out.append(dict(zip(fields, data)))
    return out


def verify_against_probing(
    *,
    require_train_steps: Optional[int] = None,
    check_step_alignment: bool = True,
    check_collectives: bool = True,
    run_id: Optional[str] = None,
) -> VerifyReport:
    """Compare fake journal rows to probing step / optional ``comm_collective``.

    Checks:
    * ``train_step`` count (optional)
    * after probing Megatron wrap, ``local_step`` should equal ``expected_iteration``
    * fake ``collective`` rows should have matching ``python.comm_collective`` ops
      when that table has data

    ``run_id`` defaults to the current journal run (see ``begin_run``).
    """
    from probing.fakes.journal import FakeEvent, current_run_id

    issues: list[VerifyIssue] = []
    rid = current_run_id() if run_id is None else run_id
    events = [e for e in _rows(FakeEvent) if not rid or e.get("run_id") == rid]
    train_events = [e for e in events if e.get("kind") == "train_step"]
    checked = 0

    if require_train_steps is not None:
        checked += 1
        if len(train_events) != int(require_train_steps):
            issues.append(
                VerifyIssue(
                    code="train_step_count",
                    message="fake train_step count mismatch",
                    expected=int(require_train_steps),
                    observed=len(train_events),
                )
            )

    if check_step_alignment:
        for ev in train_events:
            checked += 1
            expected = int(ev.get("expected_iteration", -1))
            observed = int(ev.get("local_step", -2))
            # probing wrap syncs at train_step *entry*, so local_step == iteration.
            if expected >= 0 and observed != expected:
                issues.append(
                    VerifyIssue(
                        code="step_mismatch",
                        message="probing local_step != fake expected_iteration",
                        seq=int(ev.get("seq", -1)),
                        expected=expected,
                        observed=observed,
                    )
                )

    if check_collectives:
        coll_events = [e for e in events if e.get("kind") == "collective"]
        if coll_events:
            try:
                from probing.profiling.collective.record import CommCollective

                comm_rows = _rows(CommCollective)
            except Exception:
                comm_rows = []
            checked += 1
            if not comm_rows:
                issues.append(
                    VerifyIssue(
                        code="comm_missing",
                        message=(
                            "fake recorded collectives but python.comm_collective "
                            "is empty — enable collective tracing or use "
                            "fake dist which writes both"
                        ),
                        expected=len(coll_events),
                        observed=0,
                    )
                )
            else:
                fake_ops = [str(e.get("name", "")) for e in coll_events]
                comm_ops = [str(r.get("op", "")) for r in comm_rows]
                for op in fake_ops:
                    checked += 1
                    if op and op not in comm_ops:
                        issues.append(
                            VerifyIssue(
                                code="comm_op_missing",
                                message=f"no comm_collective row for fake op {op}",
                                expected=op,
                                observed=sorted(set(comm_ops)),
                            )
                        )

    return VerifyReport(ok=not issues, checked=checked, issues=issues)
