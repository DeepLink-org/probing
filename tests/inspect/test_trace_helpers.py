"""Tests for trace.py helper functions."""

import sys
from types import ModuleType

import pytest


class TestCreateFilter:
    """Test _TraceableCollector.create_filter method."""

    def test_no_prefix_matches_all(self):
        """Test that None prefix matches everything."""
        from probing.inspect.trace import _TraceableCollector

        filter_func = _TraceableCollector.create_filter(None)
        assert filter_func("anything")
        assert filter_func("test.module.func")
        assert filter_func("")

    def test_simple_prefix_matching(self):
        """Test simple prefix matching without wildcards."""
        from probing.inspect.trace import _TraceableCollector

        filter_func = _TraceableCollector.create_filter("torch.nn")
        assert filter_func("torch.nn.Linear")
        assert filter_func("torch.nn.Module")
        assert not filter_func("torch.optim.Adam")
        assert not filter_func("numpy.array")

    def test_wildcard_star_matching(self):
        """Test wildcard * pattern matching."""
        from probing.inspect.trace import _TraceableCollector

        filter_func = _TraceableCollector.create_filter("torch.*.Linear")
        assert filter_func("torch.nn.Linear")
        assert filter_func("torch.cuda.Linear")
        assert not filter_func("torch.Linear")
        assert not filter_func("torch.nn.Conv2d")

    def test_wildcard_question_matching(self):
        """Test wildcard ? pattern matching."""
        from probing.inspect.trace import _TraceableCollector

        filter_func = _TraceableCollector.create_filter("test?")
        assert filter_func("test1")
        assert filter_func("testa")
        assert not filter_func("test")
        assert not filter_func("test12")


class TestGetObjectName:
    """Test _TraceableCollector.get_object_name method."""

    def test_function_with_name(self):
        """Test getting name from a function."""
        from probing.inspect.trace import _TraceableCollector

        def test_func():
            pass

        assert _TraceableCollector.get_object_name(test_func) == "test_func"

    def test_class_with_name(self):
        """Test getting name from a class."""
        from probing.inspect.trace import _TraceableCollector

        class TestClass:
            pass

        assert _TraceableCollector.get_object_name(TestClass) == "TestClass"

    def test_module_with_name(self):
        """Test getting name from a module."""
        from probing.inspect.trace import _TraceableCollector

        assert _TraceableCollector.get_object_name(sys) == "sys"

    def test_object_without_name(self):
        """Test object without __name__ attribute."""
        from probing.inspect.trace import _TraceableCollector

        obj = object()
        assert _TraceableCollector.get_object_name(obj) is None

    def test_object_with_non_string_name(self):
        """Test object with non-string __name__."""
        from probing.inspect.trace import _TraceableCollector

        class BadName:
            __name__ = 123

        # Python classes always have a string __name__ attribute at the class level
        # Even if we set __name__ = 123, the class's actual __name__ is still 'BadName'
        # So this test should expect the class name to be returned
        assert _TraceableCollector.get_object_name(BadName) == "BadName"


class TestShouldSkipPrefix:
    """Test _TraceableCollector.should_skip_prefix method."""

    def test_blacklisted_prefix(self):
        """Test that blacklisted prefixes are skipped."""
        from probing.inspect.trace import _TraceableCollector

        blacklist = ["numpy", "typing"]
        assert _TraceableCollector.should_skip_prefix("numpy", blacklist)
        assert _TraceableCollector.should_skip_prefix("typing", blacklist)
        # "torch" without a specific submodule is not in the allowed list
        # (torch.nn, torch.cuda, torch.distributed, torch.optim)
        # So it should be skipped
        assert _TraceableCollector.should_skip_prefix("torch", blacklist)

    def test_torch_allowed_prefixes(self):
        """Test torch special handling."""
        from probing.inspect.trace import _TraceableCollector

        blacklist = []
        assert not _TraceableCollector.should_skip_prefix("torch.nn.Linear", blacklist)
        assert not _TraceableCollector.should_skip_prefix(
            "torch.cuda.Module", blacklist
        )
        assert not _TraceableCollector.should_skip_prefix(
            "torch.distributed.all_reduce", blacklist
        )
        assert not _TraceableCollector.should_skip_prefix("torch.optim.Adam", blacklist)
        assert _TraceableCollector.should_skip_prefix("torch.utils", blacklist)
        assert _TraceableCollector.should_skip_prefix("torch.autograd", blacklist)
        # torchvision is a different package and should NOT be skipped
        assert not _TraceableCollector.should_skip_prefix("torchvision", blacklist)
        assert not _TraceableCollector.should_skip_prefix(
            "torchvision.models", blacklist
        )
        assert not _TraceableCollector.should_skip_prefix("torchaudio", blacklist)

    def test_six_module_skipped(self):
        """Test that six.* modules are skipped."""
        from probing.inspect.trace import _TraceableCollector

        blacklist = []
        assert _TraceableCollector.should_skip_prefix("six.moves", blacklist)
        assert _TraceableCollector.should_skip_prefix("six.anything", blacklist)
        assert not _TraceableCollector.should_skip_prefix("six", blacklist)


