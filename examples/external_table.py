# The probing module is now built as a standard Python package via maturin
# No need for manual ctypes loading - just import the module
import probing

tbl = probing.ExternalTable("test", ["a", "b"])
assert tbl.names() == ["a", "b"]

tbl = probing.ExternalTable.get("test")
assert tbl.names() == ["a", "b"]

for i in range(20):
    tbl.append([i, i+1])
    assert len(tbl.take(10)) == min(i+1, 10), f"error at {i}"

probing.ExternalTable.drop("test")