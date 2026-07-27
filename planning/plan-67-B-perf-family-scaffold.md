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

- [ ] Add `RuntimeHelper::Perf` (`runtime/mod.rs:3`, `name()` `:21`), the
      `helper_for_call` `perf::` arm (`:113`), and `perf_specs.rs` with four specs
      (mirror `datetime_specs.rs:9-25`): `init`/`done` zero-arg, `start`/`end` one
      String arg. Register the module and list all four in `SUPPORTED_HELPER_SPECS`
      (`catalog.rs:9`); add `perf` to the `families` list (`catalog.rs:269`).
- [ ] Add `perf_injection_enabled()` and force-require the Perf family in
      `required_helpers` (`usage.rs:109`) under it; keep the declared==used parity
      (`usage.rs:154`) balanced.
- [ ] Tests: `catalog_is_consistent` (`catalog.rs:203`) passes with the new
      family.

Acceptance: `cargo test` green; `cargo build --release && scripts/artifact-gate.sh
target/release/mfb` → `diffs=0` (release unaffected).
Commit: —

### Phase 2 — Region global + `perf_init`/`perf_done` bodies

- [ ] Add `PERF_STATE_SYMBOL` (`error_constants.rs`) and emit the zeroed `kind:"raw"`
      global under `perf_injection_enabled()` (`mod.rs:727-735` neighborhood).
- [ ] Add `src/target/shared/code/perf.rs` with `lower_perf_helper`; implement
      `perf_init` (region mmap + zero + store base; failure → base 0) and
      `perf_done` (load base; if 0 return; else write header + munmap) **for the
      macOS backend**, and return-only no-op bodies for the Linux/Windows arms. Wire
      into the dispatch `match spec.call` (`mod.rs:1700` neighborhood). No arena
      calls.
- [ ] Add the header string data object.

Acceptance: unit/inspection — the helper bodies assemble and encode on the host
backend (no panic in `artifact-gate.sh` for a fixture that would call them once
injection lands). Release still `diffs=0`.
Commit: —

### Phase 3 — Debug-gated entry/exit injection (end-to-end skeleton)

- [ ] Inject `bl _mfb_rt_perf_init` after arena init in `lower_program_entry`
      (`entry.rs`), gated, **only from the macOS entry path**.
- [ ] Inject `bl _mfb_rt_perf_done` between `entry.rs:600` and `:601`, gated,
      preserving the parked exit code at `[arena+32]`, **macOS path only**.
- [ ] Add a debug-only fixture (or a manual runtime-proof program) that compiles
      and runs; confirm the header prints at exit and the exit code is unchanged.

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

<Filled in during execution.>

## Summary

The skeleton that proves the two genuinely novel things: first build-mode-gated
codegen, and a persistent non-arena region printed at process exit. Real timing
data is C–F. The one invariant to hold from here on: perf code never calls the
arena.
