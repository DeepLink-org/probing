# ==============================================================================
# Variables
# ==============================================================================
# Build mode: release (default) or debug
ifndef DEBUG
	CARGO_FLAGS := -r
	TARGET_DIR := release
else
	CARGO_FLAGS :=
	TARGET_DIR := debug
endif

# Frontend framework: dioxus

# OS-specific library extension
ifeq ($(shell uname -s), Darwin)
	LIB_EXT := dylib
else
	LIB_EXT := so
endif

# Cargo build command: normal (default) or zigbuild
ifndef ZIG
	CARGO_BUILD_CMD := build
	TARGET_DIR_PREFIX := target
else
	ifndef TARGET
		TARGET := x86_64-unknown-linux-gnu.2.17
	endif
	CARGO_BUILD_CMD := zigbuild --target $(TARGET)
	TARGET_ARCH := $(word 1,$(subst -, ,$(TARGET)))
	TARGET_DIR_PREFIX := target/$(TARGET_ARCH)-unknown-linux-gnu
endif

# Python version
PYTHON ?= 3.12

# Paths
DATA_SCRIPTS_DIR := python/probing
PROBING_CLI := ${TARGET_DIR_PREFIX}/${TARGET_DIR}/probing
PROBING_LIB := ${TARGET_DIR_PREFIX}/${TARGET_DIR}/libprobing.${LIB_EXT}

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
	@echo "  all        Build the wheel (default)."
	@echo "  wheel      Build the Python wheel."
	@echo "  test       Run Rust tests."
	@echo "  pytest     Run Python tests."
	@echo "  coverage-rust   Run Rust coverage (cargo llvm-cov)."
	@echo "  coverage-python Generate Python coverage (pytest-cov)."
	@echo "  coverage        Run both Rust and Python coverage and aggregate report."
	@echo "  bootstrap  Install Python versions for testing."
	@echo "  clean      Remove build artifacts."
	@echo "  frontend   Build Dioxus frontend."
	@echo "  web/dist   Build the web app (Dioxus)."
	@echo ""
	@echo "Environment Variables:"
	@echo "  DEBUG      Build mode: release (default) or debug"
	@echo "  ZIG        Use zigbuild for cross-compilation"
	@echo "  PYTHON     Python version (default: 3.12)"
	@echo ""
	@echo "Examples:"
	@echo "  make frontend              # Build Dioxus frontend"
	@echo "  make web/dist              # Build web app"

# ==============================================================================
# Build Targets
# ==============================================================================
.PHONY: wheel
wheel: ${PROBING_CLI} ${PROBING_LIB} web/dist/index.html
	@echo "Building wheel..."
ifdef TARGET
	TARGET=$(TARGET) python make_wheel.py
else
	python make_wheel.py
endif

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

${DATA_SCRIPTS_DIR}:
	@echo "Creating data scripts directory..."
	@mkdir -p ${DATA_SCRIPTS_DIR}

.PHONY: ${PROBING_CLI}
${PROBING_CLI}: ${DATA_SCRIPTS_DIR}
	@echo "Building probing CLI..."
	cargo ${CARGO_BUILD_CMD} ${CARGO_FLAGS} --package probing-cli
	cp ${PROBING_CLI} ${DATA_SCRIPTS_DIR}/probing

.PHONY: ${PROBING_LIB}
${PROBING_LIB}: ${DATA_SCRIPTS_DIR}
	@echo "Building probing library..."
	@echo "Building Dioxus frontend (pre-build)..."
	@$(MAKE) --no-print-directory web/dist
	cargo ${CARGO_BUILD_CMD} ${CARGO_FLAGS}
	cp ${PROBING_LIB} ${DATA_SCRIPTS_DIR}/libprobing.${LIB_EXT}

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

.PHONY: clean
clean:
	@echo "Cleaning up..."
	rm -rf dist
	rm -rf web/dist
	cargo clean
