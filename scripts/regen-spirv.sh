#!/usr/bin/env bash
# Regenerate the canvas Vulkan shaders' SPIR-V from their GLSL (plan-98-F Phase 1).
#
# Vulkan takes SPIR-V, not source — unlike Metal, which compiles the MSL string at
# run time (plan-98-E Correction 2). So the compiled blobs are **checked in** beside
# the GLSL they came from, and this script is how they are reproduced.
#
# That is deliberately not a build step. Making `mfb build` shell out to a shader
# compiler would put glslang between a user and their program, which is exactly the
# dependency plan-98-E rejected an `xcrun metal` step for. The blobs are ~12 KB of
# bytes in the repo; regenerating them is a maintainer action taken when the GLSL
# changes, and the unit tests check the two stay in step.
#
# glslang is not installed on the macOS development host and is not required to be:
# by default this ships the GLSL to a Linux test box, compiles it there against a
# user-local `dpkg -x` of `glslang-tools` (no root needed, the same trick
# `.ai/remote_systems.md` documents for qemu-user), and copies the SPIR-V back.
#
#   scripts/regen-spirv.sh                 # via the Ubuntu x86_64 box (port 2228)
#   scripts/regen-spirv.sh --local         # with a glslangValidator already on PATH
#   MFB_SPIRV_PORT=2227 scripts/regen-spirv.sh   # a different box
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SHADERS="$ROOT/src/codegen/runtime/canvas/shaders"
PORT="${MFB_SPIRV_PORT:-2228}"
STAGES=(vert frag)

compile_local() {
  local tool
  tool="$(command -v glslangValidator || true)"
  if [ -z "$tool" ]; then
    echo "regen-spirv: no glslangValidator on PATH; drop --local to use a test box" >&2
    exit 1
  fi
  for stage in "${STAGES[@]}"; do
    "$tool" -V "$SHADERS/mfb_canvas.$stage" -o "$SHADERS/mfb_canvas.$stage.spv"
    echo "regen-spirv: $stage -> $(wc -c < "$SHADERS/mfb_canvas.$stage.spv") bytes"
  done
}

compile_remote() {
  local host="test@127.0.0.1"
  local remote="/tmp/mfb-spirv-$$"
  echo "regen-spirv: compiling on port $PORT"
  ssh -p "$PORT" "$host" "mkdir -p $remote"
  # shellcheck disable=SC2086
  scp -P "$PORT" "$SHADERS"/mfb_canvas.vert "$SHADERS"/mfb_canvas.frag "$host:$remote/" >/dev/null
  ssh -p "$PORT" "$host" "
    set -e
    cd $remote
    if ! command -v glslangValidator >/dev/null; then
      # No root on the test boxes, so unpack the package into the scratch dir.
      apt-get download glslang-tools >/dev/null 2>&1
      mkdir -p root && dpkg -x glslang-tools_*.deb root
      TOOL=\$PWD/root/usr/bin/glslangValidator
    else
      TOOL=\$(command -v glslangValidator)
    fi
    \$TOOL --version | head -1
    for stage in vert frag; do
      \$TOOL -V mfb_canvas.\$stage -o mfb_canvas.\$stage.spv
    done
  "
  for stage in "${STAGES[@]}"; do
    scp -P "$PORT" "$host:$remote/mfb_canvas.$stage.spv" "$SHADERS/" >/dev/null
    echo "regen-spirv: $stage -> $(wc -c < "$SHADERS/mfb_canvas.$stage.spv") bytes"
  done
  ssh -p "$PORT" "$host" "rm -rf $remote"
}

if [ "${1:-}" = "--local" ]; then
  compile_local
else
  compile_remote
fi

echo "regen-spirv: done — run 'cargo test --bin mfb vulkan' to check the blobs"
