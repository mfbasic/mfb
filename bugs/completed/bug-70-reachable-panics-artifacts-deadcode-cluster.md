# bug-70: reachable panics on non-default paths, descriptive-artifact skew, and misleading dead code — a batched LOW cluster across the codegen/inspection surface

Last updated: 2026-07-09
Effort: medium (1h–2h)

A cluster of LOW-severity defects that share no single root cause but are each small,
low-risk, and not worth a standalone document: two reachable panics on non-default code
paths, two inspection/plan artifacts that disagree with the emitted binary, and several
misleading-dead-code / wrong-output nits. Grouped so a maintainer can clear them in one pass;
each item is independently landable.

The single correct behavior a fix produces: no reachable `unreachable!`/`.expect` on
user-reachable input (even via non-default flags/crafted packages); inspection artifacts
match the emitted code; and dead/misleading code is removed.

References (paths under `src/target/shared/code/` unless noted):

**Reachable panics (footgun):**

- **`NirType::to_json` panics on a crafted `record`/`resource` NIR kind.**
  `nir/json.rs:170` (`_ => unreachable!("known NIR type kind")`). A corrupt `.mfp` can encode
  a type whose `kind` string is `"record"`/`"resource"` (`ir/binary.rs:548` reads `kind`
  unvalidated; `ir/verify/mod.rs:502-568` has a `_ => {}` catch-all; `validate.rs:626`
  whitelists those two kinds). Running the `-nir` dump on such a package panics the process.
  Fix: reject `record`/`resource` NIR kinds in `type_value_names`, or handle them in
  `NirType::to_json` (return `Err` instead of `unreachable!`).
- **`temporary_vreg`/`temporary_fp_vreg` and the closure-branch scratch `.expect` panic under
  `-regalloc bump`.** `builder_codegen_primitives.rs:132-135`/`:141-144` and
  `builder_collection_queries.rs:emit_direct_callable_branch:1259-1281` force-unwrap the
  fallible `allocate_register()`. Under the user-selectable `BumpAndReset` allocator
  (`-regalloc bump`, wired at `cli/build.rs:117-123`), a deep-enough expression exhausts the
  per-statement bump pool and the `.expect` panics (ICE) instead of surfacing the graceful
  "out of scratch registers" error that `allocate_register()?` bubbles. The default linear-scan
  never Errs. Fix: gate `-regalloc bump` for real builds (restrict to the self-diff harness),
  or route the exhaustion Err through the normal `Result` channel (the gate must be upstream
  since the wrappers cannot return `Result`).

**Inspection/plan artifact skew (correctness of the artifact, not the binary):**

- **Plan builder clears constant-fold state after control flow while the code backend restores
  it**, so the `.native-plan.json`'s `calls`/`operations` over-report runtime calls the real
  binary folds away. `plan/function_builder.rs:135,156,172,199,212` (`self.constants.clear()`
  after If/Match/While/For/DoUntil) vs the code backend's `restore_local_constants`
  (`builder_control.rs:519-527`). `ForEach` in function_builder (`:228`) already restores
  correctly. No runtime impact (the backend works off the NirModule, not `PlannedFunction`).
  Fix: save/restore the pre-construct constants instead of `clear()`.
- **`IO_FLUSH_SPEC` declares `clobbers: &[]`** (`runtime/io_specs.rs:67-76`) though
  `bl _mfb_rt_io_io_flush` clobbers caller-saved registers like every call; every sibling spec
  uses `abi::IO_PRINT_CLOBBERS`. Harmless today (the field is only read as an
  is-this-helper-implemented gate) but a false ABI declaration if a future pass reads per-call
  clobbers. Fix: set `clobbers: abi::IO_PRINT_CLOBBERS` (or delete the vestigial field).

**Misleading dead code / wrong output (dead-code / footgun):**

- **Dead Unicode `seqindex`/`utf8proc_sequence_*` helpers + a spec that still documents them as
  the live mechanism.** `private/unicode.rs:280,288,300,463,485` have no callers (case mapping
  flows through `emit_case_map_lookup` → `emit_unicode_u32_mapping_lookup`); the file-level
  `#![allow(dead_code)]` hides the warning, and
  `src/docs/spec/unicode/01_tables-and-algorithms.md:131-141` still links them as the active
  path. Fix: delete the five methods (and the `UNICODE_*_SEQINDEX`/`UNICODE_SEQUENCES_SYMBOL`
  references they alone consume), or update the spec.
- **stderr integer formatter takes the minus sign from `x19` (arena_base), not the value.**
  `entry_and_arena.rs:2112` tests `compare_immediate("x19", "0")` where it means the
  pre-negation value; the minus branch is dead (arena_base is always non-negative). Latent —
  both callers only pass non-negative error/cleanup codes — but it would misprint a signed
  value. Fix: record the original sign before negating and test that.
