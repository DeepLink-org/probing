# Probing Makefile
#
#   develop          → maturin develop (Rust/Python daily loop)
#   frontend         → dx bundle → python/probing/bundled_web/public (web/dist symlink)
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
	MATURIN_FEATURES := extension-module,gpu,gpu-cuda,kmsg
else
	MATURIN_FEATURES := extension-module,gpu,kmsg
endif

MATURIN_FLAGS := $(MATURIN_RELEASE) --features $(MATURIN_FEATURES)
BUNDLED_WEB_PUBLIC := python/probing/bundled_web/public

ifdef ZIG
ifdef TARGET
	MATURIN_FLAGS += --zig --target $(TARGET)
endif
endif

# Project .venv is the maturin develop target (maturin requires a virtualenv).
# Bootstrap from pyenv .python-version when present; --system-site-packages
# keeps torch etc. from the base interpreter visible inside .venv.
# Override: make develop PYTHON=/path/to/python  (also used as venv bootstrap)
# CI pins PYTHON to $(CURDIR)/.venv/bin/python after make venv.
PYTHON_BOOTSTRAP ?= $(shell \
	if [ -f .python-version ] && command -v pyenv >/dev/null 2>&1; then \
		PV=$$(tr -d '[:space:]' < .python-version); \
		PY=$$(PYENV_VERSION="$$PV" pyenv which python 2>/dev/null); \
		[ -n "$$PY" ] && [ -x "$$PY" ] && echo "$$PY" && exit 0; \
	fi; \
	command -v python3 2>/dev/null || echo python3)
ifeq ($(origin PYTHON),undefined)
PYTHON := $(if $(wildcard .venv/bin/python),.venv/bin/python,$(PYTHON_BOOTSTRAP))
endif
PYTHON_ABS := $(shell "$(PYTHON)" -c 'import sys; print(sys.executable)' 2>/dev/null)
ifeq ($(PYTHON_ABS),)
PYTHON_ABS := $(CURDIR)/$(PYTHON)
endif
VENV_PYTHON := .venv/bin/python
BUILD_PY_DEPS := build wheel toml maturin
DEV_PTH := python/probing/dev_pth.py
DEV_PY_DEPS := pyyaml pytest pytest-cov coverage ipython ipykernel ruff

PYTEST_RUN := PROBING=1 $(VENV_PYTHON) -m pytest
BENCH_SCRIPT := examples/bench_instrumentation.py
BENCH_RUN := PROBING=1 $(VENV_PYTHON) $(BENCH_SCRIPT)
PYTEST_UNIT_ARGS := tests/unit
PYTEST_REGRESSION_ARGS := tests/regression
PYTEST_ARGS := $(PYTEST_UNIT_ARGS) $(PYTEST_REGRESSION_ARGS)

CLIPPY_DENY := -- -D warnings
CLIPPY_WORKSPACE := cargo clippy --workspace --all-targets --no-default-features $(CLIPPY_DENY)
CLIPPY_CORE := cargo clippy -p probing-core --all-targets --no-default-features $(CLIPPY_DENY)
CLIPPY_WEB := cd web && cargo clippy --all-targets $(CLIPPY_DENY)

FMT_WORKSPACE := cargo fmt --all
FMT_WEB := cd web && cargo fmt --all
FMT_ALL_CFGS := ./scripts/fmt-all-cfgs.sh

# ==============================================================================
.PHONY: help
help:
	@echo "Probing — make [target]    (see docs/src/contributing.md)"
	@echo ""
	@echo "  develop / dev     Bootstrap: _core, CLI, pytest, site hook"
	@echo "  core              Rebuild probing._core after Rust edits"
	@echo "  frontend          Build UI into python/probing/bundled_web (dx bundle)"
	@echo "  wheel             Build dist/*.whl (needs bundled_web; bundles skills + UI)"
	@echo "  wheel-ci          alias for wheel (native build; PyPI uses maturin-action + zig)"
	@echo "  install-wheel     pip install dist/probing-*.whl"
	@echo "  venv              create/sync project .venv (from .python-version when set)"
	@echo "  test              Rust + editable Python (daily dev)"
	@echo "  test-wheel        Installed-wheel Python tests (needs dist/*.whl)"
	@echo "  test-ci           test + test-wheel (matches CI Python gate)"
	@echo "  validate-skills   Validate bundled skills catalog (needs probing._core)"
	@echo "  supply-chain      cargo-deny + uv lock check (matches CI)"
	@echo "  lint              ruff + clippy + mkdocs --strict"
	@echo "  fmt               rustfmt workspace + web + ruff format (passes lint-python)"
	@echo "  fmt-all-cfgs      fmt + extra targets (windows/linux/arch cfg branches)"
	@echo "  check-dev         Quick env sanity check"
	@echo "  bench             Instrumentation overhead (span / TorchProbe / train)"
	@echo "  bench-quick       Same as bench, fewer iterations"
	@echo "  soak              Long-run soak (~10 min synthetic ImageNet + assert)"
	@echo "  soak-quick        Short soak smoke (~60s)"
	@echo "  clean             Remove build artifacts"
	@echo ""
	@echo "Release gate: make frontend && make wheel && make test-ci"
	@echo "Env: DEBUG=1  ZIG=1 TARGET=<triplet>"

