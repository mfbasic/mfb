#!/usr/bin/env bash
#
# build-examples.sh — build every example project for every supported target
# and bundle the results into target/examples.zip.
#
# For each executable example under examples/, this builds it once per target
# via `mfb build --target <os-arch>`, then copies that build's artifacts into
#
#     target/examples/<example>/<target>/
#
# A per-target subfolder is required because artifact names collide across
# targets (linux-aarch64 and linux-riscv64 both emit <name>-glibc.out /
# <name>-musl.out, and macOS emits <name>.out). When all examples are built,
# the whole tree is zipped to target/examples.zip and the staging tree is
# removed.
#
# Builds that fail (e.g. a target that a given example cannot cross-compile to)
# are reported and skipped; the script keeps going and exits non-zero if any
# build failed.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

EXAMPLES_DIR="$ROOT/examples"
STAGE_DIR="$ROOT/target/examples"
ZIP_PATH="$ROOT/target/examples.zip"

# The registered native targets (kept in sync with registered_targets() in
# src/target.rs). Every example is built for each of these.
TARGETS=(
  macos-aarch64
  linux-aarch64
  linux-x86_64
  linux-riscv64
  windows-x86_64
)

# Locate the compiler, building a release binary if none exists yet.
MFB="$ROOT/target/release/mfb"
if [[ ! -x "$MFB" ]]; then
  echo "==> No release compiler at $MFB; building it"
  cargo build --release --bin mfb
fi

# The browser example is split into three local packages the app imports by
# relative path (examples/browser/README.md "Building"). Each must be built to a
# .mfp and installed at the next project's packages/<name>.mfp before that
# project can build. Packages carry portable IR (not native code), so this runs
# once, ahead of the per-target app builds. Mirrors the README exactly:
# dom (no deps) -> fetch & display (import dom) -> app (imports all three).
prepare_browser() {
  local b="$EXAMPLES_DIR/browser"
  [[ -d "$b/app" ]] || return 0

  echo "==> Preparing browser packages (dom -> fetch/display -> app)"

  "$MFB" build -q "$b/dom"

  local p
  for p in fetch display; do
    mkdir -p "$b/$p/packages"
    cp "$b/dom/dom.mfp" "$b/$p/packages/dom.mfp"
    "$MFB" build -q "$b/$p"
  done

  mkdir -p "$b/app/packages"
  cp "$b/dom/dom.mfp" "$b/fetch/fetch.mfp" "$b/display/display.mfp" \
     "$b/app/packages/"
}

prepare_browser

# Fresh staging tree.
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

fail_count=0
build_count=0

# Discover executable example projects. The example's display name is the first
# path segment under examples/ (so examples/browser/app -> "browser").
while IFS= read -r manifest; do
  proj_dir="$(dirname "$manifest")"

  # Only executables produce per-target binaries; skip package projects.
  if ! grep -Eq '"kind"[[:space:]]*:[[:space:]]*"executable"' "$manifest"; then
    continue
  fi

  rel="${proj_dir#"$EXAMPLES_DIR"/}"
  example="${rel%%/*}"

  for target in "${TARGETS[@]}"; do
    echo "==> Building $example for $target"

    # Clear any prior artifacts so a copy only ever picks up this target's
    # output (mfb build already cleans build/, but be explicit).
    rm -rf "$proj_dir/build"

    if "$MFB" build -q --target "$target" "$proj_dir"; then
      dest="$STAGE_DIR/$example/$target"
      mkdir -p "$dest"
      if [[ -d "$proj_dir/build" ]] && [[ -n "$(ls -A "$proj_dir/build" 2>/dev/null)" ]]; then
        cp -R "$proj_dir/build/." "$dest/"
      else
        echo "    (warning: no build output produced for $example/$target)"
      fi
      build_count=$((build_count + 1))
    else
      echo "    FAILED: $example for $target"
      fail_count=$((fail_count + 1))
    fi
  done
done < <(find "$EXAMPLES_DIR" -name project.json | sort)

# Bundle the staging tree into target/examples.zip (paths inside the archive are
# examples/<example>/<target>/...), then drop the staging tree.
echo "==> Packaging $ZIP_PATH"
rm -f "$ZIP_PATH"
( cd "$ROOT/target" && zip -r -q "$ZIP_PATH" examples )
rm -rf "$STAGE_DIR"

echo "==> Done: $build_count build(s) archived, $fail_count failure(s)"
echo "    Wrote $ZIP_PATH"

if [[ "$fail_count" -gt 0 ]]; then
  exit 1
fi
