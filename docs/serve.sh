#!/bin/bash
# Live preview documentation server

set -e

echo "🚀 Starting Probing documentation live preview server..."
echo ""

# Check if sphinx-autobuild is installed
if ! python3 -c "import sphinx_autobuild" 2>/dev/null; then
    echo "⚠️  sphinx-autobuild not detected, installing..."
    pip install -q sphinx-autobuild
fi

# Start server
echo "📖 Documentation will open automatically in browser"
echo "🌐 Access URL: http://127.0.0.1:8000"
echo "💡 Documentation will auto-rebuild and refresh browser when files are modified"
echo "🛑 Press Ctrl+C to stop server"
echo ""

sphinx-autobuild \
    -b html \
    . \
    _build/html \
    --host 0.0.0.0 \
    --port 8000 \
    --open-browser \
    --watch . \
    --ignore "_build/*" \
    --ignore "*.pyc" \
    --ignore ".git/*"
