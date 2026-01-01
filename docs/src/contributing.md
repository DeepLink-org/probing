# Contributing Guide

Thank you for your interest in contributing to Probing! This guide will help you get started.

## Getting Started

### Prerequisites

- Python 3.9+
- Rust (latest stable)
- maturin (for building Python extensions)

### Development Setup

1. Clone the repository:

```bash
git clone https://github.com/DeepLink-org/probing.git
cd probing
```

2. Create a virtual environment:

```bash
python -m venv .venv
source .venv/bin/activate  # Linux/macOS
# or
.venv\Scripts\activate  # Windows
```

3. Install development dependencies:

```bash
pip install -e ".[dev]"
```

4. Build the Rust extension:

```bash
maturin develop
```

## Development Workflow

### Running Tests

```bash
# Run all tests
make test

# Run specific test
pytest tests/test_specific.py -v

# Run with coverage
make coverage
```

### Code Style

We use the following tools for code quality:

**Python:**

- `ruff` for linting and formatting
- `mypy` for type checking

```bash
# Format code
ruff format .

# Check linting
ruff check .

# Type check
mypy python/probing
```

**Rust:**

- `rustfmt` for formatting
- `clippy` for linting

```bash
# Format
cargo fmt

# Lint
cargo clippy --all --tests --benches -- -D warnings
```

### Building Documentation

```bash
cd docs
make install  # Install dependencies
make serve    # Preview at http://127.0.0.1:8000
```

## Submitting Changes

### Pull Request Process

1. Fork the repository
2. Create a feature branch:

```bash
git checkout -b feature/your-feature-name
```

3. Make your changes
4. Run tests and linting:

```bash
make test
make lint
```

5. Commit with a descriptive message:

```bash
git commit -m "feat: add new feature description"
```

6. Push and create a pull request

### Commit Message Format

We follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation changes
- `style:` - Code style changes (formatting)
- `refactor:` - Code refactoring
- `test:` - Test changes
- `chore:` - Build/tooling changes

### Code Review

All submissions require code review. Please:

- Keep PRs focused on a single change
- Add tests for new functionality
- Update documentation as needed
- Respond to review feedback promptly

## Project Structure

```
probing/
├── python/             # Python source code
│   └── probing/        # Main Python package
├── probing/            # Rust crates
│   ├── core/           # Core functionality
│   ├── server/         # HTTP server
│   ├── extensions/     # Python/PyTorch extensions
│   └── cli/            # Command-line interface
├── tests/              # Python tests
├── docs/               # Documentation
└── examples/           # Usage examples
```

## Areas for Contribution

### Good First Issues

Look for issues labeled `good-first-issue` on GitHub. These are suitable for newcomers.

### Documentation

- Improve existing documentation
- Add more examples
- Translate documentation

### Testing

- Add test coverage
- Write integration tests
- Performance benchmarks

### Features

- Check the roadmap and open issues
- Discuss large changes before implementing

## Getting Help

- **GitHub Issues**: For bugs and feature requests
- **Discussions**: For questions and ideas

## Code of Conduct

Please be respectful and constructive in all interactions. We are committed to providing a welcoming environment for everyone.

## License

By contributing, you agree that your contributions will be licensed under the project's Apache 2.0 license.
