"""HTTP handler tests for pytorch profile control plane."""

from __future__ import annotations

import json
from unittest.mock import MagicMock, patch

import probing.handlers.pythonext as pythonext


def test_profile_start_handler_success():
    with patch("probing.repl.torch_magic.TorchMagic") as mock_cls:
        mock_cls.return_value._start_global_profiler.return_value = MagicMock()
        result = json.loads(
            pythonext.handle_api_request(
                "pytorch/profile/start",
                {"steps": "2", "trigger": "http"},
            )
        )
    assert result["success"] is True
    mock_cls.return_value._start_global_profiler.assert_called_once_with(
        2, trigger="http"
    )


def test_profile_status_handler():
    with patch(
        "probing.profiling.torch_profiler.profiler_status",
        return_value={"running": False, "latest_capture_id": "abc"},
    ):
        result = json.loads(pythonext.handle_api_request("pytorch/profile/status", {}))
    assert result["running"] is False
    assert result["latest_capture_id"] == "abc"


def test_profile_stop_handler():
    with patch("probing.profiling.torch_profiler.get_controller") as mock_get:
        ctrl = MagicMock()
        ctrl.stop.return_value = "cap-42"
        mock_get.return_value = ctrl
        result = json.loads(pythonext.handle_api_request("pytorch/profile/stop", {}))
    assert result["success"] is True
    assert result["capture_id"] == "cap-42"
