# plan-67-C: Name-keyed table B (linear scan), perf_start, print B rows

Last updated: 2026-07-26
Effort (Human): large
Effort (AI): medium
Depends on: plan-67-B (Perf family, region global `_mfb_rt_perf_state`, `perf_init`
region, `perf_done` header, gating predicate, injection sites)
Produces:
- An internal name-keyed table (in the perf region, not the arena, not MFB `Map`):
  a flat array of entries, looked up by **linear scan comparing `keyLen` then the
  key bytes — no hash function**. Key bytes are bump-copied into the region. (The
  name set is tiny — the arena regions plus `"program"` — so a linear scan is
  simpler and cheaper than hashing, and removes an entire class of hash/probe
  bugs.)
- Table **B** (name → i64 start-nanos) living in the region.
- `perf_start(namePtr)` helper body: read the name string block, upsert B with the
  current monotonic-nanos timestamp ("add / update / overwrite" per spec).
- A whole-program span: `perf_start("program")` injected right after `perf_init`.
- `perf_done` extended to iterate B and print one row per name (name + raw
  start-nanos), using a hand-emitted decimal formatter — the reusable table-row +
  integer-formatting machinery D/E extend.

This letter makes the table machinery real and observable through B; table A and
per-name sample arrays arrive in D.

References:

- `.ai/compiler.md` (register lifetimes; a `bl _mfb_*` clobbers x0–x17).
- Prerequisites: plan-67-A gate. Do not restart.

## 1. Goal

- A **debug**-built compiler compiles+runs a program and `perf_done` prints a row
  `program  <n>` where `<n>` is the monotonic-nanos value stored by
  `perf_start("program")` — proving the name-keyed table insert (linear scan by
  length+bytes), key storage, integer formatting, and row iteration all work.
  Release output unchanged.

### Non-goals

- No durations yet (table A / `perf_end` are D). The B row prints the raw start
  timestamp only, as a visible proof of the stored value.
- Perf code touches only the region, the monotonic clock, and `emit_write` — never
  the arena.
- **macOS only** (see plan-67-B "Platform scope"): all `perf_start` / table-B logic
  here lives in the macOS arm of `lower_perf_helper`; Linux/Windows stay no-op
  stubs. "Debug build" in every acceptance criterion below means a **debug macOS**
  build.

## 2. Current State

- **No internal lookup structure exists** in the runtime; every "hash"/"map" is the
  arena-backed MFB `Map` collection, unusable here (VERIFIED during planning:
  research over `map_mutate.rs`, `builder_collection_layout.rs`). This letter
  builds the first runtime-internal name-keyed table by hand (linear scan, no hash).
- **Monotonic clock:** `_mfb_rt_datetime_monotonicNanos` reads
  `clock_gettime(CLOCK_MONOTONIC)` (ids at `datetime.rs:32-33`) / Windows
  `QueryPerformanceCounter` (`datetime.rs:242-269`), returning i64 nanoseconds.
  `perf_start` reads the clock **itself** (per the design: the helper, not the
  caller, reads time), so replicate this sequence inline in `perf.rs` rather than
  `bl`-ing the datetime symbol (keeps perf self-contained and arena-free).
- **String arg representation:** the `namePtr` arrives as a pointer to an
  `mfb.string.v1` block — length at `[ptr+0]`, bytes at `ptr+8`, trailing NUL
  (`data_objects.rs:640-648`); read it the way fs helpers do (`fs/atomic.rs:61-64,
  81-82`). Injected literal names are emitted as string data objects and their
  address loaded with `emit_load_string_constant` /
  `emit_load_static_string_symbol` (`builder_emit_helpers.rs:126-172`).
- **Integer→decimal:** no reusable *symbol*; replicate the SCRATCH-pool div-by-10
  digit loop from `emit_write_integer_to_stderr_with_labels`
  (`entry.rs:1025-1044`), which writes digits into a stack scratch window then
  `emit_write`s them. The builder-method `emit_integer_to_string_value`
  (`builder_strings.rs:873-1011`) is arena-allocating and builder-bound — **do not
  use it** (it allocates an arena String).
- **Region:** allocated and based in `_mfb_rt_perf_state` by plan-67-B.

### Verified properties

- **The monotonic-clock sequence is inlineable without the arena** — VERIFIED by
  reading `datetime.rs:106-141,242-269`: it is a libc/QPC call returning an i64,
  no arena touch.
- **The decimal loop at `entry.rs:1025` uses the neutral SCRATCH pool** (no
  register allocator, no arena) — VERIFIED; it is the correct pattern for a
  hand-emitted helper body.

## 3. Design Overview

