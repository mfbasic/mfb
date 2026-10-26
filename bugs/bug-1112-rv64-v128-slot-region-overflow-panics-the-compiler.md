# bug-1112: a riscv64 build of the language benchmark panics: a function needs more `v128` slots than the per-thread region holds

Last updated: 2026-10-25
Effort: large (3h–1d)
Severity: HIGH (compiler panic, no executable; a valid program cannot be built for linux-riscv64)
Class: Codegen (riscv64 `v128` scalarization)

Status: Open
Regression Test: a `tests/rt-behavior/vector/` fixture whose one function keeps more than 126
`v128` values live at once, with a `linux-riscv64` `.ncodesum` golden (so the gate builds it) and
its run output proven on box 2229/2232.

Found by plan-171-A's census (`tools/ncode-census/ncode-census.sh … linux-riscv64`, its
Correction 3). Pre-existing: reproduced with the release `mfb` of `89a4dce2c`, before any
plan-171 change; unrelated to load/store pairing (riscv64 never pairs).

## Failing Reproduction

```
cp -R benchmark/language/mfb /tmp/b && rm -rf /tmp/b/build && mkdir -p /tmp/b/packages
mfb build -q benchmark/language/mfb/workers   # or build a scratch copy of it
cp benchmark/language/mfb/workers/bench_workers.mfp /tmp/b/packages/
mfb build -q -ncode -target linux-riscv64 /tmp/b
```

- Observed (2026-10-25, macOS host, release `mfb` of `89a4dce2c`):
  ```
  thread 'mfb' panicked at src/arch/riscv64/select.rs:488:5:
  rv64 v128: function needs 172 concurrent vector slots, exceeding the 126-slot region
  ```
  No executable is written. The same project builds for macos-aarch64, linux-aarch64,
  linux-x86_64 and windows-x86_64.
- Expected: the program builds for linux-riscv64 and prints the same checksums as on the other
  targets (`benchmark/language/run.sh` validates them across targets).

## Root Cause

`select_riscv64` gives every distinct `v128` value of a function a 16-byte slot in a fixed
per-thread region inside the arena state (`arch::riscv64::v128::build_slot_map`, region
`ARENA_V128_SLOTS_OFFSET`, `SLOT_COUNT = 126`, capped so every lane offset fits the 12-bit
load/store immediate from `s11`), recycling slots across non-overlapping live ranges. One function
of the benchmark has 172 values live at once, so the assertion in `select_riscv64` fires — a panic
instead of a fallback. Which function was not isolated (the panic names none); the vector/math
benchmark files are the candidates.

## Fix direction

When a function's peak exceeds the region, give it an overflow area it owns — e.g. a frame-local
slot area addressed from `sp` (selection would have to request the bytes so `finalize_frame`
reserves and shifts them), or a second per-thread region addressed through a materialized base.
Either way the panic must become a working lowering, not a diagnostic.
