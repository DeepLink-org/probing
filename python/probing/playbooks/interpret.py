"""Evaluate playbook ``interpretation.rules`` against step evidence.

Shared semantics with the Rust interpreter in ``probing-cli`` and ``web`` agent.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any, Dict, List, Mapping, Optional, Sequence

_NUM_RE = re.compile(r"^-?\d+(\.\d+)?$")


@dataclass
class StepEvidence:
    step_id: str
    row_count: int
    columns: Dict[str, List[Any]] = field(default_factory=dict)


@dataclass
class InterpretFinding:
    rule_id: str
    severity: str
    message: str


def _expand_params(template: str, params: Mapping[str, Any]) -> str:
    out = template
    for key, val in params.items():
        out = out.replace(f"{{{key}}}", str(val))
    return out


def _col_values(ev: StepEvidence, name: str) -> List[Any]:
    return list(ev.columns.get(name, []))


def _as_floats(vals: Sequence[Any]) -> List[float]:
    out: List[float] = []
    for v in vals:
        if isinstance(v, (int, float)):
            out.append(float(v))
        elif isinstance(v, str) and _NUM_RE.match(v.strip()):
            out.append(float(v.strip()))
    return out


def _avg(vals: Sequence[float]) -> float:
    return sum(vals) / len(vals) if vals else 0.0


def _median(vals: Sequence[float]) -> float:
    if not vals:
        return 0.0
    s = sorted(vals)
    mid = len(s) // 2
    if len(s) % 2 == 0:
        return (s[mid - 1] + s[mid]) / 2.0
    return s[mid]


def _max_min_ratio(vals: Sequence[float]) -> float:
    if len(vals) < 2:
        return 0.0
    hi = max(vals)
    lo = min(vals)
    if lo <= 0:
        return float("inf")
    return hi / lo


def _step_by_id(steps: Sequence[StepEvidence], step_id: str) -> Optional[StepEvidence]:
    for s in steps:
        if s.step_id == step_id:
            return s
    return None


def _eval_rows(pred: str, row_count: int, params: Mapping[str, Any]) -> bool:
    pred = pred.strip()
    for op in ("==", ">=", "<=", ">", "<"):
        if pred.startswith(op):
            rhs = pred[len(op) :].strip()
            threshold = _eval_numeric(rhs, params)
            if op == "==":
                return row_count == int(threshold)
            if op == ">=":
                return row_count >= int(threshold)
            if op == "<=":
                return row_count <= int(threshold)
            if op == ">":
                return row_count > int(threshold)
            if op == "<":
                return row_count < int(threshold)
    return False


def _eval_numeric(expr: str, params: Mapping[str, Any]) -> float:
    expr = _expand_params(expr, params)
    if "*" in expr:
        lhs, rhs = expr.split("*", 1)
        return float(lhs.strip()) * float(rhs.strip())
    return float(expr.strip())


def _eval_column(col_name: str, tail: str, ev: StepEvidence) -> bool:
    nums = _as_floats(_col_values(ev, col_name))
    texts = [str(v) for v in _col_values(ev, col_name)]
    tail = tail.strip()

    if "max/min(ratio)" in tail and ">" in tail:
        threshold = float(tail.split(">", 1)[1].strip())
        return _max_min_ratio(nums) > threshold
    if tail.startswith("max >"):
        return (max(nums) if nums else 0.0) > float(tail[5:].strip())
    if tail.startswith("avg >"):
        return _avg(nums) > float(tail[5:].strip())
    if tail.startswith("top >"):
        return (max(nums) if nums else 0.0) > float(tail[5:].strip())
    if tail.startswith("value >"):
        return (nums[0] if nums else 0.0) > float(tail[7:].strip())
    if tail.startswith("last >") and "* avg(" in tail:
        rest = tail[6:].strip()
        factor_s, col_part = rest.split("* avg(", 1)
        col = col_part.rstrip(")")
        col_vals = _as_floats(_col_values(ev, col))
        last = col_vals[-1] if col_vals else 0.0
        return last > float(factor_s.strip()) * _avg(col_vals)
    if tail.startswith("any_contains("):
        inner = tail[len("any_contains(") : -1]
        needles = [
            p.strip().strip("'\"").lower() for p in inner.split(",") if p.strip()
        ]
        return any(any(n in t.lower() for n in needles) for t in texts if t)
    return False


def _eval_top_vs_median(clause: str, steps: Sequence[StepEvidence]) -> bool:
    parts = [p.strip() for p in clause.split("|") if p.strip()]
    step_id = "rank_latency"
    if parts and parts[0].startswith("step:"):
        step_id = parts[0][5:]
    ev = _step_by_id(steps, step_id)
    if ev is None or ev.row_count < 2:
        return False
    vals = _as_floats(_col_values(ev, "avg_ms"))
    if not vals:
        return False
    return max(vals) > 2.0 * _median(vals)


def _eval_clause(
    clause: str,
    step: Optional[StepEvidence],
    steps: Sequence[StepEvidence],
    params: Mapping[str, Any],
) -> bool:
    clause = clause.strip()
    if clause == "always":
        return True
    if clause.startswith("rows "):
        if step is None:
            return False
        return _eval_rows(clause[5:], step.row_count, params)
    if clause.startswith("column:"):
        col_name = clause.split("|", 1)[0][7:].strip()
        tail = clause.split("|", 1)[1].strip() if "|" in clause else ""
        if step is None or not tail:
            return False
        return _eval_column(col_name, tail, step)
    if "top(row)" in clause:
        return _eval_top_vs_median(clause, steps)
    return False


def rule_matches(
    when: str,
    steps: Sequence[StepEvidence],
    params: Optional[Mapping[str, Any]] = None,
) -> bool:
    params = params or {}
    when = _expand_params(when, params)
    parts = [p.strip() for p in when.split("|") if p.strip()]
    if not parts:
        return False
    idx = 0
    step: Optional[StepEvidence] = None
    if parts[0].startswith("step:"):
        step = _step_by_id(steps, parts[0][5:])
        if step is None:
            return False
        idx = 1
    i = idx
    while i < len(parts):
        part = parts[i]
        if part.startswith("column:"):
            if step is None:
                return False
            col_name = part[7:].strip()
            tail = parts[i + 1] if i + 1 < len(parts) else ""
            if not _eval_column(col_name, tail, step):
                return False
            i += 2
            continue
        if not _eval_clause(part, step, steps, params):
            return False
        i += 1
    return True


def _expand_message(
    template: str,
    steps: Sequence[StepEvidence],
    params: Mapping[str, Any],
) -> str:
    msg = _expand_params(template, params)
    ev = _step_by_id(steps, "rank_latency")
    if ev and "{worst_rank}" in msg:
        ranks = _col_values(ev, "rank")
        avgs = _as_floats(_col_values(ev, "avg_ms"))
        if ranks and avgs:
            worst = ranks[max(range(len(avgs)), key=lambda i: avgs[i])]
            msg = msg.replace("{worst_rank}", str(worst))
    ev = _step_by_id(steps, "module_totals")
    if ev and "{top_module}" in msg:
        modules = _col_values(ev, "module")
        pcts = _as_floats(_col_values(ev, "pct_time"))
        if modules and pcts:
            top = modules[max(range(len(pcts)), key=lambda i: pcts[i])]
            msg = msg.replace("{top_module}", str(top))
    ev = _step_by_id(steps, "latest_torch_step")
    if ev and "{latest_step}" in msg:
        vals = _as_floats(_col_values(ev, "latest_step"))
        if vals:
            msg = msg.replace("{latest_step}", str(int(vals[0])))
    return msg


def evaluate_rules(
    rules: Sequence[Mapping[str, Any]],
    steps: Sequence[StepEvidence],
    params: Optional[Mapping[str, Any]] = None,
) -> List[InterpretFinding]:
    """Return findings for rules whose ``when`` clause matches."""
    params = params or {}
    out: List[InterpretFinding] = []
    for rule in rules:
        when = str(rule.get("when", ""))
        if not when:
            continue
        if rule_matches(when, steps, params):
            out.append(
                InterpretFinding(
                    rule_id=str(rule.get("id", "")),
                    severity=str(rule.get("severity", "info")),
                    message=_expand_message(
                        str(rule.get("message", "")), steps, params
                    ),
                )
            )
    return out


def evidence_from_dataframe(step_id: str, df: Mapping[str, Any]) -> StepEvidence:
    """Build evidence from a tabular dict (column -> list of values)."""
    columns = {str(k): list(v) for k, v in df.items()}
    row_count = max((len(v) for v in columns.values()), default=0)
    return StepEvidence(step_id=step_id, row_count=row_count, columns=columns)
