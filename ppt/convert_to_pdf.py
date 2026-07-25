#!/usr/bin/env python3
"""Convert the HTML PPT to PDF using WeasyPrint."""
import re
from weasyprint import HTML
from pathlib import Path

src = Path("/Users/reiase/workspace/probing/ppt/probing-presentation.html")
html = src.read_text(encoding="utf-8")

# 1. Replace slide CSS: show all slides, remove absolute positioning
html = html.replace(
    ".slide {\n    width: 1280px;\n    height: 720px;\n    background: var(--bg);\n    position: absolute;\n    display: none;\n    flex-direction: column;\n    overflow: hidden;\n    box-shadow: var(--shadow-lg);\n    border-radius: 12px;\n  }",
    ".slide {\n    width: 1280px;\n    height: 720px;\n    background: var(--bg);\n    position: relative;\n    display: flex;\n    flex-direction: column;\n    overflow: hidden;\n    box-shadow: var(--shadow-lg);\n    border-radius: 12px;\n    page-break-after: always;\n    margin-bottom: 0;\n  }"
)

# 2. Remove .slide.active rule (no longer needed)
html = html.replace(".slide.active { display: flex; }", "")

# 3. Fix body for print
html = html.replace(
    "body {\n    font-family: -apple-system, BlinkMacSystemFont, \"PingFang SC\", \"Microsoft YaHei\", \"Helvetica Neue\", Arial, sans-serif;\n    background: #0f172a;\n    color: var(--text);\n    overflow: hidden;\n    height: 100vh;\n  }",
    "body {\n    font-family: -apple-system, BlinkMacSystemFont, \"PingFang SC\", \"Microsoft YaHei\", \"Helvetica Neue\", Arial, sans-serif;\n    background: #ffffff;\n    color: var(--text);\n    overflow: visible;\n    height: auto;\n    margin: 0;\n    padding: 0;\n  }"
)

# 4. Fix slide-container for print
html = html.replace(
    ".slide-container {\n    width: 100vw;\n    height: 100vh;\n    display: flex;\n    align-items: center;\n    justify-content: center;\n    position: relative;\n  }",
    ".slide-container {\n    width: 1280px;\n    display: block;\n    position: relative;\n  }"
)

# 5. Hide nav bar
html = html.replace(
    ".nav-bar {",
    ".nav-bar { display: none !important;"
)

# 6. Hide progress bar
html = html.replace(
    ".progress-container {",
    ".progress-container { display: none !important;"
)

# 7. Fix gradient text (weasyprint doesn't support -webkit-background-clip: text)
html = html.replace(
    "background: linear-gradient(135deg, #fff 0%, #93c5fd 100%);\n    -webkit-background-clip: text;\n    -webkit-text-fill-color: transparent;",
    "color: #93c5fd;"
)

# 8. Fix title slide gradient text
html = re.sub(
    r"background:\s*linear-gradient\(135deg,\s*#fff[^;]*\);\s*-webkit-background-clip:\s*text;\s*-webkit-text-fill-color:\s*transparent;",
    "color: #ffffff;",
    html
)

# 9. Remove backdrop-filter (not supported by weasyprint)
html = re.sub(r"backdrop-filter:\s*blur\([^)]+\);", "", html)

# 10. Remove the JavaScript (not needed for print)
# Find and remove the <script> section
html = re.sub(r'<script>.*?</script>', '', html, flags=re.DOTALL)

# 11. Add @page rule for PDF page size
page_css = """
  @page {
    size: 1280px 720px;
    margin: 0;
  }
  @page :first {
    margin: 0;
  }
"""

# Insert @page rule before the closing </style>
html = html.replace("</style>", page_css + "</style>")

# 12. Remove slide-indicator visibility for print (optional - keep for reference)
# Actually keep them, they show page numbers

# Write the print-friendly HTML for reference
print_path = Path("/Users/reiase/workspace/probing/ppt/probing-print.html")
print_path.write_text(html, encoding="utf-8")
print(f"Print HTML saved to {print_path}")

# Convert to PDF
output_pdf = Path("/Users/reiase/workspace/probing/ppt/probing-presentation.pdf")
print("Converting to PDF...")
HTML(string=html, base_url=str(src.parent)).write_pdf(str(output_pdf))
print(f"PDF saved to {output_pdf}")
print(f"File size: {output_pdf.stat().st_size / 1024 / 1024:.1f} MB")
