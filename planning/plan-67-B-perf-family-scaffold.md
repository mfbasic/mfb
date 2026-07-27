# plan-67-B: Perf helper family, debug-gating, region lifecycle, entry/exit injection

Last updated: 2026-07-26
Effort (Human): large
Effort (AI): medium
Depends on: plan-67-A (release harness must drive the golden path before any
debug-gated injection lands, or `cargo test` goes red — see A's Prerequisites)
Produces:
- `RuntimeHelper::Perf` family with calls `perf.init`, `perf.start`, `perf.end`,
  `perf.done` → symbols `_mfb_rt_perf_init/start/end/done`, wired through the spec
  catalog, `helper_for_call`, `required_helpers`, and symbol emission.
- A writable global slot `_mfb_rt_perf_state` (`kind:"raw"`, 8 bytes) holding the
  base pointer of the perf region.
- `perf_init` (mmap a zeroed system region, store base in the global) and
  `perf_done` (print the table header + munmap) helper bodies.
- Debug-gated injection **on the macOS backend only**: `perf_init` at the program
  entry stub, `perf_done` in the exit tail — the reusable gating predicate and
  injection sites that C–F extend.

**Platform scope (applies to all of B–F):** real perf is implemented only for the
**macOS** backend for now. `lower_perf_helper`'s Linux and Windows arms emit
**no-op (return-only) stub bodies**, and injection is emitted only on the macOS
entry/exit path — Linux and Windows debug **and** release builds are byte-identical
to pre-plan-67 HEAD. (The user explicitly asked for no-op stubs on Linux/Windows;
this is the one sanctioned stub in the feature.)

This letter delivers the end-to-end skeleton: a **debug-built macOS** compiler
produces a program that, at exit, prints a perf table header and frees the region;
a **release-built** compiler — and any Linux/Windows build — produces byte-identical
output to today. `perf_start` / `perf_end` and any real rows arrive in C–F.

References:

- `.ai/compiler.md` (runtime completion gate, register lifetimes — a `bl _mfb_*`
  clobbers x0–x17), `.ai/specifications.md` (keep `mfb spec` current).
- Prerequisites: see plan-67-A. Do not restart them here; re-run A's gate.

## 1. Goal

- Building the compiler with `cargo build` (debug) makes every compiled program
  print, as the very last thing before process exit (after error banner, after
  terminal reset / `_mfb_shutdown`), a perf table **header**; `cargo build
  --release` yields output byte-identical to pre-plan-67 HEAD.

### Non-goals

- No table/array logic yet (C–D). `perf_done` prints only the header row.
- No language surface: there is **no** `perf::` builtin package; the helpers are
  invoked only by compiler-injected calls and are invisible to MFB source.
- **macOS only:** Linux and Windows get no-op stub helper bodies and no injection;
  they stay byte-identical in both debug and release.
- The perf region and its helpers must **never** touch the arena or call any
  arena helper (prevents the recursion F would otherwise create when it wraps
  arena code). System memory only.
- Release output unchanged; goldens unchanged.

## 2. Current State

- Runtime helpers are hand-emitted machine code. Registry: `RuntimeHelper` enum
  (`src/target/shared/runtime/mod.rs:3`, `name()` at `:21`, `symbol_for_call` at
  `:40`, `helper_for_call` at `:113`); master spec table `SUPPORTED_HELPER_SPECS`
  (`src/target/shared/runtime/catalog.rs:9`); consistency test at `catalog.rs:203`
  with a `families` list at `catalog.rs:269`. Body-emitter dispatch is the `match
  spec.call` in `src/target/shared/code/mod.rs:1682` onward (datetime arm
  ~`:1700`). **`datetime` is the template** (`src/target/shared/code/datetime.rs`;
  `lower_datetime_helper` at `:74`; frame finalize `finalize_vreg_body_with_locals`
  at `:215`).
- System (non-arena) memory: the `CodegenPlatform` seam `emit_arena_map` /
  `emit_arena_unmap` (declared `src/target/shared/code/types.rs:706`; macOS mmap
  `src/target/macos_aarch64/code.rs:764,793`; Windows VirtualAlloc/Free
  `src/target/win_x86_64/code.rs:620,645`; Linux `src/target/linux_common/code.rs:117`).
  Precedent for a persistent non-arena page: the audio subsystem
  (`src/target/shared/code/audio/mod.rs:43,57`; `audio/macos.rs:202-226,565`).
- Writable global slot with a stable symbol: `CodeDataObject`
  (`src/target/shared/code/types.rs:104-111`); anything with `kind != "constant"`
  lands in the writable region (`types.rs:123-163`). Canonical example
  `MAIN_ARENA_GLOBAL_SYMBOL = "_mfb_rt_main_arena"` (declared
  `src/target/shared/code/error_constants.rs:326`; emitted as an 8-byte zeroed
  `kind:"raw"` object at `src/target/shared/code/mod.rs:727-735`). Store via
  `push_symbol_address` + `abi::store_u64` (`entry.rs:183-190`;
  `push_symbol_address` defined `data_objects.rs:7-32`); load is the symmetric
  `adrp/add` + `abi::load_u64`. Writable-section guarantee holds for modules with
  an entry (`mod.rs:722-726`).
- Program entry/exit: `lower_program_entry` (`entry.rs:4`); entry label `:37`;
  single exit choke point `exit_label = "entry_exit"` (`:179`, emitted `:576`);
  error printing `:504-575`; exit code parked at `[arena+32]` (`:583-587`) and
  reloaded (`:601-605`) around the `bl _mfb_shutdown` (`:599-600`); final
  `platform.emit_program_exit` (`:606`). Callers: `macos_aarch64/code.rs:109`,
  `win_x86_64/code.rs:544`, `linux_common/code.rs:424`.
- Output primitive: `platform.emit_write(fd, buf, len)` (`types.rs:470-476`; fd in
  `RET[0]`, buf in `ARG[1]`, len in `ARG[2]` — `abi.rs:93,424,419`; macOS
  `code.rs:266-286`, Linux `linux_common/code.rs:151-159`, Windows `code.rs:695`).
  Convenience `emit_write_string_object` (`entry.rs:924-956`).
- Which helpers are emitted is decided by `required_helpers` walking the IR
  (`src/target/shared/runtime/usage.rs:109`, parity check `:154`) and
  `plan/symbols.rs:59`. Perf helpers are **not** reachable from the IR (no MFB
  call site), so they must be **force-required when `cfg!(debug_assertions)`**.
- No build-mode-gated codegen exists today (`debug_assertions` uses are internal
  asserts only — `codegen_utils.rs:446,755`, `linear_scan.rs:338`,
  `riscv64/v128.rs:248`, `rules/mod.rs:205`). This letter introduces the first.

### Verified properties

- **`_mfb_rt_main_arena` is a working writable-global precedent** — VERIFIED by
  reading `mod.rs:727-735` (emit) and `entry.rs:183-190` (store). The perf global
  mirrors it exactly.
- **The exit tail already uses a park/reload idiom around a clobbering `bl`** —
  VERIFIED at `entry.rs:583-605`: the exit code survives `_mfb_shutdown`. A
  `perf_done` call inserted here must sit **before** the reload (`:601`) and, like
  `_mfb_shutdown`, must not disturb `[arena+32]`.

## 3. Design Overview

Four independent pieces, layered:

1. **Family wiring** — enum variant + specs + catalog + `helper_for_call` +
   dispatch. Zero-arg specs for `init`/`done`; one string-arg specs for
   `start`/`end` (declared now, bodies filled in C/D). Mirror `datetime_specs.rs`.
2. **Force-require in debug** — `required_helpers` includes the Perf family iff
   `cfg!(debug_assertions)`, and the `_mfb_rt_perf_state` data object is emitted
   under the same condition, so release plans are byte-identical.
3. **Region lifecycle** — `perf_init`: `emit_arena_map` a fixed region (size TBD,
   e.g. 16 MiB; the region layout — two name-keyed tables + a bump area — is
   defined in C/D, so B only reserves and zeroes it and stores the base).
   `perf_done`: load
   base, print header, `emit_arena_unmap`. **No arena calls anywhere in these
   bodies.**
4. **Debug-gated injection (macOS only)** — a single predicate (introduce
   `fn perf_injection_enabled() -> bool { cfg!(debug_assertions) }` in the shared
   code module) gates injection, but the injection is emitted only from the
   **macOS** entry/exit lowering path (`macos_aarch64/code.rs:109` →
   `lower_program_entry`); the Linux (`linux_common/code.rs:424`) and Windows
   (`win_x86_64/code.rs:544`) callers do not inject. It gates: (a) `perf_init`
   emitted just after arena init in `lower_program_entry`; (b) `perf_done` emitted
   in the exit tail between `bl _mfb_shutdown` (`entry.rs:600`) and the exit-code
   reload (`:601`). `lower_perf_helper`'s non-macOS arms emit a return-only body so
   the dispatch is total if a perf symbol is ever referenced off-macOS.

**Design uncertainty (schedule this letter first among B–F):** does a debug build
inject while a release build stays byte-identical, and does the non-arena region +
output path actually work end to end on the host? This letter is that spike.

**Correctness risk:** the injection must not corrupt the parked exit code or the
program-exit ABI. Contained to two fixed points; low relative to F.

Rejected alternative: storing the region base in an arena scratch slot (offset 32
is transient exit-code scratch; the arena state is per-thread and every offset is
already claimed — `error_constants.rs:400-543`). A fixed writable global is the
correct home and is process-wide.

## 4. Detailed Design

- **Region base global:** `PERF_STATE_SYMBOL = "_mfb_rt_perf_state"` in
  `error_constants.rs`; emit an 8-byte, 8-aligned, zeroed `kind:"raw"` object next
  to the arena global at `mod.rs:727-735`, guarded by `perf_injection_enabled()`.
- **`perf_init`:** compute region size into the size reg, `emit_arena_map`, on
  success zero the header words and `store_u64` the base into `_mfb_rt_perf_state`;
  on failure store 0 (perf silently inert — a failed profiler must not crash the
  program). Reuse the `datetime` frame scaffolding (`finalize_vreg_body_with_locals`).
- **`perf_done`:** `load_u64` the base; if 0, return. Else write the header
  (a static string data object, e.g. `"name  count  avg  median  min  max
  sum\n"`) via `emit_write` to the chosen fd (see Open Decisions), then
  `emit_arena_unmap`. C–F extend the middle to iterate rows.
- **Injection sites:** entry — after the arena is live (so a future `perf_start`
  has a valid region) emit `bl _mfb_rt_perf_init`; exit — emit `bl
  _mfb_rt_perf_done` after `:600`, before `:601`, using the same
  `internal_branch` relocation helper the `_mfb_shutdown` call uses
  (`entry.rs:599-600`). Both wrapped in `if perf_injection_enabled()`.

## Compatibility / Format Impact

- **Debug builds on macOS only:** generated programs gain two internal helper
  calls and a perf table at exit. **Release builds, and all Linux/Windows builds:**
  no change — no injection, goldens identical. The `_mfb_rt_perf_state` global and
  `_mfb_rt_perf_*` helpers appear only in macOS debug output (non-macOS arms are
  no-op stubs and are not injected). No public API/ABI/format change.

## Phases

> Keep checkboxes current in the same commit as the work. Unticked = NOT DONE.

### Phase 1 — Family wiring (compiles, release byte-identical)

- [x] Added `RuntimeHelper::Perf` (`runtime/mod.rs`) + `name()`→"perf", and
      `perf_specs.rs` with four specs (`perf.init`/`perf.done`/`perf.start`/
      `perf.end`, all `returns:"Nothing"`). Listed all four in
      `SUPPORTED_HELPER_SPECS` and added `Perf` to the `families` loop + bumped the
      count 11→12 (`catalog.rs`). **No `helper_for_call` `perf::` arm** — see
      Corrections; instead the four calls are in `CODE_LAYER_ONLY_CALLS` (accurate:
      injected, never NIR-level), so `catalog_is_consistent` expects `None`.
- [x] Added `perf_injection_enabled() -> bool { cfg!(debug_assertions) }`
      (`code/mod.rs`). **Did NOT touch `required_helpers`/`validate`** — see
      Corrections: emission is driven by `plan::symbols::runtime_symbols` (the
      single source the code layer clones), not the declared-helper set, so the
      declared-vs-used parity never involves Perf and stays balanced untouched.
- [x] `catalog_is_consistent` passes (`cargo test -p mfb --bins
      catalog_is_consistent` → `1 passed`).

Acceptance: `cargo test` green; `cargo build --release && scripts/artifact-gate.sh
target/release/mfb` → `diffs=0` (release unaffected).
Commit: —

### Phase 2 — Region global + `perf_init`/`perf_done` bodies

- [x] Added `PERF_STATE_SYMBOL` (`_mfb_rt_perf_state`) + `PERF_HEADER_SYMBOL`
      (`_mfb_rt_perf_header`) to `error_constants.rs`; emit the zeroed 8-byte
      `kind:"raw"` global next to the arena global, gated on
      `perf_injection_enabled() && module.entry.is_some() && module.target ==
      "macos-aarch64"`.
- [x] Added `src/target/shared/code/perf.rs` (`lower_perf_helper`): `perf.init`
      (`emit_arena_map` 16 MiB → store base on success, leave global 0 on failure —
      `MAP_ANON` is pre-zeroed so no explicit clear), `perf.done` (load base; 0 →
      inert; else write the header string object to stderr; region **left mapped**
      per the Open Decision), `perf.start`/`perf.end` return-only (bodies C/D), and
      a return-only tail for the non-macOS arms (dispatch totality). Wired into the
      `match spec.call` dispatch. No arena calls (invariant held by construction —
      region via the `emit_arena_map` seam, not `_mfb_arena_alloc`).
- [x] Header string data object (`"name count avg median min max sum\n"`) emitted
      under the same gate.
- [x] Force the emitted perf symbols into `plan::symbols::runtime_symbols`
      (init/done now; start/end deferred to C/D) under the same gate — the actual
      emission driver (the code layer clones this set), replacing the plan's
      `required_helpers` route.

Acceptance: unit/inspection — the helper bodies assemble and encode on the host
backend (no panic in `artifact-gate.sh` for a fixture that would call them once
injection lands). Release still `diffs=0`.
Commit: —

### Phase 3 — Debug-gated entry/exit injection (end-to-end skeleton)

- [x] Inject `bl _mfb_rt_perf_perf_init` after the arena-global publish in
      `lower_program_entry`, gated on `perf_injection_enabled() &&
      platform.family() == MacOS`. Symbol **derived** via `symbol_for_call`, never
      hard-coded (the family doubles into the name — see Corrections).
- [x] Inject `bl _mfb_rt_perf_perf_done` after the `bl _mfb_shutdown` and before
      the exit-code reload, same gate; `perf_done` preserves the callee-saved arena
      register and never touches `[arena+32]`.
- [x] Runtime proof on the macOS host (`/tmp/p67proof`, `RETURN 7`): a **debug**
      build prints `hello from p67` on stdout (exit 7) and `name count avg median
      min max sum` on **stderr** as the last output; a **release** build of the same
      program prints nothing on stderr (stdout + exit 7 unchanged). Header is last,
      exit code intact. ✓

Acceptance (the falsifying spike): a **debug**-built macOS compiler compiles+runs a
trivial program and the perf **header** is the last output before exit, with the
program's exit code intact; a **release**-built compiler — and a Linux/Windows
build — produces byte-identical output to pre-plan-67 HEAD
(`scripts/artifact-gate.sh target/release/mfb` → `diffs=0`, acceptance suite green
under release per plan-67-A).
Commit: —

## Validation Plan

- Tests: `catalog_is_consistent`; a runtime-proof program run under a debug build
  (header present) and under release (absent).
- Coverage check: confirm the injected calls appear in a debug `.ncode` dump and
  are absent in a release dump for the same fixture.
- Runtime proof: debug `mfb build` + run of a hello-world prints the header last;
  exit code preserved.
- Doc sync: add the Perf helper family + the debug-gating to `mfb spec`
  (runtime/startup memory docs) and `.ai/specifications.md`.
- Acceptance: `cargo test`; `scripts/artifact-gate.sh target/release/mfb`
  (`diffs=0`); full acceptance under release (plan-67-A) green.

## Open Decisions

- **Output stream** — *(recommended)* stderr (fd 2), so the table is diagnostic
  and never mixes into program stdout, vs. stdout (fd 1). Note: under the debug
  test harness this is moot for goldens because plan-67-A moved golden generation
  to release. (§4)
  Decision: stderr
- **Region size** — *(recommended)* a fixed 16 MiB reservation with growth handled
  in D (mmap another region on exhaustion, never a silent cap) vs. a smaller fixed
  cap. Decide the header layout in C/D. (§4)
  Decision: a fixed 16 MiB reservation with growth handled in D
- **Region freeing at exit** — *(recommended)* `emit_arena_unmap` in `perf_done`
  for cleanliness vs. leave it (process is exiting anyway). Recommend unmap. (§4)
  Decision: Leave it

## Corrections

- **Emission path (design §3.2 was imprecise).** The plan routed perf emission
  through `required_helpers` + the declared-vs-used `validate` parity. But body
  emission is actually driven by `plan::symbols::runtime_symbols` (a call-site scan
  that the code layer clones at `code/mod.rs` `let mut runtime_symbols =
  native_plan.runtime_symbols.clone()`), **not** by `module.runtime_helpers`
  (which only feeds validation). Perf calls never appear in any NIR function, so
  the correct hook is to force the perf symbols into `runtime_symbols` (like the
  existing `term.off`/`thread.drop` special-cases), leaving `required_helpers` and
  `validate` untouched. The declared-vs-used parity therefore never involves Perf
  and cannot be tripped — the very complication §3.2 anticipated is avoided rather
  than managed.
- **Gate is debug + macOS + entry, not "iff `cfg!(debug_assertions)`".** §3.2 said
  `required_helpers` includes Perf "iff `cfg!(debug_assertions)`". Declaring/emitting
  Perf on a debug *Linux/Windows* build would add dead stub symbols and break the
  Produces-clause "Linux/Windows debug builds byte-identical to HEAD". The force,
  the data-object emission, and both injection sites all gate on
  `perf_injection_enabled() && entry && target=="macos-aarch64"` (injection uses
  `platform.family()==MacOS`). The non-macOS `lower_perf_helper` arms remain (a
  return-only body) purely for dispatch totality; they are never emitted.
- **No `helper_for_call` `perf::` arm.** Phase 1 called for one, but nothing routes
  `perf.*` at the NIR level (`lower_runtime_helper` resolves via `spec_for_symbol`,
  not `helper_for_call`), so an arm would be dead code (AGENTS.md forbids it). The
  four calls are in `CODE_LAYER_ONLY_CALLS` instead — the accurate model (they are
  "synthesized inside the code layer … never exist at the NIR level", exactly like
  `thread.drop`).
- **Symbol names double the family.** `symbol_for_call` emits
  `_mfb_rt_{family}_{sanitized_call}`, so the helper symbols are
  `_mfb_rt_perf_perf_init` / `…_perf_done` / `…_perf_start` / `…_perf_end` (cf. the
  real `_mfb_rt_io_io_print`), **not** the `_mfb_rt_perf_init` the plan text used in
  a few places. The injection derives the symbol via `symbol_for_call` rather than
  hard-coding it, so it cannot drift from the emitted body. (The two writable data
  globals — `_mfb_rt_perf_state`, `_mfb_rt_perf_header` — are hand-named and not
  subject to this.)
- **`perf_done` region freeing.** Open Decision chose "leave mapped"; `perf_done`
  does no `emit_arena_unmap` (the process is exiting).

## Summary

The skeleton that proves the two genuinely novel things: first build-mode-gated
codegen, and a persistent non-arena region printed at process exit. Real timing
data is C–F. The one invariant to hold from here on: perf code never calls the
arena.
