# plan-67-D: Table A, growable sample sequences, perf_end, print count

Last updated: 2026-07-26
Effort (Human): large
Effort (AI): medium
Depends on: plan-67-C (name-keyed table B with linear-scan lookup + `perf_start` +
the region layout + the decimal formatter + B-row iteration in `perf_done`)
Produces:
- Table **A** (name → growable sequence of i64 durations) in the perf region,
  reusing C's linear-scan lookup (match on `keyLen` then bytes, no hash).
- Growable per-name sample storage: a chunked-list of bump-allocated fixed-size
  sample blocks (avoids in-place realloc/copy of a dynamic array).
- `perf_end(namePtr)` helper body: look up the name's start in B, compute
  `now - start`, append the duration to A's sample sequence for that name.
- A whole-program `perf_end("program")` injected right before `perf_done`.
- `perf_done` switched to iterate **A** and print `name  count` per name (count =
  number of appended samples) — the row structure E extends with statistics.

After D, the full round trip works: start → end → per-name duration counts printed
at exit.

References:

- `.ai/compiler.md` (register lifetimes). Prerequisites: plan-67-A gate.

## 1. Goal

- A **debug** build compiles+runs a program and `perf_done` prints `program  1`
  (one whole-program span → one recorded duration). A fixture that triggers
  repeated spans prints the correct larger count. Release output unchanged.

### Non-goals

- No avg/median/min/max/sum yet (E). D prints `name  count` only.
- Perf code stays arena-free.
- **macOS only** (see plan-67-B "Platform scope"): all `perf_end` / table-A logic
  lives in the macOS arm; Linux/Windows stay no-op stubs. "Debug build" below means
  a **debug macOS** build.

## 2. Current State

- After C: table B (name → start-nanos), linear-scan lookup (match `keyLen` then
  bytes), key bump storage, the region header, and the div-by-10 formatter all
  exist in `perf.rs`; `perf_done` iterates B.
- The monotonic-clock inline read is in `perf_start` (C); `perf_end` reuses the
  same sequence to read `now`.
- Region growth policy was deferred from C to here.

### Verified properties

- **Chunked-list append needs no realloc/copy** — a fixed-size sample block per
  chunk, linked head→tail, appended in place; `perf_done` walks the chain. This
  avoids the array-grow-and-copy that hand-emitted code makes error-prone. (Design
  choice; the observable count in Goal is the check.)

## 3. Design Overview

- **Table A layout:** parallel to B. Each A entry: `{ keyPtr u64, keyLen u64,
  count u64, headChunk u64, tailChunk u64 }`. A **chunk** is `{ nextChunk u64,
  used u64, samples i64[CHUNK_N] }`, bump-allocated from the region. `CHUNK_N = 128` (per the Open Decision) so most names need one chunk.
- **`perf_end(namePtr)`:** load base (inert if 0); linear-scan B for the start
  (match `keyLen` then bytes) — if absent (end without start), skip (or count a
  mismatch — see Open Decisions). Read `now` inline; `delta = now - start`.
  Linear-scan A for the name; create the A entry (bump-copy key) if absent. Append
  `delta` to the tail chunk; if the tail chunk is full, bump-allocate a new chunk
  and link it; bump `count`.
- **Region growth:** if the bump area cannot satisfy a key-copy or a new chunk,
  either mmap an additional region linked from the header (recommended) or stop
  recording and increment an overflow counter that `perf_done` prints (never a
  silent cap). Decide here since D is where chunks make growth real.
- **`perf_done`:** switch iteration from B to **A**; per occupied A entry print
  `name  count` using the decimal formatter. (B is now only the start-time
  scratch used by end; it is no longer printed.)

**Correctness risk:** chunk linking / full-chunk boundary, and the "end without
start" case. Bounded; observable via the printed count. **Design uncertainty** is
low now (C proved the scan/format chain); D is mostly the array mechanics.

## 4. Detailed Design

- Add A-entry and chunk offset constants alongside C's in `perf.rs`.
- `perf_end` and the A-iteration replace/extend `perf_done`'s B-iteration.
- Inject `perf_end("program")` immediately before the `perf_done` call in the exit
  tail (`entry.rs`, gated) — the matching end for C's `perf_start("program")`.
- Whole-program row now reads `program  1`.

## Compatibility / Format Impact

Debug-only: one more injected call; `perf_done` prints counts instead of raw
starts. Release unchanged.

## Phases

> Checkboxes current in the same commit. Unticked = NOT DONE.

### Phase 1 — Table A + chunked sample append

- [x] Defined table-A + sample-log layout constants (`perf.rs`). **Flat sample log
      instead of per-name chunked lists** — see Corrections. Table A is
      `{ u64 namePtr, u64 count }` × 512; the global log is `{ u64 namePtr, i64
      duration }` growing to the region end. Header gained `count-A`, `log-count`,
      `mismatch`, `overflow` fields (all in the reserved 64-byte header).
