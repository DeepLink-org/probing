# ==============================================================================
# Variables
# ==============================================================================
# Build mode: release (default) or debug
ifndef DEBUG
	MATURIN_FLAGS := --release
	CARGO_FLAGS := --release
else
	MATURIN_FLAGS :=
	CARGO_FLAGS :=
endif

# Frontend framework: dioxus

# OS-specific library extension
ifeq ($(shell uname -s), Darwin)
	LIB_EXT := dylib
else
	LIB_EXT := so
endif

# Cross-compilation support
ifdef ZIG
	ifdef TARGET
		MATURIN_FLAGS += --zig --target $(TARGET)
	endif
endif

# Python version
PYTHON ?= python3

# Pytest runner command
# Lightweight version without uv
PYTEST_RUN := PROBING=1 PYTHONPATH=python/ $(PYTHON) -m pytest

# ==============================================================================
# Standard Targets
# ==============================================================================
.PHONY: all
all: wheel

.PHONY: help
help:
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@echo "  all             Build the wheel (default)."
	@echo "  setup           Install dev tools and environment (pre-commit, etc.)."
	@echo "  wheel           Build the Python wheel using maturin."
	@echo "  develop         Install the package in editable mode."
	@echo "  test            Run all tests (Rust + Python)."
	@echo "  test-rust       Run Rust tests."
	@echo "  test-python     Run Python tests."
	@echo "  coverage-rust   Run Rust coverage (cargo llvm-cov)."
	@echo "  coverage-python Generate Python coverage (pytest-cov)."
	@echo "  coverage        Run both Rust and Python coverage and aggregate report."
	@echo "  bootstrap       Install Python versions for testing."
	@echo "  clean           Remove build artifacts."
	@echo "  frontend        Build Dioxus frontend."
	@echo "  web/dist        Build the web app (Dioxus)."
	@echo "  docs            Build Sphinx documentation."
	@echo "  docs-serve      Start live preview server for documentation."
	@echo ""
	@echo "Environment Variables:"
	@echo "  DEBUG      Build mode: release (default) or debug"
	@echo "  ZIG        Use zigbuild for cross-compilation"
	@echo "  TARGET     Target architecture for cross-compilation"
	@echo "  PYTHON     Python interpreter to use (default: python3)"
	@echo ""

# ==============================================================================
# Setup Targets
# ==============================================================================
.PHONY: setup
setup:
	@echo "Setting up development environment..."
	@echo "Installing pre-commit..."
	@if command -v pip >/dev/null 2>&1; then pip install pre-commit; else echo "pip not found, skipping pre-commit install"; fi
	@if command -v pre-commit >/dev/null 2>&1; then pre-commit install; else echo "pre-commit not found, skipping hook install"; fi
	@echo "Environment setup complete."

# ==============================================================================
# Build Targets
# ==============================================================================
.PHONY: wheel
wheel: web/dist/index.html
	@echo "Building wheel with maturin..."
	maturin build $(MATURIN_FLAGS)

.PHONY: develop
develop:
	@echo "Installing in editable mode..."
	maturin develop $(MATURIN_FLAGS)

# Ensure frontend assets exist before packaging
web/dist/index.html:
	@echo "Ensuring frontend assets..."
	@$(MAKE) --no-print-directory frontend

.PHONY: web/dist
web/dist:
	@echo "Building Dioxus web app..."
	@mkdir -p web/dist
	cd web && dx build --release
	@echo "Copying Dioxus build output to web/dist..."
	cp -r web/target/dx/web/release/web/public/* web/dist/
	@echo "Copying static assets..."
	@mkdir -p web/dist/assets
	@cp -f web/assets/*.svg web/dist/assets/ 2>/dev/null || true
	cd ..

# Convenience targets for frontend builds
.PHONY: frontend
frontend:
	@echo "Building Dioxus frontend..."
	$(MAKE) web/dist

# ==============================================================================
# Testing & Utility Targets
# ==============================================================================

.PHONY: test
test: test-rust test-python

.PHONY: test-rust
test-rust:
	@echo "Running Rust tests..."
	@# Set Python environment variables for pyenv if available
	@if command -v pyenv >/dev/null 2>&1; then \
		PYTHON_PATH=$$(pyenv which python3 2>/dev/null || echo ""); \
		if [ -n "$$PYTHON_PATH" ]; then \
			export PYTHON_SYS_EXECUTABLE=$$PYTHON_PATH; \
			export PYO3_PYTHON=$$PYTHON_PATH; \
			echo "Using pyenv Python: $$PYTHON_PATH"; \
		fi; \
	fi; \
	cargo nextest run --workspace --no-default-features --nff

# Renamed from 'pytest' to 'test-python' for consistency
.PHONY: test-python
test-python:
	@echo "Running pytest for probing package..."
	# Note: We rely on pyproject.toml/pytest.ini for configuration options like --doctest-modules
	${PYTEST_RUN} python/probing tests

.PHONY: pytest
pytest: test-python

.PHONY: coverage-rust
coverage-rust:
	@echo "Running Rust coverage (requires cargo-llvm-cov)..."
	# Matches CI workflow step "Run Rust tests and collect coverage"
	# Uses --lcov and --output-path coverage.lcov to match CI artifact naming
	cargo llvm-cov clean --workspace
	cargo llvm-cov nextest run --workspace --no-default-features --nff --lcov --output-path coverage.lcov --ignore-filename-regex '(.*/tests?/|.*/benches?/|.*/examples?/)' || echo "Install with: cargo install cargo-llvm-cov"
	cargo llvm-cov report nextest --workspace --no-default-features --nff --json --output-path coverage.json || true

.PHONY: coverage-python
coverage-python:
	@echo "Running Python coverage..."
	# Matches CI workflow step "Run Python tests and collect coverage"
	# Uses coverage.xml as output to match CI artifact naming
	${PYTEST_RUN} --cov=python/probing --cov=tests --cov-report=xml:coverage.xml --cov-report=term python/probing tests || echo "Install pytest-cov via: uv add pytest-cov"

.PHONY: coverage
coverage: coverage-rust coverage-python
	@echo "Aggregating coverage summaries..."
	python scripts/coverage/aggregate.py || echo "Aggregation script missing or failed"

.PHONY: bootstrap
bootstrap:
	@echo "Bootstrapping Python environments..."
	uv python install 3.8 3.9 3.10 3.11 3.12 3.13

.PHONY: docs docs-serve
docs:
	@echo "Building Sphinx documentation..."
	@cd docs && $(MAKE) html

docs-serve:
	@echo "Starting documentation live preview server..."
	@cd docs && $(MAKE) serve

.PHONY: clean
clean:
	@echo "Cleaning up..."
	rm -rf dist
	rm -rf web/dist
	rm -rf docs/_build
	cargo clean
	rm -f coverage.lcov coverage.xml coverage.json
