#!/usr/bin/env bash
# Instruction-form census over the fixed corpus (plan-171-A): build every corpus
# project with `mfb build -q -ncode -target <target>` in a scratch copy and run
# census.py over the dumps. See README.md.
#
#   tools/ncode-census/ncode-census.sh <mfb> <target> <out.tsv> [extra mfb build flags...]
#
# Env: JOBS (parallel builds, default 6), KEEP=<dir> (keep the scratch copies and
# dumps there instead of a deleted temp dir — for census.py --top afterwards).
set -euo pipefail

if [ "$#" -lt 3 ]; then
    sed -n '2,9p' "$0" >&2
    exit 2
fi
MFB=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
TARGET=$2
OUT=$3
shift 3
EXTRA_FLAGS="$*"
JOBS=${JOBS:-6}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)

[ -x "$MFB" ] || { echo "ncode-census: '$MFB' is not executable" >&2; exit 2; }

if [ -n "${KEEP:-}" ]; then
    WORK=$KEEP
    rm -rf "$WORK/projects"
    mkdir -p "$WORK"
else
    WORK=$(mktemp -d /tmp/ncode-census.XXXXXX)
    trap 'rm -rf "$WORK"' EXIT
fi
mkdir -p "$WORK/projects"

# Snapshot the compiler so a rebuild mid-run cannot mix two compilers' output.
cp "$MFB" "$WORK/mfb"

# Never build in-tree (.ai/testing-gates.md "A corpus sweep that builds IN-TREE
# clobbers a concurrent one"): every project is copied, minus its build/ dir.
scratch_of() {
    printf '%s/projects/%s\n' "$WORK" "${1//\//__}"
}
copy_in() {
    local scratch
    scratch=$(scratch_of "$1")
    mkdir -p "$scratch"
    (cd "$ROOT/$1" && tar --exclude ./build -cf - .) | (cd "$scratch" && tar -xf -)
}

# The corpus: every examples/* executable, the language benchmark, every
# byte-identity fixture. `kind: package` projects are dependencies, not corpus
# members. examples/audio is excluded: it imports `jzaun#libsnd` from the
# registry, which a hermetic sweep cannot fetch.
corpus=()
while IFS= read -r manifest; do
    dir=$(dirname "$manifest")
    grep -q '"kind": *"package"' "$ROOT/$manifest" && continue
    [ "$dir" = examples/audio ] && continue
    corpus+=("$dir")
done < <(cd "$ROOT" && find examples benchmark/language/mfb tests/byte-identity \
    -name project.json -not -path '*/build/*' | sort)

for dir in "${corpus[@]}"; do
    copy_in "$dir"
done

# Local packages a corpus project imports as `file:packages/<name>.mfp` and the
# repository does not commit: built here, in their own scratch copies, exactly as
# their READMEs and scripts/build-examples.sh / benchmark/language/run.sh do.
package() { # <package dir> <mfp name> <consumer dir>...
    local source=$1 mfp=$2 built consumer
    shift 2
    built=$(scratch_of "$source")
    [ -f "$built/project.json" ] || copy_in "$source"
    "$WORK/mfb" build -q "$built" >"$built.log" 2>&1 ||
        { echo "ncode-census: package build failed: $source (log: $built.log)" >&2; return 0; }
    for consumer in "$@"; do
        mkdir -p "$(scratch_of "$consumer")/packages"
        cp "$built/$mfp" "$(scratch_of "$consumer")/packages/$mfp"
    done
}
package benchmark/language/mfb/workers bench_workers.mfp benchmark/language/mfb
package examples/browser/dom dom.mfp examples/browser/fetch examples/browser/display \
    examples/browser/app
package examples/browser/fetch fetch.mfp examples/browser/app
package examples/browser/display display.mfp examples/browser/app
package packages/yaml yaml.mfp examples/yaml-json

build_one() {
    local dir=$1 scratch name
    scratch=$(printf '%s/projects/%s' "$WORK" "${dir//\//__}")
    rm -f "$scratch.ok" "$scratch.failed"
    name=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["name"])' \
        "$scratch/project.json")
    rm -f "$scratch/$name.ncode"
    # shellcheck disable=SC2086 # EXTRA_FLAGS is a word list of plain flags
    if "$WORK/mfb" build -q -ncode -target "$TARGET" $EXTRA_FLAGS "$scratch" \
        >"$scratch.log" 2>&1 && [ -f "$scratch/$name.ncode" ]; then
        printf '%s\n' "$scratch/$name.ncode" >"$scratch.ok"
    else
        : >"$scratch.failed"
    fi
}
export -f build_one
export WORK TARGET EXTRA_FLAGS

printf '%s\n' "${corpus[@]}" | xargs -P "$JOBS" -I{} bash -c 'build_one "$@"' _ {}

args=()
failed=()
for dir in "${corpus[@]}"; do
    scratch=$(scratch_of "$dir")
    if [ -f "$scratch.ok" ]; then
        args+=(--label "$dir" "$(cat "$scratch.ok")")
    else
        failed+=("$dir")
        echo "ncode-census: build failed: $dir (log: $scratch.log)" >&2
    fi
done
python3 -I "$HERE/census.py" "${args[@]}" >"$OUT"
for dir in "${failed[@]+"${failed[@]}"}"; do
    printf '%s\t*\tbuild_failed\t1\n' "$dir" >>"$OUT"
done
echo "ncode-census: ${#corpus[@]} projects, ${#failed[@]} failed, target $TARGET -> $OUT" >&2
