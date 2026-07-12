"""SQL integration: python.profile_capture / profile_hotspot virtual tables."""

from __future__ import annotations

import probing
from probing.profiling.torch_profiler.session_store import CaptureRecord, HotspotRecord
from probing.profiling.torch_profiler.session_store import get_session_store


def _seed_capture() -> str:
    store = get_session_store()
    store.add_capture(
        CaptureRecord(
            capture_id="sql-cap",
            local_step=11,
            global_step=11,
            rank=0,
            world_size=4,
            role="dp=0",
            trigger="regression",
            steps_profiled=1,
            wall_us=2000,
            status="completed",
            event_count=3,
        ),
        [
            HotspotRecord(
                capture_id="sql-cap",
                local_step=11,
                global_step=11,
                rank=0,
                bucket_kind="kernel",
                bucket_name="volta_sgemm",
                self_us=1500,
                wall_us=1600,
                calls=4,
                pct_of_capture=0.75,
            ),
            HotspotRecord(
                capture_id="sql-cap",
                local_step=11,
                global_step=11,
                rank=0,
                bucket_kind="memcpy",
                bucket_name="Memcpy DtoH",
                self_us=500,
                wall_us=500,
                calls=2,
                pct_of_capture=0.25,
            ),
        ],
    )
    return "sql-cap"


def test_profile_hotspot_sql_q1_top_k():
    _seed_capture()
    df = probing.query(
        """
        SELECT bucket_name, bucket_kind, self_us, pct_of_capture, calls
        FROM python.profile_hotspot
        WHERE capture_id = 'sql-cap'
        ORDER BY self_us DESC
        LIMIT 10
        """
    )
    assert len(df) == 2
    assert df.iloc[0]["bucket_name"] == "volta_sgemm"
    assert int(df.iloc[0]["self_us"]) == 1500


def test_profile_hotspot_sql_q2_breakdown():
    _seed_capture()
    df = probing.query(
        """
        SELECT bucket_kind, sum(self_us) AS us, sum(pct_of_capture) AS pct
        FROM python.profile_hotspot
        WHERE capture_id = 'sql-cap'
        GROUP BY bucket_kind
        ORDER BY us DESC
        """
    )
    assert len(df) == 2
    kinds = set(df["bucket_kind"].tolist())
    assert kinds == {"kernel", "memcpy"}


def test_profile_capture_sql_q8_quality():
    _seed_capture()
    df = probing.query(
        """
        SELECT capture_id, status, truncated, event_count, error
        FROM python.profile_capture
        WHERE capture_id = 'sql-cap'
        """
    )
    assert len(df) == 1
    row = df.iloc[0]
    assert row["status"] == "completed"
    assert int(row["event_count"]) == 3
    assert int(row["truncated"]) == 0
