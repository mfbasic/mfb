#!/usr/bin/env bash
#
# fetch-suite.sh — clone the official JSON-Schema-Test-Suite beside this script.
#
# The suite is the strongest oracle available: unlike a second implementation,
# every case states whether the instance is valid, so a disagreement identifies
# WHICH side is wrong. It is a separate repository under its own licence, so it
# is fetched rather than vendored, and `oracle.mjs suite` skips itself with a
# message when it is absent.
#
#     ./fetch-suite.sh                 # clone (or update) into ./JSON-Schema-Test-Suite
#     node oracle.mjs suite            # then this finds it
#
# Pass --suite <dir> to oracle.mjs, or set $JSON_SCHEMA_TEST_SUITE, to use a
# checkout you already have.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${1:-$HERE/JSON-Schema-Test-Suite}"
REPO="https://github.com/json-schema-org/JSON-Schema-Test-Suite.git"

if [[ -d "$DEST/.git" ]]; then
  echo "==> Updating $DEST"
  git -C "$DEST" pull --ff-only
else
  echo "==> Cloning $REPO into $DEST"
  git clone --depth 1 "$REPO" "$DEST"
fi

if [[ ! -d "$DEST/tests/draft2020-12" ]]; then
  echo "the checkout has no tests/draft2020-12 directory — is $DEST the right repository?" >&2
  exit 1
fi

echo
echo "ready: $(ls "$DEST/tests/draft2020-12"/*.json | wc -l | tr -d ' ') Draft 2020-12 test files"
echo "run:   node oracle.mjs suite"
