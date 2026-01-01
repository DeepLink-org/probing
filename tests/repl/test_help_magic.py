"""Tests for help magic commands."""

import os
import sys

# Add python/ to path explicitly
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "python"))

import pytest
from probing.repl import CodeExecutor


@pytest.fixture
def executor():
    """Create a code executor with a real IPython kernel."""
    ex = CodeExecutor()
    yield ex
    ex.shutdown()


def test_cmds_shows_probing_commands(executor):
    """Test that %cmds displays probing magic commands."""
    result = executor.execute("%cmds")
    output = result.output

    # Check header
    assert "🔮 Probing Magic Commands" in output

    # Check that some expected categories are shown
    assert "Query" in output or "Trace" in output or "Inspect" in output

    # Check that some expected commands are listed
    assert "%query" in output or "%trace" in output or "%inspect" in output


def test_cmds_shows_all_magics(executor):
    """Test that %cmds --all displays all magic commands."""
    result = executor.execute("%cmds --all")
    output = result.output

    # Check header
    assert "🔮 All Magic Commands" in output

    # Check that IPython built-in magics are shown
    assert "Basics" in output or "Codes" in output
    assert "%load" in output or "%magic" in output

    # Check that probing magics are also included
    assert "Query" in output or "Trace" in output


def test_cmds_shows_tips(executor):
    """Test that %cmds includes usage tips."""
    result = executor.execute("%cmds")
    output = result.output

    # Check tips section
    assert "💡 Tips:" in output
    assert "%command?" in output or "detailed help" in output