- **Dead identical if/else in `scan_loop_locals`.** `function_lowering.rs:522-527` — both the
  `if depth >= 1` and the `else` branch run the same `collect_value_local_reads(condition,
  excluded)`. Fix: collapse to the single unconditional call.
- **macOS app exit-code formatter mis-renders codes > 255.** `macos_aarch64/app/bootstrap.rs:761-794`
  (`emit_format_exit_code`, used by `emit_finish_helper`) only computes hundreds/tens/ones, so
  e.g. exit 1000 prints a garbage digit (`'0'+10 = ':'`) in the GUI transcript's "Program
  exited with code N" line. No overrun (≤ 3 digits). Fix: clamp/mask to 0..255 or use a general
  itoa loop.

- Found during the goal-01 compiler source review of `src/target/`, `src/docs/spec/`.

## Failing Reproduction

Representative triggers (the rest are inspection/latent):

- `NirType::to_json` panic: craft a `.mfp` with a `kind: "record"` type, run the `-nir` dump →
  process panics.
- `-regalloc bump` ICE: `mfb build -regalloc bump prog.mfb` on a function with a deep enough
  single-statement expression → panic instead of a clean diagnostic.
- macOS exit-code: a GUI app that exits with code 300 → transcript shows wrong digits.

- Observed: panic / wrong transcript text / over-reported plan calls / misleading dead code.
- Expected: graceful error, correct text, artifact matching the binary, no dead code.

Contrast: the default linear-scan backend never hits the `.expect`; the code backend restores
constants correctly; sibling specs declare real clobbers; genuine 0..255 exit codes format
correctly.

## Root Cause

Independent per item — see each bullet in References. Common theme: assumptions that hold on
the default/valid path but not on a non-default flag, a crafted package, or an out-of-range
value.

## Goal

- No `unreachable!`/`.expect` reachable via `-regalloc bump` or a crafted `.mfp`.
- `.native-plan.json` `calls`/`operations` match the emitted binary.
- Dead Unicode helpers removed (or the spec corrected); stderr sign formatter reads the value's
  sign; the dead if/else collapsed; exit codes > 255 handled.

### Non-goals (must NOT change)

- Default linear-scan codegen output.
- The valid-package `-nir` dump.
- Genuine 0..255 exit-code formatting.

## Blast Radius

Each `file:symbol` above is an independent site. No shared code; land per item or per theme.

## Fix Design

Per the fix note on each bullet. The two panics and the two artifacts are the highest-value;
the dead-code/formatter nits are cleanups.

## Phases

### Phase 1 — tests

- [x] `nir::json` unit tests render `record`/`resource` kinds without panicking (the crafted
      kind path); `-regalloc bump` deep-expression proven via a pre-fix worktree (ICE) vs
      post-fix (clean error); `emit_format_exit_code` masks to 0..255; `io.flush` clobbers test.
      (The plan-artifact "folded calls absent" test is not writable — see Resolution: the fold is
      unobservable in the artifact, so the item is not a current bug.)

### Phase 2 — the fixes

- [x] Landed each item independently. Deleted the five dead Unicode helpers + three offset
      constants and corrected the spec.

### Phase 3 — validation

- [x] `cargo test --bin mfb` green (2478 passed, +4 new). Default linear-scan codegen is
      byte-identical except the intended entry stderr-sign fix (worktree diff = only the x28 sign
      flag + `b.eq` test). Both panics now surface graceful errors; runtime-verified end to end.

## Validation Plan

- Regression test(s): the crafted-package, `-regalloc bump`, plan-artifact, and exit-code tests.
- Runtime proof: `-nir` on a crafted package errors instead of panicking; GUI exit code 300
  renders correctly.
- Doc sync: correct or delete the Unicode `seqindex` spec section.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

A grab-bag of LOW defects: two panics reachable only via `-regalloc bump` or a crafted
`.mfp`, two inspection artifacts that over-report versus the real binary, and several
dead-code / wrong-output nits (Unicode `seqindex` helpers + stale spec, an arena-base sign
test, a dead if/else, a >255 exit-code formatter). Each is small and independently landable;
none affects default-backend codegen.

## Resolution

Landed 2026-07-10 (all on the current branch). Seven items; six were real and fixed, one
(the plan-artifact over-report) was found to be non-reproducible in the current codebase.

**Reachable panics**

- **`NirType::to_json` (`nir/json.rs`).** `record`/`resource` now share the field-carrying
  `type` arm (matching `validate_nir`'s `type_value_names`, which accepts all three
  interchangeably), so the `-nir` dump renders a crafted `.mfp` instead of hitting
  `unreachable!`. The `_` arm is now a proven invariant: `validate::validate_nir` runs before
  every `NirModule::to_json` caller (`write_nir`) and rejects any other kind. Unit tests:
  `renders_record_and_resource_kinds_without_panicking`, `record_kind_matches_type_shape`.
