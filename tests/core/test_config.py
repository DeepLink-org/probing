"""Unit tests for probing.config module.

Note: These tests require the probing.config submodule to be properly registered.
The config module should be available after the Rust extension is loaded.
If these tests are skipped, it means the config module hasn't been initialized yet.
"""

import unittest

try:
    import probing
except ImportError as e:
    raise unittest.SkipTest(f"probing module not available: {e}")

# Check if config module is available
# The config module is registered automatically when _core module is imported
if not hasattr(probing, "config"):
    # Try importing _core to trigger module registration
    try:
        from probing import _core
    except ImportError:
        pass
    
    if not hasattr(probing, "config"):
        raise unittest.SkipTest(
            "probing.config module not available. "
            "The config module should be registered automatically via _core module."
        )


class TestConfigModule(unittest.TestCase):
    """Test cases for probing.config module."""

    def setUp(self):
        """Set up test fixtures - clear config before each test."""
        probing.config.clear()

    def tearDown(self):
        """Clean up after each test."""
        probing.config.clear()

    def test_set_and_get_string(self):
        """Test setting and getting string values."""
        probing.config.set("test.key1", "value1")
        value = probing.config.get("test.key1")
        self.assertIsNotNone(value)
        self.assertEqual(value, "value1")

    def test_set_and_get_int(self):
        """Test setting and getting integer values."""
        probing.config.set("test.int", 42)
        value = probing.config.get("test.int")
        self.assertIsNotNone(value)
        self.assertEqual(value, 42)

        # Test large integer
        probing.config.set("test.large_int", 2**50)
        value = probing.config.get("test.large_int")
        self.assertIsNotNone(value)
        self.assertEqual(value, 2**50)

    def test_set_and_get_float(self):
        """Test setting and getting float values."""
        probing.config.set("test.float", 3.14)
        value = probing.config.get("test.float")
        self.assertIsNotNone(value)
        self.assertAlmostEqual(value, 3.14, places=5)

    def test_set_and_get_bool(self):
        """Test setting and getting boolean values."""
        probing.config.set("test.bool_true", True)
        probing.config.set("test.bool_false", False)

        value_true = probing.config.get("test.bool_true")
        value_false = probing.config.get("test.bool_false")

        self.assertIsNotNone(value_true)
        self.assertIsNotNone(value_false)
        self.assertTrue(value_true)
        self.assertFalse(value_false)

    def test_set_and_get_none(self):
        """Test setting and getting None values."""
        probing.config.set("test.none", None)
        value = probing.config.get("test.none")
        self.assertIsNone(value)

    def test_get_nonexistent_key(self):
        """Test getting a key that doesn't exist."""
        value = probing.config.get("nonexistent.key")
        self.assertIsNone(value)

    def test_get_str(self):
        """Test getting values as strings."""
        probing.config.set("test.str", "hello")
        probing.config.set("test.int", 42)
        probing.config.set("test.float", 3.14)
        probing.config.set("test.bool", True)

        self.assertEqual(probing.config.get_str("test.str"), "hello")
        self.assertEqual(probing.config.get_str("test.int"), "42")
        self.assertEqual(probing.config.get_str("test.float"), "3.14")
        self.assertEqual(probing.config.get_str("test.bool"), "True")

        # Nonexistent key
        self.assertIsNone(probing.config.get_str("nonexistent"))

    def test_contains_key(self):
        """Test checking if a key exists."""
        probing.config.set("test.key", "value")
        self.assertTrue(probing.config.contains_key("test.key"))
        self.assertFalse(probing.config.contains_key("nonexistent.key"))

    def test_remove(self):
        """Test removing keys."""
        probing.config.set("test.key", "value")
        self.assertTrue(probing.config.contains_key("test.key"))

        removed_value = probing.config.remove("test.key")
        self.assertEqual(removed_value, "value")
        self.assertFalse(probing.config.contains_key("test.key"))

        # Remove nonexistent key
        removed_none = probing.config.remove("nonexistent.key")
        self.assertIsNone(removed_none)

    def test_keys(self):
        """Test getting all keys."""
        probing.config.set("a.key", "value1")
        probing.config.set("b.key", "value2")
        probing.config.set("c.key", "value3")

        keys = probing.config.keys()
        self.assertEqual(len(keys), 3)
        self.assertIn("a.key", keys)
        self.assertIn("b.key", keys)
        self.assertIn("c.key", keys)

        # Keys should be sorted (BTreeMap guarantees ordering)
        self.assertEqual(keys[0], "a.key")
        self.assertEqual(keys[1], "b.key")
        self.assertEqual(keys[2], "c.key")

    def test_clear(self):
        """Test clearing all configuration."""
        probing.config.set("test.key1", "value1")
        probing.config.set("test.key2", "value2")
        self.assertEqual(probing.config.len(), 2)

        probing.config.clear()
        self.assertEqual(probing.config.len(), 0)
        self.assertTrue(probing.config.is_empty())

    def test_len(self):
        """Test getting the number of configuration entries."""
        self.assertEqual(probing.config.len(), 0)

        probing.config.set("test.key1", "value1")
        self.assertEqual(probing.config.len(), 1)

        probing.config.set("test.key2", "value2")
        self.assertEqual(probing.config.len(), 2)

        probing.config.remove("test.key1")
        self.assertEqual(probing.config.len(), 1)

    def test_is_empty(self):
        """Test checking if config store is empty."""
        self.assertTrue(probing.config.is_empty())

        probing.config.set("test.key", "value")
        self.assertFalse(probing.config.is_empty())

        probing.config.clear()
        self.assertTrue(probing.config.is_empty())

    def test_overwrite_value(self):
        """Test overwriting an existing value."""
        probing.config.set("test.key", "value1")
        self.assertEqual(probing.config.get("test.key"), "value1")

        probing.config.set("test.key", "value2")
        self.assertEqual(probing.config.get("test.key"), "value2")

    def test_mixed_types(self):
        """Test storing different types of values."""
        probing.config.set("str", "hello")
        probing.config.set("int", 42)
        probing.config.set("float", 3.14)
        probing.config.set("bool", True)
        probing.config.set("none", None)

        self.assertEqual(probing.config.get("str"), "hello")
        self.assertEqual(probing.config.get("int"), 42)
        self.assertAlmostEqual(probing.config.get("float"), 3.14, places=5)
        self.assertTrue(probing.config.get("bool"))
        self.assertIsNone(probing.config.get("none"))


if __name__ == "__main__":
    unittest.main()