# ==============================================================================
.PHONY: setup install-dev-python-deps
setup:
	@if command -v pip >/dev/null 2>&1; then pip install pre-commit; fi
	@if command -v pre-commit >/dev/null 2>&1; then pre-commit install; fi

install-dev-python-deps: venv
	@if command -v uv >/dev/null 2>&1 && [ -f uv.lock ]; then \
		uv sync --frozen --no-install-project --group dev; \
	elif [ -x .venv/bin/ruff ] && $(VENV_PYTHON) -c "import pytest, yaml" 2>/dev/null; then \
		echo "  dev Python deps OK"; \
	else \
		$(VENV_PYTHON) -m pip install -q -U pip $(DEV_PY_DEPS); \
	fi

# ==============================================================================
.PHONY: core develop dev check-dev frontend wheel wheel-ci install-wheel wheel-bundle nccl-profiler-lib nccl-profiler-bench hccl-shim-lib venv venv-wheel install-build-deps install-wheel-test-deps

venv:
	@BOOT="$(PYTHON_BOOTSTRAP)"; \
	if [ -x "$(VENV_PYTHON)" ]; then \
		BV=$$("$$BOOT" -c 'import sys; print(".".join(map(str, sys.version_info[:3])))' 2>/dev/null); \
		VV=$$("$(VENV_PYTHON)" -c 'import sys; print(".".join(map(str, sys.version_info[:3])))' 2>/dev/null); \
		if [ -n "$$BV" ] && [ "$$VV" != "$$BV" ]; then \
			echo "  recreating .venv (Python $$VV -> $$BV; --system-site-packages)"; \
			rm -rf .venv; \
		fi; \
	fi; \
	test -x "$(VENV_PYTHON)" || "$$BOOT" -m venv --system-site-packages .venv; \
	"$(VENV_PYTHON)" -m pip install -q -U pip

# Backward-compatible alias (CI/docs may still reference venv-wheel).
venv-wheel: venv

install-build-deps: venv
	@if command -v uv >/dev/null 2>&1 && [ -f uv.lock ]; then \
		uv sync --frozen --no-install-project --group build; \
	else \
		$(VENV_PYTHON) -m pip install -q -U pip $(BUILD_PY_DEPS); \
	fi

install-wheel-test-deps: venv
	@if command -v uv >/dev/null 2>&1 && [ -f uv.lock ]; then \
		uv sync --frozen --no-install-project --group wheel-test; \
	else \
		$(VENV_PYTHON) -m pip install -q -U pip $(PYTEST_WHEEL_DEPS); \
	fi

# CI: UV_SYNC_GROUPS=build,dev,wheel-test make sync-uv-groups
sync-uv-groups: venv
	@test -f uv.lock || { echo "error: missing uv.lock"; exit 1; }
	@test -n "$(UV_SYNC_GROUPS)" || { echo "error: set UV_SYNC_GROUPS (comma-separated)"; exit 1; }
	@args=""; \
	for g in $$(echo "$(UV_SYNC_GROUPS)" | tr ',' ' '); do \
		test -n "$$g" || continue; \
		args="$$args --group $$g"; \
	done; \
	uv sync --frozen --no-install-project $$args

core: venv nccl-profiler-lib hccl-shim-lib
	$(VENV_PYTHON) -m maturin develop $(MATURIN_FLAGS)

