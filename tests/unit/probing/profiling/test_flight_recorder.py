from __future__ import annotations


def test_flight_recorder_snapshot_flattens_entries(monkeypatch):
    from probing.profiling import flight_recorder as fr

    payload = {
        "version": "2.5",
        "comm_lib_version": "nccl-test",
        "pg_config": {
            "0": {"name": "0", "desc": "default_pg", "ranks": "[0, 1]"},
        },
        "pg_status": {
            "0": {
                "last_enqueued_collective": 4,
                "last_started_collective": 3,
                "last_completed_collective": 2,
            },
        },
        "entries": [
            {
                "frames": [
                    {"name": "train_step", "filename": "train.py", "line": 42},
                ],
                "record_id": 7,
                "pg_id": 0,
                "process_group": ("0", "default_pg"),
                "collective_seq_id": 3,
                "p2p_seq_id": 0,
                "op_id": 9,
                "profiling_name": "nccl:all_reduce",
                "time_created_ns": 100,
                "input_sizes": [[3, 4]],
                "input_dtypes": ["Float"],
                "output_sizes": [[3, 4]],
                "output_dtypes": ["Float"],
                "state": "completed",
                "time_discovered_started_ns": 120,
                "time_discovered_completed_ns": 140,
                "retired": True,
                "timeout_ms": 600000,
                "is_p2p": False,
            }
        ],
    }

    records = []
    statuses = []
    monkeypatch.setattr(fr, "_dump_nccl_trace", lambda **_: payload)
    monkeypatch.setattr(fr, "_rank_world", lambda: (1, 2))
    monkeypatch.setattr(fr.TorchNcclFlightRecord, "append_many", records.extend)
    monkeypatch.setattr(fr.TorchNcclPgStatus, "append_many", statuses.extend)

    result = fr.snapshot()

    assert result["ok"] is True
    assert result["rank"] == 1
    assert result["records"] == 1
    assert result["process_groups"] == 1
    assert records[0].profiling_name == "nccl:all_reduce"
    assert records[0].collective_seq_id == 3
    assert records[0].top_frame == "train_step (train.py:42)"
    assert statuses[0].last_enqueued_collective == 4
