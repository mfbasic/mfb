# plan-130-C: the `arena` report section — per-arena counters for every thread

Last updated: 2026-09-12
Effort: large (3h–1d)
Depends on: plan-130-B

A `--debug` build counts, per arena, every allocator event the A/B tests in
`planning/todo.md` § Memory need: blocks mapped and unmapped, bytes, alloc and free
calls, which allocation path served each request, flushes, and the live-bytes
high-water mark. Every arena is counted — the main arena, every `thread::start`
worker arena, and the canvas graphics thread's arena — and the report lists each.

Behavioral outcome: for a program whose allocations are known, the `arena.<n>.*`
lines in the report match the known counts exactly; on linux-aarch64 the sum of
`arena.*.maps` equals the strace count of executable-IP anonymous `mmap`s for the same
run (the classification validated on 2026-09-12: yamljson `to-json` → 12).

References: plan-130-A (registry, report format); `src/docs/spec/memory/04_arenas.md`;
`src/codegen/memory/arena/arena.rs`; `.ai/canvas-threading.md` (arena state is
per-thread); memory notes `arena-state-is-per-thread`,
`a-leak-counter-must-cover-everything-it-guards`.

## Prerequisites

See plan-130-A § Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-130-B complete | `ls planning/completed/plan-130-B-*` → one match | NOT MET |

## 1. Goal

- Report lines per registered arena `n` (registration order, main arena = 0):
  `arena.<n>.kind main|worker|graphics`, `maps`, `mapped_bytes`, `unmaps`,
  `unmapped_bytes`, `alloc_calls`, `alloc_bytes`, `free_calls`, `free_bytes`,
  `live_bytes`, `peak_live_bytes`, `hit_quick_bin`, `hit_carve`, `hit_large_bin`,
  `hit_walk`, `grow`, `flushes`, `insert_free_calls`; plus `arena.count <n>` and
  `arena.registry_overflow <n>`.
- Counts are exact for the owning thread's events (no sampling, no lost increments).
- Non-`--debug` builds unchanged; **`ARENA_STATE_SIZE` and the arena-state layout do not
  change in either build**.

### Non-goals

- No change to allocator behavior: the same requests take the same paths in debug and
  normal builds (the counters observe, they do not steer).
- No per-call-site attribution (which source line allocated) — a later feature.
- No atomics: each counter is written only by the thread that owns the arena.

## 2. Current State

- Allocator helpers and their gates: `lower_arena_alloc(platform)`,
  `lower_arena_free(platform)`, `lower_arena_destroy(platform)` take the platform;
  `lower_arena_flush_coalesce()` and `lower_arena_insert_free()` take nothing
  (`src/codegen/memory/arena/arena.rs`). All are emitted from
  `lower_module_for_platform` (`src/codegen/engine/builder/mod.rs`).
- The only block map is `lower_arena_alloc`'s grow path (`platform.emit_arena_map`); the
  only unmap is `lower_arena_destroy`'s walk.
- Arena states: the main arena on the entry stack (`entry.rs`); worker arena blocks of
  `ENTRY_GLOBALS_OFFSET + arena_global_slots * 8` allocated by the parent in
  `lower_thread_start_helper` (`src/codegen/runtime/thread/runtime_helpers.rs`); the
  canvas graphics thread's child arena, same size formula
  (`src/codegen/runtime/canvas/mod.rs`).
- No list of arenas or threads exists (`MAIN_ARENA_GLOBAL_SYMBOL` doc: worker arenas are
  intentionally not tracked). A worker can still be running when shutdown runs.
- `_mfb_arena_destroy` zeroes arena state from offset 104 to `ARENA_STATE_SIZE`
  (`lower_arena_destroy`), before plan-130-A's report runs.
