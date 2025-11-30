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
PYTHON ?= 3.12

# Pytest runner command
PYTEST_RUN := PROBING=1 PYTHONPATH=python/ uv run --python ${PYTHON} -w pytest -w websockets -w pandas -w torch -w ipykernel -- python -m pytest --doctest-modules

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
	@echo "  test            Run Rust tests."
	@echo "  pytest          Run Python tests."
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
	@echo "  PYTHON     Python version (default: 3.12)"
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
test:
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

.PHONY: coverage-rust
coverage-rust:
	@echo "Running Rust coverage (requires cargo-llvm-cov)..."
	cargo llvm-cov nextest run --workspace --no-default-features --nff --lcov --output-path coverage/rust.lcov --ignore-filename-regex '(.*/tests?/|.*/benches?/|.*/examples?/)' || echo "Install with: cargo install cargo-llvm-cov"
	cargo llvm-cov report nextest --workspace --no-default-features --nff --json --output-path coverage/rust-summary.json || true

.PHONY: coverage-python
coverage-python:
	@echo "Running Python coverage..."
	mkdir -p coverage
	${PYTEST_RUN} --cov=python/probing --cov=tests --cov-report=term --cov-report=xml:coverage/python-coverage.xml --cov-report=html:coverage/python-html python/probing tests || echo "Install pytest-cov via: uv add pytest-cov"

.PHONY: coverage
coverage: coverage-rust coverage-python
	@echo "Aggregating coverage summaries..."
	python scripts/coverage/aggregate.py || echo "Aggregation script missing or failed"

.PHONY: bootstrap
bootstrap:
	@echo "Bootstrapping Python environments..."
	uv python install 3.8 3.9 3.10 3.11 3.12 3.13

.PHONY: pytest
pytest:
	@echo "Running pytest for probing package..."
	${PYTEST_RUN} python/probing
	@echo "Running pytest for tests directory..."
	${PYTEST_RUN} tests

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
