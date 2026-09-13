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
| plan-130-B complete | `ls planning/completed/plan-130-B-*` → one match | MET (2026-09-12: plan-130-B archived after Phases 1–3, `fedd21989` + `5d9463bee`; its full-suite gate moved to plan-130-E Phase 3 per the user) |

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

- [x] `src/codegen/debug/arena.rs`: region layout constants, `_mfb_debug_arena_register`
      helper, `ArenaFeature` (report prints `arena.count`, `arena.registry_overflow`, and
      per-slot `kind`). The registry lock is the static `_mfb_rt_debug_arena_lock` (see
      Corrections); line assembly reuses new `write.rs` pieces (`emit_prepend_decimal`,
      `emit_prepend_object`, `emit_write_window`).
- [x] ~~`emit_debug_arena_slot` in Phase 1~~ — moved to Phase 2 with its first consumer (the
      counters): an emitter nothing calls fails the warning-free tree (Corrections).
- [x] Registration calls at the entry, `lower_thread_start_helper`, and the canvas child
      arena, all behind `module.debug.enabled`. Entry: `ArenaFeature::emit_entry_start` (kind main). Workers: the parent in
      `lower_thread_start_helper`, after the child arena is zeroed (kind worker). Graphics:
      `emit_start_graphics`, before the spawn (kind graphics). Both reached through
      `AbiCtx.debug_arena_registry`; mutex imports attributed to the entry via
      `DebugFeature::lock_helpers`. All four `--debug` cross-builds link (object plans accept).
- [x] Measure: a throwaway build of the benchmark rows cited in the arena.rs comments
      (bignum-modexp, datetime, large-list churn) with a no-op lookup emitted at every
      alloc/free entry vs a normal build; record wall times in Corrections. If the
      debug build exceeds 2×, replace the scan with an open-addressed hash before Phase 2.
      Measured 2026-09-12 (numbers in Corrections): a no-op registry slot lookup at every
      `_mfb_arena_alloc`/`_mfb_arena_free` entry costs geomean x1.007 on linux-aarch64 (2223, no
      perf) against a normal build, worst row above the noise floor x1.335 — far under 2x, so
      the linear scan stays and no hash is needed.
- [x] Test `tests/runtime/rt_debug_arena.rs`: a program starting 3 workers reports
      `arena.count 4` with kinds `main worker worker worker`; a canvas headless program
      reports a `graphics` arena. `cargo test --release --no-fail-fast --test rt_debug_arena` -> 2 passed
      (`three_workers_register_four_arenas`: exactly `arena.count 4`, `registry_overflow 0`,
      `0 main`, `1-3 worker`; `the_canvas_graphics_thread_registers_a_graphics_arena`: exactly
      `arena.count 2`, `0 main`, `1 graphics`). `rt_debug_report` -> 7 passed after its block
      check learned that a report has several sections (Corrections); `codegen::debug` unit
      tests -> 4 passed; artifact gate 0 diff(s).

Acceptance: test green on macOS; lookup-cost measurement recorded; artifact gate
`0 diff(s)`.
Commit: cf5b640c9

### Phase 2 — counters at every allocator event

- [x] (moved here from Phase 1) `emit_debug_arena_slot(dst)`: linear scan of the registered
      slots comparing `state_ptr` with the arena register — the lookup every counter uses.
- [x] `lower_arena_alloc`: `alloc_calls`/`alloc_bytes` (normalized size) at entry of the
      valid path; one `hit_*` per return path; `grow`, `maps`, `mapped_bytes` in the grow
      path; `live_bytes += size` and `peak_live_bytes = max(...)` on success.
- [x] `lower_arena_free`: `free_calls`/`free_bytes`, `live_bytes -= size` (not on the
      double-free early exits — count those as `double_free_skips`).
- [x] `lower_arena_flush_coalesce`: `flushes`; `lower_arena_insert_free`:
      `insert_free_calls`; `lower_arena_destroy`: `unmaps`/`unmapped_bytes` per block.
