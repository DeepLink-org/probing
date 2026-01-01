# Makefile for Probing Documentation (MkDocs)

.PHONY: help install serve build clean deploy

# Default target
help:
	@echo "Probing Documentation Build System"
	@echo ""
	@echo "Usage:"
	@echo "  make install   Install documentation dependencies"
	@echo "  make serve     Start live preview server with auto-reload"
	@echo "  make build     Build static documentation site"
	@echo "  make clean     Remove built documentation"
	@echo "  make deploy    Deploy to GitHub Pages"

# Install dependencies
install:
	@echo "Installing documentation dependencies..."
	pip install mkdocs mkdocs-material mkdocs-material-extensions mkdocstrings mkdocstrings-python

# Live preview server
serve:
	@echo "Starting live preview server..."
	@echo "Open http://127.0.0.1:8000 in your browser"
	mkdocs serve --dev-addr 0.0.0.0:8000

# Build static site
build:
	@echo "Building documentation..."
	mkdocs build

# Clean build artifacts
clean:
	@echo "Cleaning build artifacts..."
	rm -rf site/

# Deploy to GitHub Pages
deploy:
	@echo "Deploying to GitHub Pages..."
	mkdocs gh-deploy --force
