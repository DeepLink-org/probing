# Probing Makefile
#
#   develop          → maturin develop (Rust/Python daily loop)
#   frontend         → web/dist/  (manual, needs dx)
#   wheel            → bundle skills + UI, then maturin build
#   frontend wheel   → full release path
#
.DEFAULT_GOAL := help

ifdef DEBUG
	MATURIN_RELEASE :=
	CARGO_RELEASE :=
else
	MATURIN_RELEASE := --release
	CARGO_RELEASE := --release
endif

UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Linux)
	MATURIN_FEATURES := gpu,gpu-cuda
else
	MATURIN_FEATURES := gpu
endif

MATURIN_FLAGS := $(MATURIN_RELEASE) --features $(MATURIN_FEATURES)
DX_PUBLIC := web/target/dx/web/release/web/public

ifdef ZIG
ifdef TARGET
	MATURIN_FLAGS += --zig --target $(TARGET)
endif
endif

PYTHON ?= $(shell test -x .venv/bin/python && echo .venv/bin/python || echo python3)
DEV_PTH := python/probing/dev_pth.py
DEV_PY_DEPS := pyyaml pytest pytest-cov coverage ipython ipykernel

PYTEST_RUN := PROBING=1 $(PYTHON) -m pytest
PYTEST_UNIT_ARGS := tests/unit
PYTEST_REGRESSION_ARGS := tests/regression
PYTEST_ARGS := $(PYTEST_UNIT_ARGS) $(PYTEST_REGRESSION_ARGS)

CLIPPY_DENY := -- -D warnings
CLIPPY_WORKSPACE := cargo clippy --workspace --all-targets --no-default-features $(CLIPPY_DENY)
CLIPPY_CORE := cargo clippy -p probing-core --all-targets --no-default-features $(CLIPPY_DENY)
CLIPPY_WEB := cd web && cargo clippy --all-targets $(CLIPPY_DENY)

# ==============================================================================
.PHONY: help
help:
	@echo "Probing — make [target]    (see docs/src/contributing.md)"
	@echo ""
	@echo "  develop / dev     Bootstrap: _core, CLI, pytest, site hook"
	@echo "  core              Rebuild probing._core after Rust edits"
	@echo "  frontend          Build web/dist/ (dx; manual)"
	@echo "  wheel             Build dist/*.whl (needs web/dist/; bundles skills + UI)"
	@echo "  wheel-ci          wheel with zig cross-compile on Linux"
	@echo "  install-wheel     pip install dist/probing-*.whl"
	@echo "  test / lint       Full test and lint suites"
	@echo "  check-dev         Quick env sanity check"
	@echo "  clean             Remove build artifacts"
	@echo ""
	@echo "Env: PYTHON  DEBUG=1  ZIG=1 TARGET=<triplet>"

# ==============================================================================
.PHONY: setup install-dev-python-deps
setup:
	@if command -v pip >/dev/null 2>&1; then pip install pre-commit; fi
	@if command -v pre-commit >/dev/null 2>&1; then pre-commit install; fi

install-dev-python-deps:
	@if $(PYTHON) -c "import pytest, yaml" 2>/dev/null; then \
		echo "  dev Python deps OK"; \
	elif $(PYTHON) -c "import pip" 2>/dev/null; then \
		$(PYTHON) -m pip install -q -U pip $(DEV_PY_DEPS); \
	elif command -v uv >/dev/null 2>&1; then \
		uv pip install -q --python $(PYTHON) $(DEV_PY_DEPS); \
	else \
		$(PYTHON) -m ensurepip --upgrade; \
		$(PYTHON) -m pip install -q -U pip $(DEV_PY_DEPS); \
	fi

# ==============================================================================
.PHONY: core develop dev check-dev frontend wheel wheel-ci install-wheel wheel-bundle nccl-profiler-lib

core: nccl-profiler-lib
	maturin develop $(MATURIN_FLAGS)

develop: core install-dev-python-deps
	$(PYTHON) $(DEV_PTH) install
	@$(MAKE) --no-print-directory check-dev

dev: develop

check-dev:
	@$(PYTHON) $(DEV_PTH) status >/dev/null 2>&1 \
		|| { echo "run: make develop"; exit 1; }
	@PROBING=1 $(PYTHON) -c "\
import shutil, sys; \
from probing import _core, VERSION; \
from probing.skills.tools import list_skills; \
from probing.skills.paths import repo_skills_dir; \
print(f'ok: probing {VERSION}, {len(list_skills())} skills, cli={shutil.which(\"probing\") or sys.executable}')" \
		|| { echo "run: make develop"; exit 1; }

frontend:
	@test -n "$$SKIP_FRONTEND_CLEAN" || rm -rf web/dist
	cd web && dx build --release
	@test -f $(DX_PUBLIC)/index.html
	cp -R $(DX_PUBLIC)/. web/dist/
	@mkdir -p web/dist/assets
	@cp -f web/assets/logo.svg web/dist/logo.svg 2>/dev/null || true
	@cp -f web/assets/tailwind.css web/dist/assets/tailwind.css
	@echo "web/dist ($$(du -sh web/dist | cut -f1))"

wheel-bundle:
	@test -f web/dist/index.html || { echo "error: run 'make frontend' first"; exit 1; }
	rm -rf python/probing/_skills python/probing/_web
	cp -R skills python/probing/_skills
	cp -R web/dist python/probing/_web

