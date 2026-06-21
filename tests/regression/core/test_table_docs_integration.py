"""Integration tests for code-first table docs through the live probing engine."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile

import probing


def _python_path_env(*, defer_engine_init: bool = False) -> dict[str, str]:
    env = os.environ.copy()
    # Do not inherit PROBING=1: wheel probing.pth site-hook would import probing
    # (and initialize the engine) before the subprocess script sets PROBING_CLI_MODE.
    env.pop("PROBING", None)
    env.pop("PROBING_ORIGINAL", None)
    if defer_engine_init:
        env["PROBING_CLI_MODE"] = "1"
    else:
        env.pop("PROBING_CLI_MODE", None)
    return env


def _run_fresh_probing_script(
    body: str, *, defer_engine_init: bool = False
) -> subprocess.CompletedProcess[str]:
    cli_mode_line = (
        'os.environ["PROBING_CLI_MODE"] = "1"'
        if defer_engine_init
        else 'os.environ.pop("PROBING_CLI_MODE", None)'
    )
    script = f"""
import json
import os
import sys
import tempfile

os.environ["PROBING"] = "1"
{cli_mode_line}
os.environ["PROBING_DATA_DIR"] = tempfile.mkdtemp(prefix="probing_doc_it_")

{body}
"""
    with tempfile.NamedTemporaryFile(mode="w", suffix=".py", delete=False) as handle:
        handle.write(script)
        script_path = handle.name
    try:
        return subprocess.run(
            [sys.executable, script_path],
            env=_python_path_env(defer_engine_init=defer_engine_init),
            capture_output=True,
            text=True,
            timeout=60,
        )
    finally:
        os.unlink(script_path)


def test_register_table_docs_visible_in_catalog_subprocess():
    """Fresh process: register_table_docs before first query lands in column_docs."""
    table = f"manual_reg_{os.getpid()}"
    result = _run_fresh_probing_script(
        f"""
from probing import _core
table = "{table}"
_core.register_table_docs(
    f"python.{{table}}",
    "Manual registration table",
    {{"payload": "json payload column"}},
)
_core.start_local()

import probing
ext = probing.ExternalTable.get_or_create(table, ["payload"])
ext.append(["{{}}"])

df = probing.query(
    "SELECT description FROM probe.probing.column_docs "
    f"WHERE table_schema = 'python' AND table_name = '{{table}}' "
    "AND column_name = 'payload'"
)
assert len(df) == 1
assert "json payload" in str(df["description"].iloc[0])
tbl = probing.query(
    "SELECT description FROM probe.probing.table_docs "
    f"WHERE table_schema = 'python' AND table_name = '{{table}}'"
)
assert len(tbl) == 1
assert "Manual registration" in str(tbl["description"].iloc[0])
print("OK")
""",
        defer_engine_init=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "OK" in result.stdout


def test_table_decorator_docs_visible_in_catalog_subprocess():
    """@table metadata is registered before the engine catalog is built."""
    result = _run_fresh_probing_script(
        """
from dataclasses import dataclass, field
import importlib

table_mod = importlib.import_module("probing.core.table")

@table_mod.table("subproc_metrics")
@dataclass
class Metrics:
    \"\"\"Subprocess metrics table.\"\"\"
    score: float = field(metadata={"doc": "model score"})
    rank: int = field(metadata={"doc": "process rank"})

Metrics.append(Metrics(0.75, 2))

from probing import _core
_core.start_local()

import probing
score = probing.query(
    "SELECT description FROM probe.probing.column_docs "
    "WHERE table_schema = 'python' AND table_name = 'subproc_metrics' "
    "AND column_name = 'score'"
)
assert len(score) == 1
assert "model score" in str(score["description"].iloc[0])
tbl = probing.query(
    "SELECT description FROM probe.probing.table_docs "
    "WHERE table_schema = 'python' AND table_name = 'subproc_metrics'"
)
assert len(tbl) == 1
assert "Subprocess metrics" in str(tbl["description"].iloc[0])
print("OK")
""",
        defer_engine_init=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "OK" in result.stdout


def test_describe_builtin_hccl_via_query():
    df = probing.query("DESCRIBE probe.probing.column_docs")
    assert "comment" in df.columns
    assert "table_comment" in df.columns
    assert "column_name" in df.columns
    assert "description" in df["column_name"].tolist()


def test_hccl_catalog_and_select_roundtrip():
    """Builtin HCCL docs are queryable; YAML synonyms remain on table_docs."""
    col = probing.query(
        "SELECT description FROM probe.probing.column_docs "
        "WHERE table_schema = 'hccl' AND table_name = 'tasks' "
        "AND column_name = 'local_rank'"
    )
    assert len(col) == 1
    assert "rank" in str(col["description"].iloc[0]).lower()

    meta = probing.query(
        "SELECT description, synonyms FROM probe.probing.table_docs "
        "WHERE table_schema = 'nccl' AND table_name = 'proxy_ops'"
    )
    assert len(meta) == 1
    assert "culprit" in str(meta["description"].iloc[0]).lower() or "NCCL" in str(
        meta["description"].iloc[0]
    )
    assert "proxy" in str(meta["synonyms"].iloc[0]).lower()


def test_describe_json_shape_subprocess():
    """Sanity-check DESCRIBE rewrite columns in an isolated process."""
    result = _run_fresh_probing_script(
        """
import json
import probing
df = probing.query("DESCRIBE probe.probing.table_docs")
payload = {
    "columns": list(df.columns),
    "rows": len(df),
}
print(json.dumps(payload))
"""
    )
    assert result.returncode == 0, result.stdout + result.stderr
    payload = json.loads(result.stdout.strip().splitlines()[-1])
    assert payload["rows"] > 0
    assert "comment" in payload["columns"]
    assert "table_comment" in payload["columns"]
