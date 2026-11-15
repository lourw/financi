#!/usr/bin/env bash
# Render the committed PNG icon set from assets/icon/financi.svg (3.63).
# Needs rsvg-convert (librsvg). Output: assets/icon/png/financi-<n>.png
# for n = 16 … 1024. Deterministic: re-running with the same SVG and
# librsvg version leaves the committed files unchanged.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v rsvg-convert >/dev/null; then
    echo "rsvg-convert not found; install librsvg (e.g. apt install librsvg2-bin)" >&2
    exit 1
fi

src=assets/icon/financi.svg
out=assets/icon/png
mkdir -p "$out"
for n in 16 32 64 128 256 512 1024; do
    rsvg-convert -w "$n" -h "$n" "$src" -o "$out/financi-$n.png"
done
echo "wrote $out/financi-{{16,32,64,128,256,512,1024}}.png"
