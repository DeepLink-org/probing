# Probing Documentation System

This directory contains the complete documentation for the Probing project, built using [Sphinx](https://www.sphinx-doc.org/).

## Quick Start

### Install Dependencies

```bash
pip install -r requirements_doc.txt
```

### Build Documentation

```bash
# Build HTML documentation
make html

# Or use sphinx-build directly
sphinx-build -b html . _build/html
```

After building, the documentation will be generated in the `_build/html/` directory. Open `_build/html/index.html` in your browser to view the documentation.

### Live Preview (Development Mode)

Using `sphinx-autobuild` can automatically detect file changes and rebuild the documentation while refreshing the browser:

```bash
# Method 1: Start from project root (recommended)
make docs-serve

# Method 2: Use convenience script in docs directory
cd docs && ./serve.sh

# Method 3: Use Makefile in docs directory
cd docs && make serve

# Method 4: Use sphinx-autobuild directly
cd docs && sphinx-autobuild . _build/html --host 0.0.0.0 --port 8000 --open-browser
```

After starting, the terminal will display the local access address (usually `http://127.0.0.1:8000`). Open it in your browser to view the documentation in real-time. After modifying any documentation files, the documentation will automatically rebuild and refresh the browser.

**Common Options:**
- `--host 0.0.0.0` - Allow access from other devices (on local network)
- `--port 8000` - Specify port number (default 8000)
- `--open-browser` - Automatically open browser

## Documentation Structure

```
docs/
├── conf.py              # Sphinx configuration file
├── index.rst            # Main documentation index
├── requirements_doc.txt # Python dependencies (documentation build only)
├── Makefile            # Build script (Unix)
├── getting-started/    # Getting started guide
├── user-guide/         # User guide
├── design/             # System design
├── advanced/           # Advanced topics
└── development/        # Development related
```

## Writing Documentation

### Markdown Support

The documentation uses [MyST Parser](https://myst-parser.readthedocs.io/) to support Markdown format. You can write documentation directly in Markdown, with support for the following extensions:

- Code block fences (using `:colon_fence:`)
- Definition lists
- Task lists
- Math formulas (using `$` or `$$`)
- Substitutions and references
- And more

### Adding New Documentation

1. Create a `.md` file in the appropriate directory
2. Add the corresponding entry in `index.rst`

For example, to add a new user guide:

```rst
.. toctree::
   :maxdepth: 2
   :caption: User Guide

   user-guide/sql-analytics
   user-guide/memory-analysis
   user-guide/new-guide  # Newly added documentation
```

### Documentation Format Examples

```markdown
# Title

This is a paragraph of regular text.

## Code Example

```python
def hello():
    print("Hello, World!")
```

## Admonitions

```{note}
This is a note.
```

```{warning}
This is a warning.
```

## Math Formulas

Inline formula: $E = mc^2$

Block formula:

$$
\int_{-\infty}^{\infty} e^{-x^2} dx = \sqrt{\pi}
$$
```

## Build Options

### HTML Output

```bash
make html
```

### PDF Output (requires LaTeX)

```bash
make latexpdf
```

### Other Formats

```bash
make help  # View all available formats
```

## Theme and Styling

The documentation uses the `sphinx_rtd_theme` (Read the Docs theme). You can modify the theme configuration in `conf.py`.

## Troubleshooting

### Common Issues

1. **Module not found error**
   - Ensure all dependencies are installed: `pip install -r requirements_doc.txt`

2. **Build failure**
   - Check for syntax errors in the documentation
   - Ensure all referenced files exist

3. **Character encoding issues**
   - Ensure documentation uses UTF-8 encoding
   - Check language settings in `conf.py`

## More Information

- [Sphinx Documentation](https://www.sphinx-doc.org/)
- [MyST Parser Documentation](https://myst-parser.readthedocs.io/)
- [Read the Docs Theme Documentation](https://sphinx-rtd-theme.readthedocs.io/)
