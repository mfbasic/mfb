# plan: error-block-in-slot (design "b") — single-owner error propagation

Goal: carry a propagating error as ONE owned flat Error block, moved by pointer
through a per-thread "current error" slot and *adopted* (not rebuilt) by whoever
catches it. Kills the interior-pointer + deep-copy + rebuild scheme that roots
bug-152, the re-raise/propagation orphans, and 147.5(b)'s cousin. No hot-path cost
(the slot is touched only on error paths; not a lock/atomic).

## Current model (what we're replacing)
- Error `Result` travels in 4 return registers: `RET[0]=tag`, `RET[1]=code`,
  `RET[2]=message*` (interior ptr), `RET[3]=source*`/ErrorLoc (interior ptr).
- The flat Error block `{code@0, message_off@8, source_off@16, [inlined msg][inlined src]}`
  is built ONLY at the consumer: `route_current_result_to_trap` calls
  `emit_build_error_inline` to materialize `e`. Re-raise deep-copies (store_pending_
  error_from_value → lower_value_owned → copy_flat_block) then orphans the copy.
- `lower_value_owned(Error Local/ResultError)` already returns a STANDALONE block
  base (copy_flat_block). That standalone block is the thing we'll move+adopt.

## New model
- Per-thread slot `ARENA_CURRENT_ERROR_OFFSET` holds the in-flight owned Error
  block base (0 when none).
- New tag `RESULT_ERR_BLOCK_TAG = 3` = "error; adopt the block in the slot."
- Producer that has a real block stores base→slot, sets tag=ERR_BLOCK (and, during
  migration, ALSO fills the legacy message/source registers so non-adopting
  consumers still work).
- Consumer trap route: tag==ERR_BLOCK → adopt slot base as `e`, clear slot; else
  (tag==ERR) rebuild from registers (unchanged legacy path).
- Adopted block is a normal owned value → freed once by the catching handler
  (bug-151 machinery). Single owner at every instant, created once, freed once.

## Safety invariant
One in-flight error per thread; propagation is sequential (no async/coroutines
within a thread). The catching trap route adopts+clears promptly. Only async
reentrancy is signal handlers — verify they never run FAIL/trap machinery
(`_mfb_shutdown`/SIGINT-SIGTERM are teardown only). Dual-tag makes migration safe:
an unmigrated producer still sets tag=ERR (rebuild), never a stale slot.

## Stages (each: build + full acceptance + revert on any regression)
1. **Infra** — DONE (14de6012): `ARENA_CURRENT_ERROR_OFFSET` + `RESULT_ERR_BLOCK_TAG`.
2. **Adopt in trap route** — DONE (14de6012): `route_current_result_to_trap` handles
   ERR_BLOCK (adopt+clear) alongside the legacy rebuild.
3. **FAIL path produces a block** — DONE (14de6012): `store_pending_error_from_value`
   / `emit_direct_error_return` park the deep-copied standalone base→slot + ERR_BLOCK
   for aliasing sources (re-raise). Fixes bug-152.
4. **Migrate domain errors** — DONE: every `emit_error_register_return` domain error
   (index-out-of-range, overflow, `FAIL error(...)`, inline-builtin failures, …)
   now runs the `emit_park_error_block_from_registers` funnel: build the flat block
   once from the loose registers, park base→slot, tag=ERR_BLOCK. The catcher ADOPTS.
   Consumer parity added: `materialize_current_result` adopts the parked block (and
   frees the single owner after copying it into the materialized `Result`) so an
   inline `TRAP` no longer orphans it.
5. **Migrate raw runtime helpers + worker errors** — DONE: `emit_stamp_current_error_source`
   (all fs/io/net/os/tls/term/link/crypto/… helper call sites) and
   `emit_finalize_worker_error_source` (thread::waitFor) run the same funnel. Because
   every raw helper propagates through the ONE call-site stamp, this migrates the whole
   fleet with a single funnel insertion rather than touching ~150 producer sites.
6. **147.5(b)** — DONE separately (8fb7d59a): failed thread-send message reclamation
   via the dest-drain list.

### Findings that bound stage 5's "drop the rebuild path + registers"
- **OOM keeps the legacy loose-register path.** An allocation failure *while building
  the error block* cannot itself park a block; it must fall back to a static-message
  loose `RESULT_ERR_TAG` that the catcher rebuilds. So `emit_build_error_inline` (the
  rebuild) is retained as the OOM safety net, and the funnel is guarded by
  `building_error_block`/`emitting_error_route` so the OOM fallback never recurses into
  another park. Net effect: every **non-OOM** error is now block-carried and adopted
  exactly once; the rebuild path survives solely for OOM/allocation-failure.
- **The message/source registers are retained**, not dropped: the top-level exit
  printer reads `code`(x1)/`message`(x2) from them, the OOM path needs them, and the
  funnel itself consumes them to build the block. They are no longer the propagation
  mechanism (the block is) — just the printer/OOM carrier — so the "weird" dual
  *adopt-vs-rebuild* split for normal errors is gone even though the registers remain.
- **`FAIL error(...)` with live cleanups** (the `store_pending_error_from_value`
  fresh-non-aliasing branch) is deliberately left on the legacy rebuild path: the
  stages-1-3 authors flagged a double-free risk in parking that already-pending temp,
  and it is an exceptional-path minor orphan, so it is not worth a memory-safety gamble.

### Validated
- Full local acceptance (913 tests) + artifact gate green (5 native goldens refreshed:
  control_flow_if, parser_hello_world, control_flow_match — the error paths that now
  emit the park sequence).
- Hardware, all 4 remotes (2223 aarch64 glibc, 2227 x86_64 musl, 2228 x86_64 glibc,
  2229 riscv64 musl): bug152 re-raise (flat RSS), multi-frame domain propagation,
  raw-helper propagation, inline-TRAP materialize-adopt — all correct, no crash/leak.
- Regression test: tests/rt-behavior/trap/error_block_adopt_paths.

## Validation gates
- `scripts/artifact-gate.sh` / full `scripts/test-accept.sh` after each stage.
- rt-error + trap suites are the behavioral guard (crash = double-free/UAF).
- Leak regression: bug151/bug152 fixtures RSS-flat.
- 4 remotes: 2223 Kali aarch64, 2227 Alpine x86_64 musl, 2228 Ubuntu x86_64 glibc,
  2229 Alpine riscv64 musl.
- Revert any stage that can't pass its gate.