develop: venv install-build-deps core install-dev-python-deps
	@echo "  Python: $$($(VENV_PYTHON) -c 'import sys; print(sys.executable)')"
	$(VENV_PYTHON) $(DEV_PTH) install
	@$(MAKE) --no-print-directory check-dev

dev: develop

check-dev: venv
	@test -L skills && test -f skills/catalog.yaml \
		|| { echo "error: skills/ must be a symlink to python/probing/bundled_skills"; exit 1; }
	@ROOT=$$(cd skills && pwd -P); PKG=$$(cd python/probing/bundled_skills && pwd -P); \
	test "$$ROOT" = "$$PKG" \
		|| { echo "error: skills symlink target mismatch"; exit 1; }
	@$(VENV_PYTHON) $(DEV_PTH) status >/dev/null 2>&1 \
		|| { echo "run: make develop"; exit 1; }
	@PROBING=1 $(VENV_PYTHON) -c "\
import shutil, sys; \
from probing import _core, VERSION; \
from probing.skills.tools import list_skills; \
from probing.skills.paths import repo_skills_dir; \
print(f'ok: probing {VERSION}, {len(list_skills())} skills, cli={shutil.which(\"probing\") or sys.executable}')" \
		|| { echo "run: make develop"; exit 1; }

.PHONY: bench bench-quick soak soak-quick
bench: check-dev
	$(BENCH_RUN)

bench-quick: check-dev
	$(BENCH_RUN) --quick

soak: check-dev
	@test -x examples/run_soak.sh || chmod +x examples/run_soak.sh
	DURATION_SEC=600 PYTHON=$(VENV_PYTHON) PROBING=1 ./examples/run_soak.sh

soak-quick: check-dev
	@test -x examples/run_soak.sh || chmod +x examples/run_soak.sh
	DURATION_SEC=60 MAX_STEPS=8 PYTHON=$(VENV_PYTHON) PROBING=1 ./examples/run_soak.sh

frontend:
	@test -n "$$SKIP_FRONTEND_CLEAN" || rm -rf python/probing/bundled_web
	cd web && dx bundle --release
	@test -f $(BUNDLED_WEB_PUBLIC)/index.html
	@js=$$(grep -oE 'web-dxh[^" ]+\.js' $(BUNDLED_WEB_PUBLIC)/index.html | head -1); \
	for f in $(BUNDLED_WEB_PUBLIC)/assets/web-dxh*.js; do \
	  test "$$f" = "$(BUNDLED_WEB_PUBLIC)/assets/$$js" || rm -f "$$f"; \
	done
	@mkdir -p $(BUNDLED_WEB_PUBLIC)/assets
	@cp -f web/assets/logo.svg $(BUNDLED_WEB_PUBLIC)/logo.svg 2>/dev/null || true
	@cp -f web/assets/logo.svg $(BUNDLED_WEB_PUBLIC)/assets/logo.svg 2>/dev/null || true
	@cp -f web/assets/tailwind.css $(BUNDLED_WEB_PUBLIC)/assets/tailwind.css
	@rm -rf web/dist
	@ln -s python/probing/bundled_web/public web/dist
	@echo "$(BUNDLED_WEB_PUBLIC) ($$(du -sh $(BUNDLED_WEB_PUBLIC) | cut -f1))"

wheel-bundle:
	@test -f $(BUNDLED_WEB_PUBLIC)/index.html || { echo "error: run 'make frontend' first"; exit 1; }
	@test -f python/probing/bundled_skills/catalog.yaml \
		|| { echo "error: missing python/probing/bundled_skills/catalog.yaml"; exit 1; }

wheel: install-build-deps wheel-bundle nccl-profiler-lib hccl-shim-lib
	$(PYTHON) -m maturin build $(MATURIN_FLAGS) --out dist

wheel-ci:
	$(MAKE) wheel

install-wheel:
	@WH=$$(ls -1 dist/probing-*.whl 2>/dev/null | head -1); \
	test -n "$$WH" || { echo "run: make wheel"; exit 1; }; \
	$(PYTHON) -m pip install -q -U pip && \
	$(PYTHON) -m pip install --force-reinstall "$$WH" && \
	PROBING=0 $(PYTHON) -c "\
import probing; from probing import _core; from pathlib import Path; \
root = Path(probing.__file__).resolve().parent; \
assert (root / 'bundled_skills' / 'catalog.yaml').is_file(), f'missing bundled skills under {root}'; \
assert (root / 'bundled_web' / 'public' / 'index.html').is_file() or (root / 'bundled_web' / 'index.html').is_file(), f'missing bundled web UI under {root}'; \
print('probing', probing.VERSION)"