wheel: wheel-bundle nccl-profiler-lib
	maturin build $(MATURIN_FLAGS) --out dist

wheel-ci:
ifeq ($(UNAME_S),Linux)
	$(MAKE) ZIG=1 wheel
else
	$(MAKE) wheel
endif

install-wheel:
	@WH=$$(ls -1 dist/probing-*.whl 2>/dev/null | head -1); \
	test -n "$$WH" || { echo "run: make wheel"; exit 1; }; \
	$(PYTHON) -m pip install -q -U pip && \
	$(PYTHON) -m pip install --force-reinstall "$$WH" && \
	$(PYTHON) -c "import probing; from probing import _core; print('probing', probing.VERSION)"

# Linux NCCL plugin copied into python/probing/libs/ for the wheel.
ifeq ($(UNAME_S),Linux)
ifdef DEBUG
NCCL_OUT := target/debug/libprobing_nccl_profiler.so
else
NCCL_OUT := target/release/libprobing_nccl_profiler.so
endif
nccl-profiler-lib:
	cargo build -p probing-nccl-profiler $(CARGO_RELEASE)
	mkdir -p python/probing/libs
	cp $(NCCL_OUT) python/probing/libs/
else
nccl-profiler-lib:
	@:
endif

# ==============================================================================
PYTEST_WHEEL_DEPS := pytest pytest-cov coverage pyyaml websockets pandas torch ipykernel
PYTEST_WHEEL_ARGS := tests/unit tests/regression python/probing
PYTEST_WHEEL_EXTRA ?=

.PHONY: test test-rust test-rust-unit test-rust-regression test-python test-python-unit test-python-regression test-doctest test-python-wheel coverage-python-wheel
.PHONY: lint lint-python lint-rust lint-core clippy clippy-fix coverage coverage-rust coverage-python bootstrap clean docs-install docs docs-serve docs-clean

test: test-rust test-python
test-rust: test-rust-unit test-rust-regression

test-rust-unit:
	@if command -v pyenv >/dev/null 2>&1; then \
		P=$$(pyenv which python3 2>/dev/null); \
		test -n "$$P" && export PYTHON_SYS_EXECUTABLE=$$P PYO3_PYTHON=$$P; \
	fi; \
	cargo nextest run --lib --workspace --no-default-features --nff

test-rust-regression:
	@if command -v pyenv >/dev/null 2>&1; then \
		P=$$(pyenv which python3 2>/dev/null); \
		test -n "$$P" && export PYTHON_SYS_EXECUTABLE=$$P PYO3_PYTHON=$$P; \
	fi; \
	cargo nextest run --tests -p probing-rust-regression -p probing-macros --no-default-features --nff

test-python: check-dev test-python-unit test-python-regression
test-python-unit: check-dev
	PROBING=0 ${PYTEST_RUN} $(PYTEST_UNIT_ARGS)
test-python-regression: check-dev
	${PYTEST_RUN} $(PYTEST_REGRESSION_ARGS)
test-doctest:
	${PYTEST_RUN} --doctest-modules python/probing --ignore=python/probing/cli/__main__.py

test-python-wheel:
	$(PYTHON) -m pip install -q -U pip $(PYTEST_WHEEL_DEPS)
	PROBING=1 PYTHONPATH=python/ $(PYTHON) -m pytest $(PYTEST_WHEEL_EXTRA) $(PYTEST_WHEEL_ARGS)

coverage-python-wheel:
	$(MAKE) test-python-wheel PYTEST_WHEEL_EXTRA="--cov=python/probing --cov=tests --cov-report=xml:coverage.xml"

lint: lint-python lint-rust
lint-core:
	$(CLIPPY_CORE)
lint-python:
	@if $(PYTHON) -c "import ruff" 2>/dev/null; then \
		$(PYTHON) -m ruff check python/ tests/; \
	elif command -v ruff >/dev/null 2>&1; then ruff check python/ tests/; \
	else echo "install ruff"; exit 1; fi
lint-rust:
	$(CLIPPY_WORKSPACE)
	$(CLIPPY_WEB)
clippy: lint-rust
clippy-fix:
	cargo clippy --workspace --all-targets --no-default-features --fix --allow-dirty --allow-staged $(CLIPPY_DENY)
	cd web && cargo clippy --all-targets --fix --allow-dirty --allow-staged $(CLIPPY_DENY)

coverage-rust:
	cargo llvm-cov clean --workspace
	cargo llvm-cov nextest run --workspace --no-default-features --nff --lcov --output-path coverage.lcov --ignore-filename-regex '(.*/tests?/|.*/benches?/|.*/examples?/)' || true
coverage-python:
	${PYTEST_RUN} --cov=python/probing --cov=tests --cov-report=xml:coverage.xml --cov-report=term $(PYTEST_ARGS) || true
coverage: coverage-rust coverage-python
	python scripts/coverage/aggregate.py || true

bootstrap:
	uv python install 3.8 3.9 3.10 3.11 3.12 3.13

docs-install:
	@cd docs && $(MAKE) install
docs:
	@cd docs && $(MAKE) build
docs-serve:
	@cd docs && $(MAKE) serve
docs-clean:
	@cd docs && $(MAKE) clean

clean:
	rm -rf dist web/dist docs/site python/probing/_skills python/probing/_web
	cargo clean
	rm -f coverage.lcov coverage.xml coverage.json