- [x] Report section writes every counter per slot (`arena.<n>.<counter> <value>`, 18 per
      arena, after its `kind`).
- [x] `tests/runtime/rt_debug_arena.rs` exact-count cases: (a) a loop allocating and
      freeing one 24-byte record N=1000 times → `alloc_calls == free_calls`,
      `hit_quick_bin >= 999`, `maps` unchanged between N=1000 and N=2000; (b) a single
      1 MiB `List OF Byte` → one `grow` and `mapped_bytes >= 1 MiB`; (c) the control
      required by memory `a-leak-counter-must-cover-everything-it-guards`: a program
      that drops a value without freeing (a known leak shape) → `live_bytes` grows with N.
      Implemented as (see Corrections): `churn_counts_scale_with_iterations_and_partition_by_path`
      (N=1000 vs N=2000: +>=1000 allocations and frees, +>=999 quick-bin hits, `maps` equal,
      and `alloc_calls == hit_quick_bin + hit_carve + hit_large_bin + hit_walk + grow` in both);
      `a_mebibyte_read_grows_the_arena` (`grow >= 1`, `mapped_bytes >= 1 MiB`, `maps == unmaps`);
      `retained_values_raise_peak_live_bytes_and_churn_does_not` (the control).
      `cargo test --release --no-fail-fast --test rt_debug_arena` -> 5 passed;
      `rt_debug_report` -> 7 passed; `cargo test --bin mfb -- codegen::debug perf arena` -> 27
      passed; artifact gate `0 diff(s)` (2011 goldens).

Acceptance: `cargo test --release --test rt_debug_arena` green; artifact gate
`0 diff(s)`.
Commit: 99ce67004

### Phase 3 — cross-target proof against strace

- [x] Build `examples/yaml-json` with `--debug --target linux-aarch64`; on 2223 run
      `to-json samples/config.yaml` under the 2026-09-12 strace harness; assert
      `sum(arena.*.maps) == ` executable-IP anonymous mmap count. Record both.
      2223 has no strace and no sudo; strace 7.0 was unpacked unprivileged (`apt-get download
      strace` + `dpkg -x`, `~/strace-local/root/usr/bin/strace`). `strace -f -i -e
      trace=mmap,munmap ./yamljson-glibc.out to-json config.yaml`: executable-IP (`0xaaaa…`)
      anonymous `mmap`s = 13 — 12 from one call site (the arena grow) plus 1 of 163,856 bytes
      from another (the `--debug` registry map: 16 + 1024 × 160, not an arena block); 12
      executable-IP `munmap`s. Report: `arena.0.maps 12`, `arena.0.unmaps 12`,
      `arena.0.grow 12`. Equality holds once the registry map is excluded (see Corrections).
      The musl binary cannot run on 2223 (no `/lib/ld-musl-aarch64.so.1`); musl is covered on
      2227 and 2229.
- [x] Same program's report on 2227, 2228, 2229, 2230 (Windows: no strace; record the
      report and compare `maps` with the macOS/Linux value for the same input).
      All exit 0 and hold `alloc_calls == quick + carve + large + walk + grow`:
      | target | maps/unmaps | mapped_bytes | alloc_calls | free_calls | peak_live | quick/carve/large/walk/grow |
      | macos-aarch64 (local) | 12/12 | 57,344 | 5,972 | 5,656 | 45,488 | 5578/380/0/2/12 |
      | linux-aarch64 glibc (2223) | 12/12 | — | — | — | — | (maps from the strace run) |
      | linux-x86_64 musl (2227) | 12/12 | 57,344 | 5,972 | 5,656 | 45,328 | — |
      | linux-x86_64 glibc (2228) | 12/12 | 57,344 | 5,972 | 5,656 | 45,328 | 5574/384/0/2/12 |
      | linux-riscv64 musl (2229) | 12/12 | 57,344 | 5,972 | 5,656 | 45,328 | — |
      | windows-x86_64 (2230) | 12/12 | 516,096 | 5,977 | 5,656 | 504,112 | 5574/385/0/6/12 |
      `maps` is 12 everywhere. Windows makes 5 more allocations holding ~458 KB live at exit
      (`alloc_bytes` 748,704 vs 290,256 on macOS), so its grow blocks are larger; the
      counters stay self-consistent (partition holds, `live_bytes` ≤ `mapped_bytes`).
