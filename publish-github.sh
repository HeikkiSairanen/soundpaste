#!/bin/bash
# Builds the page (./build.sh) and publishes dist/ to GitHub Pages
# (branch gh-pages, address https://heikkisairanen.github.io/soundpaste/).
# The branch always holds a single commit: build output history is not kept.
set -euo pipefail
cd "$(dirname "$0")"

./build.sh
REMOTE="$(git remote get-url origin)"
REV="$(git rev-parse --short HEAD)"
NAME="$(git config user.name)"
MAIL="$(git config user.email)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cp -R dist/. "$TMP/"
find "$TMP" -name .DS_Store -delete
touch "$TMP/.nojekyll"          # serve the files as they are, no Jekyll processing
git -C "$TMP" init -q -b gh-pages
git -C "$TMP" add -A
git -C "$TMP" -c user.name="$NAME" -c user.email="$MAIL" commit -q -m "Page built from $REV"
git -C "$TMP" push -q -f "$REMOTE" gh-pages
echo "Published: https://heikkisairanen.github.io/soundpaste/ (updates in about a minute)"
