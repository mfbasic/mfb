#!/usr/bin/env bash
#
# fetch-xmlconf.sh — download and unpack the W3C XML Conformance Test Suite.
#
# The suite is FETCHED, not vendored: it is an upstream artifact of about 640 KB
# with thousands of files, and pinning a copy in this repository would mean
# maintaining someone else's test suite. `packages/mustache/oracle/fetch-spec.sh`
# sets the precedent. The unpacked tree lands in xmlconf/, which this directory's
# .gitignore covers.
#
#     ./fetch-xmlconf.sh                 # fetch if absent
#     ./fetch-xmlconf.sh --force         # re-fetch even if present
#     XMLCONF_URL=<url> ./fetch-xmlconf.sh
#
# Exit status is 0 iff the suite is unpacked and its catalog is present.

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
URL="${XMLCONF_URL:-https://www.w3.org/XML/Test/xmlts20130923.tar.gz}"
TARGET="$HERE/xmlconf"
CATALOG="$TARGET/xmlconf/xmlconf.xml"

force=0
[[ "${1:-}" == "--force" ]] && force=1

if [[ -f "$CATALOG" && "$force" -eq 0 ]]; then
  echo "xmlconf already present ($(find "$TARGET" -name '*.xml' | wc -l | tr -d ' ') xml files); --force to re-fetch"
  exit 0
fi

rm -rf "$TARGET"
mkdir -p "$TARGET"

archive="$(mktemp)"
trap 'rm -f "$archive"' EXIT

echo "fetching $URL"
if ! curl -sSL --fail "$URL" -o "$archive"; then
  echo "could not download $URL" >&2
  exit 1
fi

if ! tar -xzf "$archive" -C "$TARGET"; then
  echo "could not unpack the archive" >&2
  exit 1
fi

if [[ ! -f "$CATALOG" ]]; then
  echo "unpacked, but $CATALOG is missing — is this the right archive?" >&2
  find "$TARGET" -maxdepth 2 -name 'xmlconf*' >&2
  exit 1
fi

echo "unpacked $(find "$TARGET" -name '*.xml' | wc -l | tr -d ' ') xml files into $TARGET"