- [x] Browser example (`examples/browser`, packages rebuilt from source) on 2223 with
      `--debug`: record the report for the Wikipedia load as the first real data point
      for `planning/todo.md` § Memory.
      Driven in tmux (`G`, `https://en.wikipedia.org/wiki/Main_Page`, Enter). The program exits
      1 within 8 s with `List or string index/range is outside valid bounds.`; a normal build
      does the same and `https://example.com` loads and quits cleanly in both (see Corrections).
      The report, identical in two runs to within 33 main-arena allocations:
      | arena | maps | mapped_bytes | alloc_calls | alloc_bytes | free_calls | live at exit | peak_live | quick/carve/large/walk/grow |
      | 0 main | 15,238 | 173,797,376 | 261,097 | 260,068,544 | 167,935 | 112,061,104 | 114,458,944 | 168235/68938/5716/2970/15238 |
      | 1 worker | 192,271 | 895,033,344 | 109,560,293 | 4,006,567,520 | 67,918,822 | 841,424,912 | 842,257,664 | 67339942/41999021/25900/3159/192271 |
      The worker makes 109.6 M allocations (4.0 GB requested) for one page and keeps
      841 MB live when it ends, and the main arena receives 112 MB (the deep copy).
      `flushes 0` and `insert_free_calls 0` in both arenas, and 38% of the worker's
      allocations are carves, so free-list coalescing is not where the memory goes: the
      live volume is. `unmaps 0` because the program exits through the error path.

Acceptance: strace equality holds on 2223; reports recorded for all five targets.
Commit: abb7246a3

### Phase 4 — docs

- [x] Debug-report spec page: the `arena` section, counter meanings, snapshot caveat.
      `src/docs/spec/tooling/09_debug-report.md` § Sections: 11 `arena` rows, a paragraph
      on the snapshot and the `live_bytes` clamp, cited to `ArenaFeature`.
- [x] `src/docs/spec/memory/04_arenas.md`: a short note that `--debug` counts these events.
      New § Measuring an Arena, and a See Also entry for the debug-report page.
- [x] `planning/todo.md` § Memory § 1: tick the harness items this delivers.
      None of § 1's three items (per-thread RSS over time, entropy-fill cost, soak test) is
      delivered by this plan; it delivers § 2 item 2 (alloc vs free counts for one browser
      load), so the measurement is recorded there instead.
- [x] Full suite + artifact gate + test-accept as in plan-130-A Phase 4.
      Per the user's instruction (scope each letter's tests; one full suite at the end), the
      full `cargo test` and `test-accept.sh` run once in plan-130-E Phase 3. Scoped here:
      `cargo test --bin mfb docs::spec` -> 8 passed (incl. `spec_citations_resolve`);
      `mfb spec tooling debug-report` and `mfb spec memory arenas` render the new text.

Acceptance: the three suites green; `citations_resolve` green.
Commit: abb7246a3

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

- **Phase 3 — the strace equality excludes the registry map.** In a `--debug` build the
  executable makes one more anonymous `mmap` than `arena.*.maps`: the registry itself
  (163,856 bytes, its own call site, mapped once at startup). The 2026-09-12 classification (12)
  was taken on a normal build; removing that one map gives 12 = `arena.0.maps`, and the
  executable's 12 `munmap`s equal `arena.0.unmaps`.
- **Phase 3 — the browser's Wikipedia load crashes, in normal builds too.** Found while
  recording the data point: the app exits 1 with `List or string index/range is outside valid
  bounds.` after the worker parses the page. A normal build fails identically, so `--debug`
  did not cause it; `https://example.com` works. Localizing and fixing it is tracked as its own
  task; the report above was taken up to the crash.
