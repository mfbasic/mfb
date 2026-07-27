#!/usr/bin/env bash
# Rebuild every buildable package fixture from source and copy the resulting
# `<name>.mfp` over every committed copy of it (consumer `packages/<name>.mfp` and
# fixture `golden/<name>.mfp`).
#
# Why this exists: a package `.mfp` is a compiled binary artifact. Consumer
# fixtures carry a *committed copy* of a dependency package's `.mfp`, and package
# fixtures carry a golden `.mfp`. When the binary-representation format changes
# (e.g. plan-58-C, BINARY_REPR 5->6), those committed copies go stale — the source
# is unchanged so nothing regenerates them. A stale copy is silently mis-lowered by
# the newer compiler; plan-58-C left them stale and it surfaced as a runtime
# SIGSEGV in `trap-builtin-consumer` (the toInt-TRAP error path). Run this after any
# change that alters `.mfp` bytes so no consumer copy is left behind.
#
# Package sources built here:
#   tools/thread-package-sources/*    worker packages for the thread test suite
#   tools/link-package-sources/*      native link-collision packages
#   tests/{syntax,rt-behavior}/**     in-tree `kind: package` fixtures (valid ones)
#
# NOT handled — the security fixtures under `tools/security-package-sources/*` are
# CRAFTED and deliberately TAMPERED by their own `generate.py` (via `mfp_craft.py`).
# Their committed `.mfp` are intentionally malformed to exercise the decode/verify
# rejection paths, so a clean rebuild must never overwrite them. Regenerate those
# with `python3 tools/security-package-sources/<pkg>/generate.py` when needed.
#
# Usage: scripts/sync-package-mfp.sh <mfb-exe>
#   <mfb-exe>: use the RELEASE build (target/release/mfb). A debug compiler injects
#     plan-67 perf instrumentation into the `.mfp` codegen, which is not what ships.
set -u

MFB=${1:?usage: sync-package-mfp.sh <mfb-exe> (use target/release/mfb)}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT" || exit 1

if [ ! -x "$MFB" ]; then
  echo "error: mfb executable not found or not executable: $MFB" >&2
  exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

updated=0
unchanged=0
build_failed=0
no_copy=0

# A package source is any `project.json` with `"kind": "package"`, excluding the
# deliberately-tampered security sources and negative (`*-invalid`) fixtures that
# are expected not to build.
package_projects() {
  # tools sources + in-tree fixtures
  grep -rl '"kind"[[:space:]]*:[[:space:]]*"package"' \
    tools/thread-package-sources tools/link-package-sources tests \
    --include=project.json 2>/dev/null \
    | grep -v '/security-package-sources/' \
    | grep -v -- '-invalid/'
}

while IFS= read -r proj; do
  [ -n "$proj" ] || continue
  srcdir=$(dirname "$proj")
  name=$(grep -o '"name"[^,]*' "$proj" | head -1 | sed -E 's/.*"name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')
  [ -n "$name" ] || continue

  # Build in a clean copy so no prior build artifact leaks in.
  bd="$work/$name"
  rm -rf "$bd"
  cp -r "$srcdir" "$bd"
  if ! "$MFB" build "$bd" >/dev/null 2>&1; then
    echo "build-failed: $name ($srcdir)" >&2
    build_failed=$((build_failed + 1))
    continue
  fi
  fresh=$(find "$bd" -maxdepth 1 -name "$name.mfp" | head -1)
  if [ -z "$fresh" ]; then
    echo "no-mfp-produced: $name ($srcdir)" >&2
    build_failed=$((build_failed + 1))
    continue
  fi

  # Copy over every committed copy of this package, anywhere under tests/.
  found_any=0
  while IFS= read -r committed; do
    [ -n "$committed" ] || continue
    found_any=1
    if cmp -s "$fresh" "$committed"; then
      unchanged=$((unchanged + 1))
    else
      cp "$fresh" "$committed"
      echo "updated: $committed"
      updated=$((updated + 1))
    fi
  done < <(find tests -name "$name.mfp")
  [ "$found_any" -eq 0 ] && no_copy=$((no_copy + 1))
done < <(package_projects)

echo "----"
echo "updated $updated, unchanged $unchanged (build-failed/skipped $build_failed, no committed copy $no_copy)"