# Linux NCCL plugin copied into python/probing/libs/ for the wheel.
ifeq ($(UNAME_S),Linux)
ifdef DEBUG
NCCL_OUT := target/debug/libprobing_nccl_profiler.so
else
NCCL_OUT := target/release/libprobing_nccl_profiler.so
endif
nccl-profiler-lib:
	cargo build -p probing-nccl-profiler-cdylib $(CARGO_RELEASE)
	mkdir -p python/probing/libs
	cp $(NCCL_OUT) python/probing/libs/

nccl-profiler-bench:
	cargo bench -p probing-nccl-profiler --bench callback_path
else
nccl-profiler-lib:
	@:
nccl-profiler-bench:
	cargo bench -p probing-nccl-profiler --bench callback_path
endif

# Linux HCCL libprofapi.so shim → python/probing/shim/hccl/
ifeq ($(UNAME_S),Linux)
ifdef DEBUG
HCCL_SHIM_OUT := target/debug/libprofapi.so
else
HCCL_SHIM_OUT := target/release/libprofapi.so
endif
hccl-shim-lib:
	cargo build -p probing-hccl-profapi $(CARGO_RELEASE)
	mkdir -p python/probing/shim/hccl
	cp $(HCCL_SHIM_OUT) python/probing/shim/hccl/
else
hccl-shim-lib:
	@:
endif

# ==============================================================================
PYTEST_WHEEL_DEPS := pytest pytest-cov coverage pyyaml websockets pandas torch ipykernel
# Installed wheel only — do not pass python/probing (conflicts with site-packages).
PYTEST_WHEEL_ARGS := tests/unit tests/regression
# Clear repo pythonpath; sibling unit/regression conftest.py need importlib mode.
# Override addopts to drop --doctest-modules (wheel uses site-packages, not python/).
PYTEST_WHEEL_FLAGS := --import-mode=importlib -o pythonpath= -o "addopts=--verbose --color=yes --durations=10 --strict-markers"
PYTEST_WHEEL_EXTRA ?=

.PHONY: test test-wheel test-ci validate-skills test-rust test-rust-unit test-rust-regression test-python test-python-unit test-python-regression test-doctest test-python-wheel coverage-python-wheel bench bench-quick
.PHONY: fmt fmt-check fmt-all-cfgs fmt-all-cfgs-check lint lint-python lint-rust lint-docs lint-core clippy clippy-fix coverage coverage-rust coverage-python bootstrap clean docs-install docs docs-serve docs-clean supply-chain

test: test-rust test-python
test-wheel: install-wheel test-python-wheel
test-ci: test test-wheel
validate-skills:
	@test -L skills && test -f skills/catalog.yaml \
		|| { echo "error: skills/ must be a symlink to python/probing/bundled_skills"; exit 1; }
	PROBING=0 $(PYTHON) -m probing.skills validate
test-rust: test-rust-unit test-rust-regression

test-rust-unit:
	@export PYTHON_SYS_EXECUTABLE=$(PYTHON_ABS) PYO3_PYTHON=$(PYTHON_ABS); \
	cargo nextest run --lib --workspace --no-default-features --nff

test-rust-regression:
	@export PYTHON_SYS_EXECUTABLE=$(PYTHON_ABS) PYO3_PYTHON=$(PYTHON_ABS); \
	cargo nextest run --tests -p probing-rust-regression -p probing-macros --no-default-features --nff

test-python: check-dev test-python-unit test-python-regression
test-python-unit: check-dev
	PROBING=0 ${PYTEST_RUN} $(PYTEST_UNIT_ARGS)
test-python-regression: check-dev
	${PYTEST_RUN} $(PYTEST_REGRESSION_ARGS)
test-doctest:
	${PYTEST_RUN} --doctest-modules python/probing --ignore=python/probing/cli/__main__.py

test-python-wheel: install-wheel-test-deps
	PROBING=1 PROBING_VM_TRACER=0 $(PYTHON) -m pytest $(PYTEST_WHEEL_FLAGS) $(PYTEST_WHEEL_EXTRA) $(PYTEST_WHEEL_ARGS)

