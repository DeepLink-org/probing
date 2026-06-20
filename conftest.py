import sys
from pathlib import Path

# Add python directory to sys.path to allow importing probing without installation
root_dir = Path(__file__).parent
python_dir = root_dir / "python"
sys.path.insert(0, str(python_dir))

# The Rust extension (probing._core) must exist — run once: ``make develop``
# (builds _core into python/probing/ and installs the ``probing`` CLI).
# Release/CI wheel path: ``make wheel && make install-wheel``.
