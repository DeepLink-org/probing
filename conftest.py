import sys
from pathlib import Path

# Add python directory to sys.path to allow importing probing without installation
root_dir = Path(__file__).parent
python_dir = root_dir / "python"
sys.path.insert(0, str(python_dir))

# Note: The rust extension (_core) must be built and available.
# If running in development mode, use 'maturin develop' to build and install
# the extension into your virtual environment.

