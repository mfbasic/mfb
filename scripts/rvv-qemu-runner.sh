#!/usr/bin/env bash
# plan-32-D: a `runtime_ulp.py --runner` for the one-binary RVV dual-path.
#
# Runs a `linux-riscv64` mfb executable under qemu-user on the riscv64 box
# (default 2232, Debian glibc) under a chosen CPU profile, relaying the program's
# stdout and exit code — so the SAME binary can be scored under `v=true` (native
# RVV arm) and `v=false` (scalar arm) and required to match ≤1 ULP in both.
#
# Both remote riscv64 boxes lack the V extension in hardware, so qemu-user is the
# portable oracle: `-cpu rv64,v=true` emulates V and sets AT_HWCAP bit 21;
# `v=false` is equivalent to a native run. See `.ai/remote_systems.md`.
#
# Env:
#   MFB_QEMU_CPU  cpu profile         (default: rv64,v=true)
#   MFB_RV_PORT   ssh port of the box (default: 2232)
#   MFB_QEMU      qemu-user path      (default: ~/qemuroot/usr/bin/qemu-riscv64)
#
# Usage (from runtime_ulp.py): `<this-script> <executable>`.
set -u
# runtime_ulp.py passes the last-written artifact (musl); the glibc box's loader
# runs the glibc variant under qemu-user, so map to it.
EXE="${1/-musl.out/-glibc.out}"
CPU="${MFB_QEMU_CPU:-rv64,v=true}"
PORT="${MFB_RV_PORT:-2232}"
QEMU="${MFB_QEMU:-\$HOME/qemuroot/usr/bin/qemu-riscv64}"
REMOTE="/tmp/rvvrun.$$.${RANDOM}"
SSH_OPTS=(-o ConnectTimeout=15 -o BatchMode=yes -o StrictHostKeyChecking=no)

scp -q -P "$PORT" "${SSH_OPTS[@]}" "$EXE" "test@127.0.0.1:$REMOTE" >&2 || exit 97
ssh -p "$PORT" "${SSH_OPTS[@]}" test@127.0.0.1 \
  "$QEMU -cpu $CPU $REMOTE; c=\$?; rm -f $REMOTE; exit \$c"