coverage-python-wheel:
	$(MAKE) test-python-wheel PYTEST_WHEEL_EXTRA="--cov=probing --cov=tests --cov-report=xml:coverage.xml"

FMT_PYTHON := @if [ -x .venv/bin/ruff ]; then \
		.venv/bin/ruff check --fix python/ tests/; \
		.venv/bin/ruff format python/ tests/; \
	elif command -v ruff >/dev/null 2>&1; then \
		ruff check --fix python/ tests/; \
		ruff format python/ tests/; \
	else \
		echo "install ruff: make install-dev-python-deps  (or: $(VENV_PYTHON) -m pip install ruff)"; exit 1; \
	fi

lint: lint-python lint-rust lint-docs
fmt:
	$(FMT_WORKSPACE)
	$(FMT_WEB)
	$(FMT_PYTHON)
fmt-check:
	$(FMT_WORKSPACE) -- --check
	$(FMT_WEB) -- --check
	@if [ -x .venv/bin/ruff ]; then \
		.venv/bin/ruff check python/ tests/ && \
		.venv/bin/ruff format --check python/ tests/; \
	elif command -v ruff >/dev/null 2>&1; then \
		ruff check python/ tests/ && \
		ruff format --check python/ tests/; \
	else \
		echo "install ruff: make install-dev-python-deps  (or: $(VENV_PYTHON) -m pip install ruff)"; exit 1; \
	fi
fmt-all-cfgs:
	@chmod +x scripts/fmt-all-cfgs.sh
	$(FMT_ALL_CFGS)
fmt-all-cfgs-check:
	@chmod +x scripts/fmt-all-cfgs.sh
	$(FMT_ALL_CFGS) --check
lint-core:
	$(CLIPPY_CORE)
lint-python:
	@if [ -x .venv/bin/ruff ]; then \
		.venv/bin/ruff check python/ tests/ && \
		.venv/bin/ruff format --check python/ tests/; \
	elif command -v ruff >/dev/null 2>&1; then \
		ruff check python/ tests/ && \
		ruff format --check python/ tests/; \
	else \
		echo "install ruff: make install-dev-python-deps  (or: $(VENV_PYTHON) -m pip install ruff)"; exit 1; \
	fi
lint-rust:
	$(CLIPPY_WORKSPACE)
	$(CLIPPY_WEB)
lint-docs: docs-install
	@$(PYTHON_ABS) -m mkdocs build --strict -f docs/mkdocs.yml
clippy: lint-rust
clippy-fix:
	cargo clippy --workspace --all-targets --no-default-features --fix --allow-dirty --allow-staged $(CLIPPY_DENY)
	cd web && cargo clippy --all-targets --fix --allow-dirty --allow-staged $(CLIPPY_DENY)

coverage-rust:
	cargo llvm-cov clean --workspace
	cargo llvm-cov nextest --workspace --no-default-features --nff \
		--exclude probing-hccl-profapi --exclude probing-nccl-profiler-cdylib \
		--lcov --output-path coverage.lcov --ignore-filename-regex '(.*/tests?/|.*/benches?/|.*/examples?/)'
coverage-python: install-dev-python-deps check-dev
	${PYTEST_RUN} --cov=python/probing --cov=tests --cov-report=xml:coverage.xml --cov-report=term $(PYTEST_ARGS)
coverage: coverage-rust coverage-python

bootstrap:
	uv python install 3.8 3.9 3.10 3.11 3.12 3.13

supply-chain:
	@command -v cargo-deny >/dev/null 2>&1 || { echo "install: cargo install cargo-deny --locked"; exit 1; }
	cargo deny check
	@if command -v uv >/dev/null 2>&1 && [ -f uv.lock ]; then uv lock --check; fi
	@cd web && cargo deny --config ../deny.toml check

docs-install:
	@cd docs && $(MAKE) install PYTHON=$(PYTHON_ABS)
docs:
	@cd docs && $(MAKE) build PYTHON=$(PYTHON_ABS)
docs-serve:
	@cd docs && $(MAKE) serve PYTHON=$(PYTHON_ABS)
docs-clean:
	@cd docs && $(MAKE) clean

clean:
	rm -rf dist docs/site python/probing/bundled_web web/dist
	cargo clean
	rm -f coverage.lcov coverage.xml coverage.json
