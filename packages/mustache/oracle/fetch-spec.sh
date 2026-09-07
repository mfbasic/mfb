#!/usr/bin/env bash
#
# fetch-spec.sh — download the official Mustache specification suite.
#
# The suite is a set of JSON files of {template, data, partials, expected}
# cases, maintained at github.com/mustache/spec. It is NOT vendored here: it is
# a moving upstream document, and pinning a copy would quietly turn "the
# specification says" into "the specification said, once". Fetch it and run it.
#
#     ./fetch-spec.sh          # the six required modules
#
# Downloads into spec/, which is git-ignored.
#
# Only the required modules are fetched. `~lambdas` is the specification's one
# optional module and this package does not implement it -- a json::Json value
# cannot hold a callable -- so running those cases would report failures for a
# feature that is documented as absent. `~dynamic-names` and `~inheritance` are
# likewise optional and out of scope.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REF="${MUSTACHE_SPEC_REF:-master}"
BASE="https://raw.githubusercontent.com/mustache/spec/${REF}/specs"

mkdir -p "$HERE/spec"
for module in comments delimiters interpolation inverted partials sections; do
  printf 'fetching %s.json ... ' "$module"
  curl -fsS -o "$HERE/spec/$module.json" "$BASE/$module.json"
  printf 'ok (%s cases)\n' "$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")).tests.length)' "$HERE/spec/$module.json")"
done

echo
echo "spec/ is ready. Run: node diff.mjs spec"
