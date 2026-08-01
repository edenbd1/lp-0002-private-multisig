#!/usr/bin/env bash
# Render VIDEO_SCRIPT.md to a printable PDF you can read from while recording.
#
# markdown -> standalone HTML (pandoc) -> PDF (headless Chrome). Chrome rather
# than LaTeX because the script is full of emoji and the emoji are load-bearing:
# 🎬 means "do this" and 💬 means "read this out loud", and a PDF that drops them
# is useless at 2am with a microphone in front of you.
#
#   ./scripts/video-script-pdf.sh
#
# Output: VIDEO_SCRIPT.pdf

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SRC=VIDEO_SCRIPT.md
OUT=VIDEO_SCRIPT.pdf
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

command -v pandoc >/dev/null || { echo "pandoc not found: brew install pandoc" >&2; exit 1; }
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
[ -x "$CHROME" ] || { echo "Google Chrome not found at $CHROME" >&2; exit 1; }

cat > "$TMP/style.css" <<'CSS'
@page { size: A4; margin: 16mm 14mm; }
body {
  font: 11.5pt/1.55 -apple-system, "Helvetica Neue", Arial, sans-serif;
  color: #17181c; max-width: none;
}
h1 {
  font-size: 17pt; margin: 26px 0 10px; padding-top: 10px;
  border-top: 3px solid #17181c; page-break-before: always;
}
h1:first-of-type, h1:nth-of-type(2) { page-break-before: avoid; border-top: none; }
header#title-block-header { display: none; }
h2 { font-size: 13pt; margin: 18px 0 6px; color: #333; }
p { margin: 7px 0; }
/* The two instruction kinds have to be distinguishable at a glance. */
strong { color: #000; }
blockquote {
  margin: 8px 0 8px 0; padding: 9px 13px;
  background: #eef4ff; border-left: 4px solid #2f6bd8; border-radius: 3px;
  font-size: 12.5pt; line-height: 1.5;
}
blockquote p { margin: 3px 0; }
code {
  font: 10.5pt/1.4 "SF Mono", Menlo, Consolas, monospace;
  background: #f2f3f5; padding: 1px 4px; border-radius: 3px;
}
pre {
  background: #16181d; color: #e6e9ee; padding: 10px 13px; border-radius: 5px;
  page-break-inside: avoid;
  /* Wrap rather than clip: a URL cut off at the page edge is a URL the reader
     copies wrong without noticing. */
  white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere;
}
pre code { background: none; color: inherit; font-size: 10pt; padding: 0; }
table { border-collapse: collapse; margin: 10px 0; font-size: 11pt; }
th, td { border: 1px solid #ccd; padding: 5px 10px; text-align: left; }
th { background: #f2f3f5; }
hr { border: none; border-top: 1px dashed #bbb; margin: 16px 0; }
a { color: #2f6bd8; text-decoration: none; }
li { margin: 4px 0; }
CSS

pandoc "$SRC" \
  --standalone --embed-resources \
  --metadata title="LP-0002 — script vidéo" \
  --css "$TMP/style.css" \
  -o "$TMP/script.html"

"$CHROME" --headless --disable-gpu --no-pdf-header-footer \
  --print-to-pdf="$ROOT/$OUT" "file://$TMP/script.html" 2>/dev/null

[ -s "$OUT" ] || { echo "PDF was not produced" >&2; exit 1; }
echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
