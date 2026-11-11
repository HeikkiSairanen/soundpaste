#!/bin/bash
# Kääntää sivun (./rakenna.sh) ja julkaisee dist/-hakemiston GitHub Pagesiin
# (haara gh-pages, osoite https://heikkisairanen.github.io/soundpaste/).
# Haarassa on aina vain yksi commit: käännöstuotteiden historiaa ei säilytetä.
set -euo pipefail
cd "$(dirname "$0")"

./rakenna.sh
REMOTE="$(git remote get-url origin)"
REV="$(git rev-parse --short HEAD)"
NAME="$(git config user.name)"
MAIL="$(git config user.email)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cp -R dist/. "$TMP/"
find "$TMP" -name .DS_Store -delete
touch "$TMP/.nojekyll"          # tiedostot sellaisenaan, ei Jekyll-käsittelyä
git -C "$TMP" init -q -b gh-pages
git -C "$TMP" add -A
git -C "$TMP" -c user.name="$NAME" -c user.email="$MAIL" commit -q -m "Sivu versiosta $REV"
git -C "$TMP" push -q -f "$REMOTE" gh-pages
echo "Julkaistu: https://heikkisairanen.github.io/soundpaste/ (päivittyy noin minuutissa)"
