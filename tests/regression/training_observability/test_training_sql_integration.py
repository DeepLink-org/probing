"""SQL engine paths for training observability (run after memtable-only tests).

Keep SQL integration tests in a dedicated module collected last within this
package so memtable fixtures are initialized before the engine scans them.
"""

import pytest

from probing.profiling.collective.record import record_comm_lite

from .conftest import COMM_COLLECTIVE_RECENT_SQL


@pytest.mark.training_observability
class TestTrainingSqlIntegration:
    def test_comm_collective_queryable_via_sql(self, rank_env, parallel_env, sql_query):
        rank_env(rank=1, world_size=8)
        parallel_env(tp_rank=0, pp_rank=1, dp_rank=1)

        record_comm_lite(
            op="all_reduce",
            duration_ms=8.5,
            group_rank=1,
            group_size=8,
            nbytes=4096,
        )

        comm_df = sql_query(COMM_COLLECTIVE_RECENT_SQL, limit=5)
        assert not comm_df.empty
        assert comm_df.iloc[0]["op"] == "all_reduce"
        assert "pp=1" in str(comm_df.iloc[0]["role"])
        assert int(comm_df.iloc[0]["bytes"]) == 4096