- Mutex shim: thread helpers call `pthread_mutex_lock`/`unlock`, mapped to
  `AcquireSRWLockExclusive`/`ReleaseSRWLockExclusive` on Windows by
  `emit_windows_thread_call` (`runtime_helpers.rs`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Code lines using `ARENA_STATE_SIZE` or the `ENTRY_*` offsets derived from it | 15 | `git grep -nwE "ARENA_STATE_SIZE\|ENTRY_SEED_SCRATCH_OFFSET\|ENTRY_STACK_SIZE\|ENTRY_GLOBALS_OFFSET" -- src ':!src/docs' \| grep -vE "^[^:]+:[0-9]+:\s*//" \| wc -l` |
| Places an arena state is created | 3 | entry (`entry.rs`), `lower_thread_start_helper`, canvas child arena (`canvas/mod.rs`) — `git grep -nw "ENTRY_GLOBALS_OFFSET + arena_global_slots" -- src` → 2, plus the entry |
| Allocation paths in `lower_arena_alloc` that return a pointer | 5 | quick-bin pop, carve (DV) serve, large-bin hit, walk split/DV take, grow (read of `arena.rs` 2026-09-12: labels `arena_alloc_ret` reached from each) — recount at Phase 2 |
| Free paths in `lower_arena_free` | 2 | quick-bin push, large-bin push (+ the two double-free early exits) |

### Verified properties

- *Why not add a word to the arena state:* 15 code lines derive from
  `ARENA_STATE_SIZE`, including the entry frame, the entry zero loop, the worker and
  canvas child sizes, the global-slot offsets baked into every global access
  (`builder/mod.rs`), and link thunks (`link_thunk.rs`). A debug-dependent size would
  thread `DebugOptions` into all of them and shift every global offset in debug builds.
  Rejected in §3.
- UNVERIFIED — that a keyed lookup of the arena's registry slot costs few enough
  instructions that a debug build of the benchmark rows stays within 2× of a normal
  build's wall time. Phase 1 measures it before any counter exists.

## 3. Design Overview

**A process-global debug arena registry, keyed by arena-state address.**

- `_mfb_rt_debug_arena_registry`: a region mapped once at entry through
  `platform.emit_arena_map` (like perf's region — never the arena), holding a header
  `{mutex (64 B), count, overflow}` and `ARENA_DEBUG_SLOTS = 1024` slots, each
  `{state_ptr, kind, counters[18]}` (160 B). Base stored in writable global
  `_mfb_rt_debug_arena_base` (raw 8 B).
- **Registration** (`_mfb_debug_arena_register(state, kind)`): lock, append a slot,
  unlock; overflow counts and later lookups find nothing (counts drop, reported). Called
  from the entry after the main arena is live, from `lower_thread_start_helper` for the
  worker block, and from the canvas child-arena setup.
- **Lookup** (`emit_debug_arena_slot(dst)`): linear scan of the registered slots
  comparing `state_ptr` with `ARENA_STATE_REGISTER`. Registered arenas per program are
  few (main + workers); Phase 1 measures the cost.
  - Rejected in favor of a hash only if Phase 1's measurement exceeds the bound.
- **Increments**: `emit_debug_arena_count(counter_offset, amount)` = lookup + load/add/
  store, emitted only when `module.debug.enabled`, at each event site. Only the owning
  thread (whose `x19` is the arena) writes its slot, so no lock is needed; registration
  is the only locked operation.
- **Report** (`_mfb_debug_report_arena`): reads `count` and each slot (a still-running
  worker's slot is read unlocked: word-sized reads, a snapshot, documented).

`lower_arena_flush_coalesce` and `lower_arena_insert_free` gain a `DebugOptions`
parameter (they have no platform today; the counters need only the flag).

**Risk:** the allocator hot path. Register clobbers are the danger — every counting site
sits inside vreg-allocated helpers whose live values (`size`, `eff_align`, results in
`RET[0]`/`RET[1]`) must survive; the counting emitter uses fresh vregs and no calls, so
the allocator's own spill model covers it. Proven by the exact-count tests and by the
full suite run in `--debug` for the arena-heavy rt tests.

**Design uncertainty:** lookup cost (Phase 1, measured first).

### Rejected alternatives

- *A counter word in the arena state* — layout churn across the 15 derived sites (§2).
- *Per-arena counters in a separate block pointed to from the arena state* — still needs
  a state word.
- *One process-global counter set* — loses per-thread attribution, and concurrent
  increments from several threads would race without atomics.
- *Counting inside `emit_arena_map`* — misses frees, paths and flushes.

## Phases

### Phase 1 — registry, registration, and the lookup-cost measurement

- [ ] `src/codegen/debug/arena.rs`: region layout constants, `_mfb_debug_arena_register`
      helper, `emit_debug_arena_slot`, `ArenaFeature` (report prints `arena.count`,
      `arena.registry_overflow`, and per-slot `kind`).
- [ ] Registration calls at the entry, `lower_thread_start_helper`, and the canvas child
      arena, all behind `module.debug.enabled`.
- [ ] Measure: a throwaway build of the benchmark rows cited in the arena.rs comments
      (bignum-modexp, datetime, large-list churn) with a no-op lookup emitted at every
      alloc/free entry vs a normal build; record wall times in Corrections. If the
      debug build exceeds 2×, replace the scan with an open-addressed hash before Phase 2.
- [ ] Test `tests/runtime/rt_debug_arena.rs`: a program starting 3 workers reports
      `arena.count 4` with kinds `main worker worker worker`; a canvas headless program
      reports a `graphics` arena.

Acceptance: test green on macOS; lookup-cost measurement recorded; artifact gate
`0 diff(s)`.
Commit: —

### Phase 2 — counters at every allocator event

- [ ] `lower_arena_alloc`: `alloc_calls`/`alloc_bytes` (normalized size) at entry of the
      valid path; one `hit_*` per return path; `grow`, `maps`, `mapped_bytes` in the grow
      path; `live_bytes += size` and `peak_live_bytes = max(...)` on success.
- [ ] `lower_arena_free`: `free_calls`/`free_bytes`, `live_bytes -= size` (not on the
      double-free early exits — count those as `double_free_skips`).
- [ ] `lower_arena_flush_coalesce`: `flushes`; `lower_arena_insert_free`:
      `insert_free_calls`; `lower_arena_destroy`: `unmaps`/`unmapped_bytes` per block.
- [ ] Report section writes every counter per slot.
- [ ] `tests/runtime/rt_debug_arena.rs` exact-count cases: (a) a loop allocating and
      freeing one 24-byte record N=1000 times → `alloc_calls == free_calls`,
      `hit_quick_bin >= 999`, `maps` unchanged between N=1000 and N=2000; (b) a single
      1 MiB `List OF Byte` → one `grow` and `mapped_bytes >= 1 MiB`; (c) the control
      required by memory `a-leak-counter-must-cover-everything-it-guards`: a program
      that drops a value without freeing (a known leak shape) → `live_bytes` grows with N.

Acceptance: `cargo test --release --test rt_debug_arena` green; artifact gate
`0 diff(s)`.
Commit: —

### Phase 3 — cross-target proof against strace

- [ ] Build `examples/yaml-json` with `--debug --target linux-aarch64`; on 2223 run
      `to-json samples/config.yaml` under the 2026-09-12 strace harness; assert
      `sum(arena.*.maps) == ` executable-IP anonymous mmap count. Record both.
- [ ] Same program's report on 2227, 2228, 2229, 2230 (Windows: no strace; record the
      report and compare `maps` with the macOS/Linux value for the same input).
- [ ] Browser example (`examples/browser`, packages rebuilt from source) on 2223 with
      `--debug`: record the report for the Wikipedia load as the first real data point
      for `planning/todo.md` § Memory.

Acceptance: strace equality holds on 2223; reports recorded for all five targets.
Commit: —

### Phase 4 — docs

- [ ] Debug-report spec page: the `arena` section, counter meanings, snapshot caveat.
- [ ] `src/docs/spec/memory/04_arenas.md`: a short note that `--debug` counts these events.
- [ ] `planning/todo.md` § Memory § 1: tick the harness items this delivers.
- [ ] Full suite + artifact gate + test-accept as in plan-130-A Phase 4.

Acceptance: the three suites green; `citations_resolve` green.
Commit: —

## Validation Plan

- Tests: `tests/runtime/rt_debug_arena.rs` (registration, exact counts, leak control).
- Coverage check: the counting emitter is reached only by `--debug` builds, which only
  `rt_debug_arena.rs` and `rt_debug_report.rs` produce — confirmed by grepping their
  build commands for `--debug`.
- Runtime proof: Phase 3 strace equality on 2223 plus reports on all five targets.
- Doc sync: debug-report spec page, `04_arenas.md` note, `planning/todo.md`.

## Open Decisions

- **Registry capacity** — recommended 1024 slots with an overflow counter; programs
  starting more threads than that lose per-arena counts beyond it (reported).
- **Counting a worker's events after it finished** — recommended: the slot persists (the
  arena never unmaps before exit, per `04_arenas.md` today), so its final counts report.

## Corrections

## Summary

The allocator hot path is the risk; the design keeps the arena layout untouched by
putting all state in a debug-only registry, and measures the one uncertain cost (slot
lookup) before any counter is placed.
