import json

import probing

from probing.core.engine import QueryOutcome, query_outcome


def test_query_outcome_preserves_partial_quality(monkeypatch):
    payload = {
        "data": {"names": ["rank"], "cols": [{"SeqI32": [0]}], "size": 1},
        "quality": {
            "nodes_succeeded": 1,
            "nodes_failed": ["rank-1: timeout"],
            "peer_batches_dropped": 2,
            "partial": True,
        },
    }
    monkeypatch.setattr(
        probing._core,
        "query_outcome_json",
        lambda _sql: json.dumps(payload),
        raising=False,
    )

    outcome = query_outcome("select rank from global.demo.metrics")

    assert isinstance(outcome, QueryOutcome)
    assert outcome.data.to_dict(orient="list") == {"rank": [0]}
    assert outcome.quality.nodes_succeeded == 1
    assert outcome.quality.nodes_failed == ["rank-1: timeout"]
    assert outcome.quality.peer_batches_dropped == 2
    assert outcome.quality.partial is True
