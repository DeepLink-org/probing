# Probing Documentation

This directory contains the documentation for Probing, built with [MkDocs](https://www.mkdocs.org/) and the [Material theme](https://squidfunk.github.io/mkdocs-material/).

## Quick Start

### Install Dependencies

```bash
# Using pip
pip install mkdocs mkdocs-material mkdocs-material-extensions mkdocstrings mkdocstrings-python

# Or using uv
uv pip install -r pyproject.toml
```

### Build and Serve

```bash
# Start live preview server
make serve

# Or directly with mkdocs
mkdocs serve
```

Then open http://localhost:8000 in your browser.

### Build Static Site

```bash
make build
```

The built site will be in the `site/` directory.

## Directory Structure

```
docs/
├── mkdocs.yml           # MkDocs configuration
├── pyproject.toml       # Python dependencies
├── Makefile             # Build commands
├── overrides/           # Custom templates
│   └── home.html        # Homepage template
└── src/                 # Documentation source
    ├── index.md         # Homepage content
    ├── installation.md  # Installation guide
    ├── quickstart.md    # Quick start guide
    ├── guide/           # User guide
    ├── design/          # Design documents
    ├── examples/        # Example workflows
    ├── api-reference.md # API reference
    └── assets/          # Static assets
        └── stylesheets/ # CSS styles
```

## Writing Documentation

- All documentation is written in Markdown
- Use [Material for MkDocs](https://squidfunk.github.io/mkdocs-material/reference/) extensions
- Admonitions, code blocks with copy button, and Mermaid diagrams are supported

### Example Admonition

```markdown
!!! note "Title"
    This is a note admonition.

!!! warning
    This is a warning.
```

### Example Mermaid Diagram

````markdown
```mermaid
graph LR
    A[Start] --> B[Process]
    B --> C[End]
```
````

## Deployment

To deploy to GitHub Pages:

```bash
make deploy
```
