"""REPL magics contributed by probing-acme."""

from IPython.core.magic import Magics, line_magic, magics_class


@magics_class
class AcmeMagic(Magics):
    """Vendor-specific REPL helpers (demo: ``%acme``)."""

    @line_magic
    def acme(self, line: str) -> str:
        """Echo a line — replace with real vendor diagnostics."""
        text = line.strip() or "(no args)"
        return f"[probing-acme] {text}"
