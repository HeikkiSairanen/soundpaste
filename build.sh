#!/bin/zsh
# Builds the command-line tool and the browser version and assembles the
# publishable page into dist/ (index.html, style.css, app.js, worklet.js, soundpaste.wasm).
#
#   ./build.sh           # build everything and assemble dist/
#   ./build.sh --test    # also run the tests
set -euo pipefail
cd "${0:A:h}"

[[ "${1:-}" == "--test" ]] && cargo test -q
cargo build --release -q -p soundpaste-cli
cargo build -q --profile wasm --target wasm32-unknown-unknown -p soundpaste-wasm

rm -rf dist
mkdir -p dist
cp web/index.html web/style.css web/app.js web/worklet.js dist/
cp target/wasm32-unknown-unknown/wasm/soundpaste_wasm.wasm dist/soundpaste.wasm

# The published page must not contact anything or contain dev tooling.
if grep -RIlE 'https?://|/dev/|WebSocket|XMLHttpRequest|sendBeacon|EventSource' dist --include='*.js' --include='*.html' --include='*.css'; then
  echo "ERROR: the files listed above contain a network address or a trace of the dev tooling" >&2
  exit 1
fi
ls -l dist