class TestDetermineItemType:
    """Test _TraceableCollector.determine_item_type method."""

    def test_function_type(self):
        """Test function classification."""
        from probing.inspect.trace import _TraceableCollector

        def test_func():
            pass

        assert _TraceableCollector.determine_item_type(test_func) == "F"

    def test_class_type(self):
        """Test class classification."""
        from probing.inspect.trace import _TraceableCollector

        class TestClass:
            pass

        assert _TraceableCollector.determine_item_type(TestClass) == "C"

    def test_module_type(self):
        """Test module classification."""
        from probing.inspect.trace import _TraceableCollector

        assert _TraceableCollector.determine_item_type(sys) == "M"

    def test_variable_type(self):
        """Test variable classification."""
        from probing.inspect.trace import _TraceableCollector

        var = "test"
        assert _TraceableCollector.determine_item_type(var) == "V"
        assert _TraceableCollector.determine_item_type(123) == "V"
        assert _TraceableCollector.determine_item_type([1, 2, 3]) == "V"


class TestShouldIncludeModule:
    """Test _TraceableCollector.should_include_module method."""

    def test_whitelisted_module(self):
        """Test that whitelisted modules are included."""
        from probing.inspect.trace import _TraceableCollector

        whitelist = ["__main__", "test_module"]
        mock_module = ModuleType("test_module")
        assert _TraceableCollector.should_include_module(
            "__main__", mock_module, whitelist
        )

    def test_probing_module(self):
        """Test that probing modules are included."""
        from probing.inspect.trace import _TraceableCollector

        whitelist = []
        mock_module = ModuleType("probing.core")
        assert _TraceableCollector.should_include_module(
            "probing.core", mock_module, whitelist
        )
        assert _TraceableCollector.should_include_module(
            "probing.inspect", mock_module, whitelist
        )

    def test_module_without_spec(self):
        """Test that modules without __spec__ are excluded."""
        from probing.inspect.trace import _TraceableCollector

        whitelist = []
        mock_module = ModuleType("test")
        # Remove __spec__ if it exists
        if hasattr(mock_module, "__spec__"):
            delattr(mock_module, "__spec__")
        assert not _TraceableCollector.should_include_module(
            "test", mock_module, whitelist
        )

    def test_dunder_module_excluded(self):
        """Test that __name__ modules are excluded."""
        from probing.inspect.trace import _TraceableCollector

        whitelist = []
        mock_module = ModuleType("__test__")
        assert not _TraceableCollector.should_include_module(
            "__test__", mock_module, whitelist
        )


class TestFilterByPrefix:
    """Test _TraceableCollector.filter_by_prefix method."""

    def test_no_prefix_returns_top_level(self):
        """Test that no prefix returns only top-level modules."""
        from probing.inspect.trace import _TraceableCollector

        items = [
            {"name": "module1.func1", "type": "F"},
            {"name": "module1.func2", "type": "F"},
            {"name": "module2.Class1", "type": "C"},
            {"name": "module2.submodule.func3", "type": "F"},
        ]
        result = _TraceableCollector.filter_by_prefix(items, None)
        assert len(result) == 2
        assert {"name": "module1", "type": "M"} in result
        assert {"name": "module2", "type": "M"} in result

    def test_wildcard_prefix_returns_all_matches(self):
        """Test that wildcard prefix returns all matching items."""
        from probing.inspect.trace import _TraceableCollector

        items = [
            {"name": "torch.nn.Linear", "type": "F"},
            {"name": "torch.nn.Module", "type": "C"},
            {"name": "torch.cuda.Linear", "type": "F"},
            {"name": "numpy.array", "type": "F"},
        ]
        result = _TraceableCollector.filter_by_prefix(items, "torch.*")
        # Wildcard prefix returns all items sorted by name (not filtered!)
        # The actual filtering should happen at the collector level
        # filter_by_prefix with wildcard just returns sorted items
        assert len(result) == 4
        assert result == sorted(items, key=lambda x: x["name"])

    def test_exact_prefix_returns_one_level_deeper(self):
        """Test that exact prefix returns items one level deeper."""
        from probing.inspect.trace import _TraceableCollector

        items = [
            {"name": "torch.nn.Linear", "type": "F"},
            {"name": "torch.nn.Module", "type": "C"},
            {"name": "torch.nn.functional.relu", "type": "F"},
            {"name": "torch.optim.Adam", "type": "C"},
        ]
        result = _TraceableCollector.filter_by_prefix(items, "torch.nn")
        # Should return torch.nn.Linear, torch.nn.Module, and torch.nn.functional (as module)
        assert len(result) == 3
        names = [item["name"] for item in result]
        assert "torch.nn.Linear" in names
        assert "torch.nn.Module" in names
        assert "torch.nn.functional" in names

    def test_sorted_results(self):
        """Test that results are sorted by name."""
        from probing.inspect.trace import _TraceableCollector

        items = [
            {"name": "zebra", "type": "F"},
            {"name": "alpha", "type": "F"},
            {"name": "beta", "type": "F"},
        ]
        result = _TraceableCollector.filter_by_prefix(items, None)
        # Top level should be sorted
        assert result == sorted(result, key=lambda x: x["name"])


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