- **Region header + table B layout** (define the region layout here; D adds A):
  a small header (magic, capacity, count-B, bump-cursor, bump-end) followed by
  table B's entry array and the key-bytes bump area. Each B entry: `{ keyPtr u64
  (into bump area), keyLen u64, startNanos i64 }` (an empty entry is `keyPtr == 0`).
  Entries are packed `[0..count-B)`; lookup is a **linear scan** over them. Fixed
  generous capacity; if `count-B` would exceed it, either grow (mmap more —
  coordinate with D) or record an overflow count (printed by `perf_done`, never
  silent). Real growth policy decided in D.
- **Key lookup (no hash):** scan entries `[0..count-B)`; an entry matches when
  `keyLen` equals the name length **and** the `keyLen` bump-stored bytes equal the
  name bytes at `ptr+8` (compare length first, then bytes). No hash field, no
  probing.
- **`perf_start(namePtr)`:** load base; if 0 return. Read len/bytes; linear-scan B
  — if a matching entry exists, overwrite `startNanos`; if none, bump-copy the key
  bytes, fill entry `[count-B]`, bump count-B (upsert = "add / update / overwrite").
  Read monotonic nanos inline; store into the entry.
- **`perf_done` row iteration:** walk B's entries `[0..count-B)`; for each, write
  the name bytes (`emit_write` of `keyPtr..keyPtr+keyLen`), a separator, then the
  decimal of `startNanos`, then `\n`. Reuse the div-by-10 scratch formatter.

**Correctness risk:** off-by-one in the scan/entry stride, and mis-reading the
string block length. Bounded and testable via the visible B row. **Design
uncertainty:** whether the whole hand-emitted scan+bump+format chain works on the
host — the visible `program <n>` row is the falsifier.

## 4. Detailed Design

- Constants for the region header offsets and B-entry stride go in `perf.rs` (or
  `error_constants.rs` if shared with D). Document each offset.
- `perf_start` and the B-iteration in `perf_done` are added to `lower_perf_helper`
  (`perf.rs`), reusing B's frame scaffolding.
- Whole-program span injection: emit `perf_start("program")` immediately after the
  `perf_init` call in `lower_program_entry` (gated). The matching
  `perf_end("program")` is added in D (until then, B holds a start with no end,
  and `perf_done` prints the raw start — that is the intended C-phase observable).

## Compatibility / Format Impact

Debug-only: one more injected call and a real B row at exit. Release unchanged.

## Phases

> Checkboxes current in the same commit. Unticked = NOT DONE.

### Phase 1 — Linear-scan table B insert/lookup

- [ ] Define the region header + B-entry layout constants (`perf.rs`).
- [ ] Implement linear-scan insert/lookup over B in `perf_start` (match on `keyLen`
      then bytes, no hash), with key bytes bump-copied. Handle the "region base is
      0" inert path.
- [ ] Read the name `mfb.string.v1` block correctly (len `[ptr+0]`, bytes `ptr+8`).

Acceptance: assembles/encodes on host; `artifact-gate.sh target/release/mfb`
`diffs=0` (release inert).
Commit: —

### Phase 2 — Inline monotonic clock + whole-program span + print B

- [ ] Inline the `CLOCK_MONOTONIC` read in `perf_start`; store into the entry.
- [ ] Extend `perf_done` to iterate B and print `name  <startNanos>` rows via the
      div-by-10 scratch formatter (pattern from `entry.rs:1025-1044`).
- [ ] Inject `perf_start("program")` after `perf_init` in `lower_program_entry`
      (gated); emit the `"program"` string data object.

Acceptance: a **debug** build compiles+runs a trivial program and prints a
`program  <nonzero-nanos>` row under the header; release build byte-identical
(`diffs=0`, acceptance green under release).
Commit: —

## Validation Plan

- Tests: runtime-proof program (debug) shows the `program` row with a plausible
  monotonic value; a second name (add a temporary second injected span or a
  fixture) shows two distinct rows — proving insert vs. overwrite.
- Coverage check: debug `.ncode` dump shows the `perf_start` call; release dump
  does not.
- Runtime proof: `program  <n>` printed at exit under debug.
- Doc sync: update the perf-helper spec section (region layout, linear-scan
  lookup) added in B.
- Acceptance: `cargo test`; `scripts/artifact-gate.sh target/release/mfb`
  (`diffs=0`); acceptance under release green.

## Open Decisions

- **Table B capacity / growth** — *(recommended)* fixed generous power-of-two
  capacity in C, defer real growth to D (mmap-another-region), and have
  `perf_done` print an overflow count if the table ever saturates (never a silent
  cap). (§3)
  Decision: fixed generous power-of-two
- **Key comparison** — compare `keyLen` then bytes, or add a `keyHash` prefilter.
  (§3)
  Decision: `keyLen` then bytes, no hash needed.

## Corrections

<Filled in during execution.>

## Summary

Builds the runtime's first internal name-keyed table by hand (linear scan by
length+bytes, no hash) and proves it through a visible B row. The array/duration
half is D; this letter's risk is entirely in the scan/format machinery, made
observable rather than assumed.
