# Makefile for Probing Documentation (MkDocs)

PYTHON ?= $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/../.venv/bin/python)

.PHONY: help install serve build lint clean deploy

# Default target
help:
	@echo "Probing Documentation Build System"
	@echo ""
	@echo "Usage:"
	@echo "  make install   Install documentation dependencies"
	@echo "  make serve     Start live preview server with auto-reload"
	@echo "  make build     Build static documentation site"
	@echo "  make lint      Check links and build (--strict)"
	@echo "  make clean     Remove built documentation"
	@echo "  make deploy    Deploy to GitHub Pages"

# Install dependencies
install:
	@echo "Installing documentation dependencies..."
	$(PYTHON) -m pip install -r requirements.txt

# Live preview server
serve:
	@echo "Starting live preview server..."
	@echo "Open http://127.0.0.1:8000 in your browser"
	mkdocs serve --dev-addr 0.0.0.0:8000

# Build static site
build:
	@echo "Building documentation..."
	mkdocs build

# Strict build (broken links fail) — used by root `make lint`
lint:
	@echo "Linting documentation (mkdocs --strict)..."
	$(PYTHON) -m mkdocs build --strict

# Clean build artifacts
clean:
	@echo "Cleaning build artifacts..."
	rm -rf site/

# Deploy to GitHub Pages
deploy:
	@echo "Deploying to GitHub Pages..."
	mkdocs gh-deploy --force
