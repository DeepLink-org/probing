"""Unit tests for query_magic module - testing core functionality."""

import pytest
import pandas as pd
from unittest.mock import patch
import sys
import os

# Add python directory to path
python_dir = os.path.join(os.path.dirname(__file__), '../../python')
if python_dir not in sys.path:
    sys.path.insert(0, python_dir)

from probing.repl.query_magic import QueryMagic
from traitlets.config.configurable import Configurable


@pytest.fixture
def magic():
    """Create a QueryMagic instance."""
    shell = Configurable()
    shell.user_ns = {}
    return QueryMagic(shell=shell)


def test_query_basic(magic):
    """Test basic query execution."""
    mock_df = pd.DataFrame({'a': [1, 2]})
    
    with patch('probing.core.engine.query', return_value=mock_df), \
         patch('probing.repl.query_magic.display') as mock_display:
        magic.query("SELECT * FROM table")
        mock_display.assert_called_once_with(mock_df)


def test_q_alias(magic):
    """Test %q is an alias for %query."""
    mock_df = pd.DataFrame({'x': [1]})
    
    with patch('probing.core.engine.query', return_value=mock_df), \
         patch('probing.repl.query_magic.display') as mock_display:
        magic.q("SELECT x FROM table")
        mock_display.assert_called_once_with(mock_df)


def test_tables(magic):
    """Test %tables command."""
    with patch('probing.core.engine.query') as mock_query, \
         patch('probing.repl.query_magic.display'):
        magic.tables("")
        mock_query.assert_called_once_with("SHOW TABLES")


def test_describe(magic):
    """Test %describe command."""
    with patch('probing.core.engine.query') as mock_query, \
         patch('probing.repl.query_magic.display'):
        magic.describe("my_table")
        mock_query.assert_called_once_with("DESCRIBE my_table")


def test_peek_default_limit(magic):
    """Test %peek with default limit."""
    with patch('probing.core.engine.query') as mock_query, \
         patch('probing.repl.query_magic.display'):
        magic.peek("my_table")
        mock_query.assert_called_once_with("SELECT * FROM my_table LIMIT 10")


def test_peek_custom_limit(magic):
    """Test %peek with custom limit."""
    with patch('probing.core.engine.query') as mock_query, \
         patch('probing.repl.query_magic.display'):
        magic.peek("--limit 5 my_table")
        mock_query.assert_called_once_with("SELECT * FROM my_table LIMIT 5")


def test_load_ext(magic):
    """Test %load_ext command."""
    with patch('probing.core.engine.load_extension') as mock_load:
        magic.load_ext("torch")
        mock_load.assert_called_once_with("torch")


def test_query_error_handling(magic):
    """Test query error handling."""
    with patch('probing.core.engine.query', side_effect=Exception("Error")):
        # Should not raise, just handle gracefully
        magic.query("BAD SQL")
