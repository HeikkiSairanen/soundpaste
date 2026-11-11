#!/bin/zsh
# Kääntää komentorivityökalun ja selainversion ja kokoaa julkaistavan sivun
# hakemistoon dist/ (index.html, style.css, app.js, worklet.js, soundpaste.wasm).
#
#   ./rakenna.sh            # käännä kaikki ja kokoa dist/
#   ./rakenna.sh --testit   # aja lisäksi testit
set -euo pipefail
cd "${0:A:h}"

[[ "${1:-}" == "--testit" ]] && cargo test -q
cargo build --release -q -p soundpaste-cli
cargo build -q --profile wasm --target wasm32-unknown-unknown -p soundpaste-wasm

rm -rf dist
mkdir -p dist
cp web/index.html web/style.css web/app.js web/worklet.js dist/
cp target/wasm32-unknown-unknown/wasm/soundpaste_wasm.wasm dist/soundpaste.wasm

# Julkaistava sivu ei saa ottaa yhteyttä mihinkään eikä sisältää kehitystyökaluja.
if grep -RIlE 'https?://|/dev/|WebSocket|XMLHttpRequest|sendBeacon|EventSource' dist --include='*.js' --include='*.html' --include='*.css'; then
  echo "VIRHE: yllä luetelluissa tiedostoissa on verkko-osoite tai kehitystyökalujen jälki" >&2
  exit 1
fi
ls -l dist