- **Phase 2 — counting sites are the attribution sites, not the returns.** Each helper looks up
  its slot once (after size normalization) and every successful allocation is counted exactly
  once where its path is decided: quick-bin pop, designated-victim carve, carve renewal (before
  the shared `arena_alloc_dv_serve` label, which the walk's whole-chunk take also enters), large
  bin, walk fit (at the walk, not `arena_alloc_found`, which the grow path also enters), and grow
  (`arena_alloc_mapped`, plus `maps`/`mapped_bytes`). That makes
  `alloc_calls == quick + carve + large + walk + grow` an invariant, checked in the tests and in a
  `--debug` run of the whole `benchmark/mfb` suite (`/tmp/p130-c2-invariants.py`): every one of 5
  arenas held it, the main arena with 22,014,381 allocations (quick 21,772,047, carve 120,398,
  large 111,234, walk 3,564, grow 7,138). Counter code is emitted only under `debug_arena`, with
  its vregs allocated inside that branch, so a normal build's instruction stream and vreg
  numbering are unchanged (artifact gate `0 diff(s)`).
- **Phase 2 — double-free skips need a stub; `live_bytes` clamps at zero.** The two early exits
  branch to `arena_free_done` in a normal build and to an `arena_free_double` stub (emitted after
  the return: bump `double_free_skips`, branch to done) in a `--debug` build. A chunk freed that
  was allocated before its arena registered was never added to `live_bytes`, so the subtraction
  saturates at zero instead of wrapping.
- **Phase 2 — test (a) could not assert `alloc_calls == free_calls` for a whole program.** A
  program's startup, printing and end-of-run values allocate outside the loop; the exact claim is
  about the loop, so the test compares N=1000 with N=2000 and asserts the deltas, and uses a short
  string per iteration (a record may be held inline and allocate nothing).
- **Phase 2 — `three_workers_register_four_arenas`'s worker allocated nothing.** The added check
  that each worker counts an allocation failed: arenas 1–3 reported `alloc_calls 0`, `maps 0`,
  because `RETURN len(seed)` allocates nothing in the worker's arena (the benchmark suite's workers
  counted 212 each, so worker counting is live). The worker now builds `seed & "!"`, so the check
  proves counts land in the worker's own slot.
- **Phase 1 — lookup-cost measurement (recorded as the task requires).** Method: the whole
  `benchmark/mfb` suite (it has no row filter) built four ways and run `--run 3`, back to back per
  pair: `--debug` and `--debug` + a THROWAWAY patch inserting a no-op registry slot scan (load
  base, walk `count` slots comparing `state_ptr` with the arena register) at the entry of
  `_mfb_arena_alloc` and `_mfb_arena_free` (`/tmp/p130-c1-lookup-patch2.pl`, never committed;
  `arena.rs` restored from HEAD and the compiler rebuilt afterwards), plus a normal build.
  Tables compared per `section.row` median by `/tmp/p130-bench-compare2.py` (485 rows, no
  duplicate keys). Every `--debug` run reported `arena.count 13` (main + the suite's 12
  workers), so the scan walked a live registry.
  - linux-aarch64 on 2223 (no perf section), normal -> `--debug` + lookup: geomean **x1.007**;
    arena-heavy rows bignum.modmul x0.988, bignum.modexp x0.996, crypto.churn x0.999,
    arena.transient x0.976, arena.mixed x1.035, arena.growshrink x1.075,
    scalarbench.listchurn x1.024, mapchurn.churn x1.028, datetime.civil x0.961,
    datetime.iso x1.022; worst above the 0.05 ms floor: set (Dynamic).union x1.335,
    list (Record-Dynamic).distinct x1.319, list (Dynamic).distinct x1.269,
    list (State-Dynamic).distinct x1.243, map (Dynamic).get x1.188. map.intchurn x1.771 is
    below the floor (0.035 ms).
  - macOS, `--debug` -> `--debug` + lookup (perf in both, so the ratio isolates the scan):
    geomean **x0.997**; arena-heavy rows x0.946–x1.029; worst above the floor
    list (Fixed).insert x1.230, string.unibig x1.136.
  - Verdict: nothing approaches 2x; the linear scan stays. Caveats: `--run 3` medians carry
    run-to-run noise, and this times the lookup alone — Phase 2's counters add a load/add/store
    per event, so the `distinct` rows are the ones to re-check after Phase 2.
  - First attempt broke the compile: the free-path patch anchored inside `lower_arena_free`'s
    initial instruction literal and closed it early (`mismatched closing delimiter` at
    `arena.rs:1128`); the chain restored `arena.rs` and the second patch anchors after that
    literal. The first comparison script also merged every section whose name contains spaces
    (`list (Fixed):`) into the previous one (179 of 479 keys); the recorded numbers are from the
    corrected parser.
