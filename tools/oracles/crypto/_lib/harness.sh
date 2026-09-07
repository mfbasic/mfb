# Shared plumbing for the crypto oracles' run.sh scripts. Source it; do not run it.
#
# Every oracle here has the same skeleton -- build the MFBASIC subject, run it,
# build the Rust judge, compare -- and the fiddly part is identical in all of
# them: finding an `mfb` binary, and picking which of the executables a build
# emitted this host can actually load. That logic existing once means a fix to it
# reaches every oracle, instead of five copies drifting apart.
#
# Contract for the caller:
#   . "$(dirname "$0")/../_lib/harness.sh"
#   oracle_init "$0" "$@"      # sets HERE, ROOT, WORK, MFB_EXE; installs cleanup
#   oracle_run_mfb             # builds+runs $HERE/mfb -> $ORACLE_MFB_OUT
#   oracle_build_rust <bin>    # builds $HERE/rust -> $ORACLE_REF_BIN
#
# Exit codes, shared by every oracle in this tree:
#   0  everything agreed
#   1  at least one case disagreed
#   2  the harness could not run (build failure, no cases, wrong count)

say() { printf '%s\n' "$*"; }
die() { printf '%s\n' "$*" >&2; exit 2; }

# oracle_init <path-to-run.sh> [mfb-binary]
oracle_init() {
  HERE=$(cd "$(dirname "$1")" && pwd) || exit 2
  # <repo>/tools/oracles/crypto/<name>/run.sh -> up four to the repo root.
  ROOT=$(cd "$HERE/../../../.." && pwd) || exit 2
  shift

  WORK=$(mktemp -d "${TMPDIR:-/tmp}/mfb-oracle.XXXXXX") || exit 2
  # shellcheck disable=SC2064
  trap "rm -rf '$WORK'" EXIT
  ORACLE_MFB_OUT="$WORK/mfb-output.txt"

  MFB_EXE=${1:-${MFB_EXE:-$ROOT/target/release/mfb}}
  if [ ! -x "$MFB_EXE" ]; then
    say "building $MFB_EXE"
    (cd "$ROOT" && cargo build --release --bin mfb) || die "cannot build mfb"
  fi
  [ -x "$MFB_EXE" ] || die "no such mfb binary: $MFB_EXE"
}

# Build $HERE/mfb and run it, leaving its stdout in $ORACLE_MFB_OUT.
#
# A `mfb build` can emit several libc flavors on one host and only some of them
# load here. Rather than guess from `ldd`, try each and keep the first that
# actually produces case lines -- a flavor this host cannot exec fails outright.
oracle_run_mfb() {
  say "building mfb/"
  rm -rf "$HERE/mfb/build"
  local build_log paths candidate
  build_log=$("$MFB_EXE" build -q "$HERE/mfb" 2>&1) || {
    printf '%s\n' "$build_log" >&2
    die "mfb build failed"
  }
  paths=$(printf '%s\n' "$build_log" | sed -n 's/^Wrote executable to //p')
  [ -n "$paths" ] || { printf '%s\n' "$build_log" >&2; die "build reported no executable"; }

  : >"$ORACLE_MFB_OUT"
  ORACLE_MFB_EXE=""
  while IFS= read -r candidate; do
    [ -n "$candidate" ] || continue
    [ -x "$candidate" ] || continue
    if "$candidate" >"$ORACLE_MFB_OUT" 2>"$WORK/mfb-stderr.txt" &&
       grep -q '^case ' "$ORACLE_MFB_OUT"; then
      ORACLE_MFB_EXE=$candidate
      break
    fi
  done <<EOF
$paths
EOF
  if [ -z "$ORACLE_MFB_EXE" ]; then
    [ -s "$WORK/mfb-stderr.txt" ] && cat "$WORK/mfb-stderr.txt" >&2
    die "no built flavor ran and emitted case lines"
  fi
  say "ran $ORACLE_MFB_EXE"
}

# oracle_build_rust <binary-name> -- build $HERE/rust, set $ORACLE_REF_BIN.
oracle_build_rust() {
  say "building rust/"
  (cd "$HERE/rust" && cargo build --release --quiet) || die "cannot build the rust reference"
  ORACLE_REF_BIN="$HERE/rust/target/release/$1"
  [ -x "$ORACLE_REF_BIN" ] || die "no reference binary at $ORACLE_REF_BIN"
}

# oracle_verdict <label> <ran> <fail> <expected>
#
# The expected count is the caller's own constant, never derived from the
# subject's output: a count taken from the producer is true by construction, so
# a program that died after case 3 would report "3 of 3 agreed" and pass.
oracle_verdict() {
  say "$1: $2 case(s), $3 failure(s)"
  if [ "$2" -ne "$4" ]; then
    say "ran $2 case(s) but expected $4 -- that is a harness bug, not a pass" >&2
    exit 2
  fi
  [ "$3" -eq 0 ] || exit 1
  exit 0
}
