#!/usr/bin/env bash
# Key interop cross-check: MFBASIC `crypto::generate` / `sign` / `verify` /
# `exchange` / `convert` against implementations that share no code with it,
# over the full `crypto::Certificate` matrix and both `KeyConvert` directions.
#
# Unlike the sibling oracles this script is thin: key interop needs more than one
# round (you must see a key MFB generated before you can ask it to sign with
# that key), so `rust/` is the DRIVER rather than a reference the shell compares
# against. It spawns the MFB program with a job in the environment.
#
# `tests/rt_crypto_key_interop.rs` covers the Ed25519 / X25519 / P-256 / P-384
# subset on every `cargo test`. Ed448, X448 and P-521 need crates that are not
# in the compiler's lockfile, and adding them would be new compiled code in
# every CI job on five platforms -- so they are checked here.
#
# Usage: ./run.sh [path-to-mfb-binary]
set -uo pipefail
. "$(dirname "$0")/../_lib/harness.sh"
oracle_init "$0" "$@"

# No output marker: this program correctly prints nothing until it is handed a
# job, so "it ran" is the only thing the flavor probe can require of it.
oracle_build_mfb
oracle_build_rust keysref

"$ORACLE_REF_BIN" "$ORACLE_MFB_EXE"
status=$?
case $status in
  0) exit 0 ;;
  1) exit 1 ;;
  *) exit 2 ;;
esac
