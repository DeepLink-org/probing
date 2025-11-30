# Minimal makefile for Sphinx documentation
#

# You can set these variables from the command line, and also
# from the environment for the first two.
SPHINXOPTS    ?=
SPHINXBUILD  ?= sphinx-build
SOURCEDIR    = .
BUILDDIR     = _build

# Put it first so that "make" without argument is like "make help".
help:
	@$(SPHINXBUILD) -M help "$(SOURCEDIR)" "$(BUILDDIR)" $(SPHINXOPTS) $(O)
	@echo ""
	@echo "Additional targets:"
	@echo "  serve      Start live preview server with auto-reload"

.PHONY: help Makefile serve

# Live preview server
serve:
	@echo "Starting live preview server..."
	@echo "Open http://127.0.0.1:8000 in your browser"
	@sphinx-autobuild -b html "$(SOURCEDIR)" "$(BUILDDIR)/html" --host 0.0.0.0 --port 8000 --open-browser

# Catch-all target: route all unknown targets to Sphinx using the new
# "make mode" option.  $(O) is meant as a shortcut for $(SPHINXOPTS).
%: Makefile
	@$(SPHINXBUILD) -M $@ "$(SOURCEDIR)" "$(BUILDDIR)" $(SPHINXOPTS) $(O)
