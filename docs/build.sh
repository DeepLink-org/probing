#!/bin/bash
# Quick build script

set -e

echo "📚 Building Probing documentation..."

# Check dependencies
if ! python3 -c "import sphinx" 2>/dev/null; then
    echo "⚠️  Sphinx not detected, installing dependencies..."
    pip install -r requirements_doc.txt
fi

# Build documentation
echo "🔨 Starting build..."
make html

echo "✅ Build complete!"
echo "📖 Documentation location: _build/html/index.html"
echo ""
echo "💡 Tip: Use the following commands to open documentation in browser:"
echo "   open _build/html/index.html  # macOS"
echo "   xdg-open _build/html/index.html  # Linux"
