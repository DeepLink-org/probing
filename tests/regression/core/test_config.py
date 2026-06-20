import pytest

try:
    import probing.config  # noqa: F401

    config_available = True
except ImportError:
    config_available = False


def _clear_test_keys() -> None:
    """Remove test.* keys used by this module."""
    import probing

    for key in list(probing.config.keys()):
        if key.startswith("test.") or key in {
            "str",
            "int",
            "float",
            "bool",
            "none",
            "a.key",
            "b.key",
            "c.key",
        }:
            probing.config.remove(key)


@pytest.fixture
def clear_config():
    """Clear test configuration before and after each test."""
    if config_available:
        try:
            _clear_test_keys()
        except Exception:
            pass
    yield
    if config_available:
        try:
            _clear_test_keys()
        except Exception:
            pass


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_config_module_exists():
    """Test that the config module is available."""
    import probing

    assert hasattr(probing, "config")


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_set_and_get_string():
    """Test setting and getting string values."""
    import probing

    probing.config.set("test.key1", "value1")
    value = probing.config.get("test.key1")
    assert value == "value1"


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_set_and_get_int():
    """Test setting and getting integer values."""
    probing.config.set("test.int", 42)
    value = probing.config.get("test.int")
    assert value == 42

    # Test large integer
    probing.config.set("test.large_int", 2**50)
    value = probing.config.get("test.large_int")
    assert value == 2**50


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_set_and_get_float():
    """Test setting and getting float values."""
    probing.config.set("test.float", 3.14)
    value = probing.config.get("test.float")
    assert abs(value - 3.14) < 1e-5


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_set_and_get_bool():
    """Test setting and getting boolean values."""
    probing.config.set("test.bool_true", True)
    probing.config.set("test.bool_false", False)

    value_true = probing.config.get("test.bool_true")
    value_false = probing.config.get("test.bool_false")

    assert value_true is True
    assert value_false is False


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_set_and_get_none():
    """Test setting and getting None values."""
    probing.config.set("test.none", None)
    value = probing.config.get("test.none")
    assert value is None


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_get_nonexistent_key():
    """Test getting a key that doesn't exist."""
    value = probing.config.get("nonexistent.key")
    assert value is None


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_get_str():
    """Test getting values as strings."""
    probing.config.set("test.str", "hello")
    probing.config.set("test.int", 42)
    probing.config.set("test.float", 3.14)
    probing.config.set("test.bool", True)

    assert probing.config.get_str("test.str") == "hello"
    assert probing.config.get_str("test.int") == "42"
    assert probing.config.get_str("test.float") == "3.14"
    assert probing.config.get_str("test.bool") == "True"

    # Nonexistent key
    assert probing.config.get_str("nonexistent") is None


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_contains_key():
    """Test checking if a key exists."""
    probing.config.set("test.key", "value")
    assert probing.config.contains_key("test.key")
    assert not probing.config.contains_key("nonexistent.key")


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_remove():
    """Test removing keys."""
    probing.config.set("test.key", "value")
    assert probing.config.contains_key("test.key")

    removed_value = probing.config.remove("test.key")
    assert removed_value == "value"
    assert not probing.config.contains_key("test.key")

    # Remove nonexistent key
    removed_none = probing.config.remove("nonexistent.key")
    assert removed_none is None


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_keys():
    """Test getting all keys."""
    import probing

    _clear_test_keys()
    probing.config.set("a.key", "value1")
    probing.config.set("b.key", "value2")
    probing.config.set("c.key", "value3")

    keys = [k for k in probing.config.keys() if k in {"a.key", "b.key", "c.key"}]
    assert len(keys) == 3
    assert "a.key" in keys
    assert "b.key" in keys
    assert "c.key" in keys

    # Keys should be sorted (BTreeMap guarantees ordering)
    assert keys[0] == "a.key"
    assert keys[1] == "b.key"
    assert keys[2] == "c.key"


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_clear():
    """Test clearing all configuration."""
    probing.config.set("test.key1", "value1")
    probing.config.set("test.key2", "value2")
    assert probing.config.len() >= 2

    _clear_test_keys()
    assert probing.config.get("test.key1") is None
    assert probing.config.get("test.key2") is None


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_len():
    """Test getting the number of configuration entries."""
    import probing

    _clear_test_keys()
    base = probing.config.len()

    probing.config.set("test.key1", "value1")
    assert probing.config.len() == base + 1

    probing.config.set("test.key2", "value2")
    assert probing.config.len() == base + 2

    probing.config.remove("test.key1")
    assert probing.config.len() == base + 1

    _clear_test_keys()
    assert probing.config.len() == base


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_is_empty():
    """Test checking if config store is empty for test keys."""
    _clear_test_keys()
    probing.config.set("test.key", "value")
    assert not probing.config.is_empty()
    _clear_test_keys()
    assert probing.config.get("test.key") is None


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_overwrite_value():
    """Test overwriting an existing value."""
    probing.config.set("test.key", "value1")
    assert probing.config.get("test.key") == "value1"

    probing.config.set("test.key", "value2")
    assert probing.config.get("test.key") == "value2"


@pytest.mark.skipif(not config_available, reason="probing.config module not available")
def test_mixed_types():
    """Test storing different types of values."""
    probing.config.set("str", "hello")
    probing.config.set("int", 42)
    probing.config.set("float", 3.14)
    probing.config.set("bool", True)
    probing.config.set("none", None)

    assert probing.config.get("str") == "hello"
    assert probing.config.get("int") == 42
    assert abs(probing.config.get("float") - 3.14) < 1e-5
    assert probing.config.get("bool") is True
    assert probing.config.get("none") is None
