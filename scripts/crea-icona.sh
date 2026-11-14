#!/bin/sh
# Rigenera assets/Sentiero.icns da assets/icona.svg. Serve solo se si cambia il disegno:
# il file .icns è nel repository. Richiede rsvg-convert (brew install librsvg) e macOS.
set -eu
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
mkdir "$tmp/Sentiero.iconset"
for lato in 16 32 128 256 512; do
  rsvg-convert -w "$lato" assets/icona.svg -o "$tmp/Sentiero.iconset/icon_${lato}x${lato}.png"
  rsvg-convert -w "$((lato * 2))" assets/icona.svg -o "$tmp/Sentiero.iconset/icon_${lato}x${lato}@2x.png"
done
iconutil -c icns "$tmp/Sentiero.iconset" -o assets/Sentiero.icns
rm -rf "$tmp"