- **Phase 1 — `rt_debug_report`'s block check assumed one section.** Its `assert_block` (plan-130-B) required every macOS body line to be `perf.`; with the arena section registered, every `--debug` report also carries `arena.*` lines and six cases failed with `` `arena.count 1` is not a `perf.<key> <integer>` line``. The report format is sections in registry order (`09_debug-report.md`), so the check now accepts `perf.`/`arena.` `<key> <value>` lines and keeps the perf-specific assertions.
- **Phase 1 — the registry mutex is a static data object, not a region field.** §3 put `{mutex (64 B), count, overflow}` in the mapped region, but the lock must exist BEFORE the first registration maps the region (it guards the map). It is `_mfb_rt_debug_arena_lock`, statically initialized exactly like the `os::` env lock (`os_env_lock_init_hex`: macOS `_PTHREAD_MUTEX_SIG_init`, Linux/Windows all-zero), so the region header is `{count, overflow}` (16 B) and slots start at +16.
- **Phase 1 — `emit_debug_arena_slot` moves to Phase 2 with its first consumer (the counters).** Committing an emitter nothing calls fails the warning-free `cargo check --all-targets` (`.ai/build-tooling.md`); the Phase 1 lookup-cost measurement uses a throwaway, uncommitted patch instead.
- **Phase 1 — lock imports need a registry hook, attributed to the program entry.** `import_calls()` resolves a whole runtime call's import set against its own `required_by`, so the register helper's `pthread_mutex_lock`/`unlock` (SRW on Windows) come from a new `DebugFeature::lock_helpers()`. First attributed to `_mfb_debug_arena_register`, every `--debug` build failed: `native object plan relocation source '_mfb_debug_arena_register' is not defined`. Every object plan (`src/os/{macos,linux,windows}/object.rs` `validate_relocations`) turns an import's `required_by` into a relocation source and accepts only `defined_symbols` = entry + functions + runtime + link symbols + data units; a code-layer helper is none of those. The imports are attributed to the entry (`_main` / Windows `_start`, taken from `entry_imports`), exactly how the report's own `_write` links.
- **Phase 1 — registration needed plumbing the plan did not list.** `lower_thread_start_helper` and `emit_start_graphics` are reached through `AbiCtx`, which carried no debug information: `ArenaLayout.debug_arena_registry` -> `lower_abi_function_helper` -> `AbiCtx.debug_arena_registry` (false on the inline path), set from `feature_active(module, ARENA_SECTION)`.
- **Phase 1 — `codegen::debug` tests needed two fixture fixes.** Their import map lacked the mutex pair a real plan attributes (`lower` errored `thread runtime helper requires _pthread_mutex_lock import`), and `debug_helpers_reference_no_arena_symbol` matched the word "arena", which the registry's own debug-owned globals contain. The check now forbids the allocator symbols (every `ARENA_*_SYMBOL` is `_mfb_arena_*`: alloc, destroy, free, insert_free, flush_coalesce, fill_random/seed/next) and `MAIN_ARENA_GLOBAL_SYMBOL` (`_mfb_rt_main_arena`, the one arena pointer the prefix misses), which is the behavior it protects.
## Summary

The allocator hot path is the risk; the design keeps the arena layout untouched by
putting all state in a debug-only registry, and measures the one uncertain cost (slot
lookup) before any counter is placed.
