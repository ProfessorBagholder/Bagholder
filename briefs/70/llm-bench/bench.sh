#!/bin/bash
# The filings-model benchmark (brief 70). Runs the same engine build, models and
# filing texts on any machine; GPU on and GPU off. Everything it downloads lives in
# a temporary folder under $TMPDIR that is deleted when it ends, whatever way it ends. It starts
# one llama-server at a time on 127.0.0.1:18080 and stops it before the next.
# Usage: ./bench.sh            (writes results-<host>.md in the current folder)
set -euo pipefail
BUILD=b11750
NGLS="${NGLS:-99 0}"   # GPU on, then off; NGLS=0 on a machine with no usable GPU
REPS="${REPS:-2}"
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$PWD/results-$(hostname -s)${OUT_SUFFIX:-}.md"
# under $TMPDIR, which a sandboxed session may write to (macOS mktemp ignores TMPDIR otherwise)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/llm-bench.XXXXXX")
SERVER=""
cleanup() { [ -n "$SERVER" ] && kill "$SERVER" 2>/dev/null; wait 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT INT TERM
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ASSET=macos-arm64 ;;
  Darwin-x86_64) ASSET=macos-x64 ;;
  Linux-x86_64) ASSET=ubuntu-vulkan-x64 ;;
  Linux-aarch64) ASSET=ubuntu-vulkan-arm64 ;;
  *) echo "no build for $(uname -s)-$(uname -m)"; exit 1 ;;
esac
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -c1-64; else shasum -a 256 "$1" | cut -c1-64; fi; }
curl -fsSL "https://github.com/ggml-org/llama.cpp/releases/download/$BUILD/llama-$BUILD-bin-$ASSET.tar.gz" | tar xz -C "$WORK"
BIN=$(dirname "$(find "$WORK" -name llama-bench -type f | head -1)")
export LD_LIBRARY_PATH="$BIN" DYLD_LIBRARY_PATH="$BIN"
{
  echo "# Filings-model benchmark on $(hostname -s), $(date -u +%Y-%m-%dT%H:%MZ)"
  echo
  echo "- machine: $(uname -sm); $( (sysctl -n machdep.cpu.brand_string 2>/dev/null || grep -m1 'model name' /proc/cpuinfo | cut -d: -f2) | sed 's/^ *//'); $( (sysctl -n hw.memsize 2>/dev/null | awk '{print $1/1073741824 " GB"}') || free -g | awk '/Mem:/{print $2 " GB"}')"
  echo "- engine: llama.cpp $BUILD ($ASSET)"
  echo "- devices:"; "$BIN/llama-bench" --list-devices 2>&1 | sed -n '/Available devices/,$p' | sed 's/^/    /'
  echo
} > "$OUT"
while read -r LABEL URL SUM; do
  case "$LABEL" in ''|\#*) continue ;; esac
  M="$WORK/$LABEL.gguf"
  echo "== $LABEL: downloading"
  curl -fsSL -o "$M" "$URL"
  if [ "$SUM" != "-" ] && [ "$(sha "$M")" != "$SUM" ]; then echo "SHA mismatch for $LABEL"; exit 1; fi
  echo "## $LABEL ($(du -h "$M" | cut -f1))" >> "$OUT"
  echo '```' >> "$OUT"
  for NGL in $NGLS; do
    echo "== $LABEL: speed, ngl=$NGL"
    "$BIN/llama-bench" -m "$M" -p 768 -n 48 -r "$REPS" -ngl "$NGL" 2>/dev/null | grep -E '^\|' >> "$OUT" || echo "(llama-bench failed at ngl=$NGL)" >> "$OUT"
  done
  echo '```' >> "$OUT"
  echo "== $LABEL: quality (one call per filing, GPU on)"
  "$BIN/llama-server" -m "$M" -ngl 99 -c 4096 --host 127.0.0.1 --port 18080 >/dev/null 2>&1 &
  SERVER=$!
  for i in $(seq 1 120); do curl -sf http://127.0.0.1:18080/health >/dev/null && break; sleep 1; done
  echo "| model | filing | time | tokens in/out | title | summary |" >> "$OUT"
  echo "|---|---|---|---|---|---|" >> "$OUT"
  python3 -I "$HERE/quality.py" "$HERE/samples" "$LABEL" 18080 >> "$OUT"
  kill "$SERVER"; wait "$SERVER" 2>/dev/null || true; SERVER=""
  echo >> "$OUT"
  rm -f "$M"
done < "$HERE/${MODELS_FILE:-models.txt}"
echo "Done: $OUT"