- **`-regalloc bump` `.expect` ICE.** Root cause: the fixed bump pool has no spilling, so a
  deep single-statement expression exhausts it, and the infallible vreg minters
  (`temporary_vreg`/`temporary_fp_vreg`) `.expect`-panicked on the `Err`. Fix: the minters now
  record the first exhaustion in a new `CodeBuilder.regalloc_error` field and return a
  placeholder vreg (the `allocate_register` error path keeps `vreg_eager`/`fp_vreg_eager`
  aligned with `next_vreg` so the invariant holds); `run_register_allocation` returns
  `Result<(), String>` and surfaces the recorded error at the top, before coloring, aborting the
  build with a clean diagnostic. All three former `.expect` sites
  (`builder_codegen_primitives` wrappers, `builder_collection_queries::emit_direct_callable_branch`,
  `builder_simd_float_math::math_pool_base_reg`) now route through `temporary_vreg`. Proven with
  a pre-change worktree: pre-fix a nested-`strings::` expression panics at
  `builder_codegen_primitives.rs:134`; post-fix it errors gracefully
  (`aarch64 code plan exhausted physical registers …`). Default linear-scan never sets
  `regalloc_error` (it spills), so its output is byte-identical.

**Inspection/plan artifact skew**

- **`IO_FLUSH_SPEC` clobbers** set to `abi::IO_PRINT_CLOBBERS` (matching every sibling io spec),
  a truthful declaration. Only read as an is-implemented gate today (a sibling Io spec already
  satisfies it), so no golden/behaviour change. Test: `io_flush_declares_call_clobbers`.
- **Plan builder constant-fold "over-report" — NOT a current bug.** Two independent findings:
  (1) The premise is stale. The doc says the code backend *restores* constants after control
  flow; bug-57 (commit `84caa8c8`, filed the same day) changed the code backend to
  `clear_local_constants()` after every construct and before every loop body, so the plan
  builder's `clear()` after If/Match already *matches* the binary. (2) More fundamentally, the
  plan builder's `constants` map has **no observable effect on the emitted `.nplan`**: the only
  consumer is the fold early-return in `lower_value`, which suppresses recording a value's nested
  calls — but every foldable target (`toString`, `strings.upper/lower/caseFold/normalizeNfc`,
  `strings.graphemes`, `&`-concat) is a *native direct/inline* call that is never entered into
  `calls` regardless, and their args are all static (no non-direct call can hide inside a static
  string). Verified empirically: `strings::upper(base)` never appears in the `.nplan` `calls`
  whether `base` folds or not. `operations` are structural (`describe_value`) and unaffected.
  No fix applied (nothing to fix, and no failing test is constructible); the plan builder is
  left byte-identical.

**Dead code / wrong output**

- **Dead Unicode helpers.** Deleted `emit_unicode_property_{casefold,uppercase,lowercase}_seqindex`
  and `emit_utf8proc_sequence_{init,decode_next}` (zero callers) plus the three
  `UNICODE_PROPERTY_OFFSET_*_SEQINDEX` constants they alone consumed. `UNICODE_SEQUENCES_SYMBOL`
  is kept (still emitted by `data_objects.rs`; only the dead *reader* is gone). Spec
  `unicode/01_tables-and-algorithms.md` corrected: the seqindex offset-table rows now read
  "(emitted; not read by runtime helpers)", the prose no longer claims a parallel constant, and
  the sequences-table section no longer documents the removed decoders as live. Runtime-verified
  `strings::upper`/`caseFold` still correct.
- **stderr integer sign (`entry_and_arena.rs`).** The minus test read `x19` (arena_base, always
  non-negative) — the value's pre-negation sign was already lost. Now the original sign is
  recorded in `x28` (0/1) before the value is negated, and the minus branch tests `x28`. Changes
  every non-app binary's entry code by 3 instructions (worktree diff confirms *only* this).
  Runtime-verified: a failing program still prints its multi-digit non-negative error code with
  no spurious minus.
- **Dead if/else (`function_lowering::scan_loop_locals`).** Collapsed to the single
  unconditional `collect_value_local_reads(condition, excluded)` (both branches were identical).
- **macOS app exit code > 255 (`bootstrap.rs`).** `emit_format_exit_code` now masks the code to
  its low 8 bits (`& 0xFF`) before formatting, matching what `_exit(status)` delivers to the
  parent (POSIX truncation), so e.g. `300` renders `44` consistently with the headless path
  instead of a garbage digit. Spec `app/01_macos-runtime.md` updated. Test:
  `exit_code_formatter_masks_to_low_8_bits`.

### Golden impact

- **All non-app `.ncode`/`.nobj` goldens shift** (entry stderr-sign fix — 3 instructions in every
  binary's error/cleanup path).
- **App-mode (`.app.ncode`/`.app.nobj`) goldens shift** (exit-code mask — 2 instructions in the
  finish helper).
- `.nir`/`.nplan` goldens are unchanged (record/resource fix only affects crafted input; the
  plan builder was not modified).