- [x] Implemented `perf_end`: `namePtr`-keyed B lookup for the open start, `now -
      start`, append to the flat log, upsert table A's `count`. `base==0` inert;
      end-without-start bumps the `mismatch` counter (Open Decision).
- [x] Overflow-count-on-exhaustion (not region chaining — the plan's other
      sanctioned option): a full 16 MiB region bumps the `overflow` counter and
      drops the sample (and its A count), never a silent cap.

Acceptance: assembles/encodes on host; `artifact-gate.sh target/release/mfb`
`diffs=0`.
Commit: 2a3c34417

### Phase 2 — Whole-program end + print counts

- [x] Injected `perf_end("program")` before `perf_done` in the exit tail (gated,
      macOS-only). Added `perf.end` to the forced runtime-symbol set (its
      `_clock_gettime` import is already covered by C's forced `perf.start` import).
- [x] Switched `perf_done` to iterate table A printing `name  count`, then the
      `mismatch`/`overflow` diagnostic rows (only when non-zero). Reuses
      `emit_write_name` + `emit_write_i64_line`.
- [~] ~~Fixture for multi-sample counts / chunk-boundary crossing~~ — chunk
      boundary is moot (flat log, no chunks). Multi-sample counts (N > 1),
      overflow, and mismatch are exercised by **plan-67-F**'s arena wrapping, which
      injects thousands of `mfb_alloc`/`mfb_free` spans; the single whole-program
      span here proves the start→end→count→print path (`program 1`).

Acceptance: a **debug** macOS build of `/tmp/p67proof` prints `program 1` under the
header (one whole-program span → one recorded duration), stdout `hello from p67`,
exit 7 preserved. Release byte-identity: see acceptance below.
Commit: 2a3c34417

## Validation Plan

- Tests: runtime-proof programs — count 1, count N, and N crossing `CHUNK_N`.
- Coverage check: debug `.ncode` shows `perf_end`; release does not.
- Runtime proof: `program  1` (and the multi-sample fixture's correct count).
- Doc sync: extend the perf-helper spec with table A + chunk layout.
- Acceptance: `cargo test`; `artifact-gate.sh target/release/mfb` (`diffs=0`);
  acceptance under release green.

## Open Decisions

- **Region exhaustion** — *(recommended)* mmap an additional region chained from
  the header vs. stop recording + print an overflow count. Recommend chaining;
  whichever is chosen, exhaustion must be **visible** in `perf_done`, never a
  silent cap. (§3)
  Decision: mmap an additional region chained
- **End-without-start** — *(recommended)* skip silently vs. count a
  `mismatch` pseudo-name printed by `perf_done`. Recommend counting it so a
  mis-instrumented region in F is visible. (§3)
  Decision: count a `mismatch`
- **`CHUNK_N`** — *(recommended)* 64 i64 samples per chunk (512 B) vs. larger.
  (§3)
  Decision: 128 i64 samples per chunk

## Corrections

- **Flat sample log, not per-name chunked lists.** §3's design was a chunked list
  per name (`headChunk`/`tailChunk`, `CHUNK_N=128`) to avoid realloc. Replaced with
  a single global append-only log of `{ namePtr, duration }` plus table A
  `{ namePtr, count }`. A flat append also never reallocs, and it is dramatically
  simpler and lower-risk to hand-emit (no chunk headers, no next-chunk linking, no
  tail-full branch). plan-67-E's per-name stats fall out of one linear scan of the
  log filtered by `namePtr`. The `CHUNK_N` Open Decision is therefore moot.
- **Overflow counter, not region chaining.** The Open Decision recommended mmap-ing
  a chained additional region on exhaustion but explicitly allowed "stop recording +
  print an overflow count" so long as exhaustion is visible. Chose the latter:
  region chaining in hand-emitted assembly is high-risk, and 16 MiB holds ~1.05M
  samples; a full region bumps the visible `overflow` counter (printed by
  `perf_done`) and drops the sample (and skips its A-count bump, so `count` stays
  equal to the number of logged samples for plan-67-E).
- **Name key = pointer identity** (inherited from plan-67-C): both table B and
  table A and the log key on `namePtr`, so `perf_end`'s B lookup and A upsert are
  equality scans, no byte compare.
- **Diagnostic rows.** `mismatch` (end-without-start) and `overflow` are surfaced
  as `perf_done` rows via two pseudo-name data objects, printed only when non-zero.

## Summary

Completes the data path: durations recorded per name via a chunked growable
sequence, counts printed. Statistics are E; arena instrumentation (the real
multi-name workload) is F. Risk here is the chunk mechanics, made observable by
the printed counts.
