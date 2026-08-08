# plan-88-C: migrate the symbol-path emitters + declare helper errors

Last updated: 2026-08-03
Effort: large (3h–1d)
Depends on: plan-88-B (all per-call-site emitters must already be on
`raise_error`/`raise_error_bare` — if B is not complete, C cannot start, full
stop.)

Sub-plan **C** of plan-88. See plan-88-A §3 for the overall design. C migrates
the **symbol path** — the 49 `push_error_message_address(.., ERR_*_SYMBOL, ..)`
sites across 11 fixed-native-helper files — onto the same `raise_error` /
`raise_error_bare` primitive, and declares the `errors` contract for the builtins
those helpers implement. After C, the used-set is populated by *every* error
emission in the codebase (per-call-site from B, symbol-path from C), which is the
precondition D needs to switch emission over and delete the manual gating.

Behavioral outcome for C: **every symbol-path error emission goes through
`raise_error`/`raise_error_bare` and records into the used-set; the compiled
program raises the same error code+message as before (byte-identity is expected
to break here — the emission unifies — and goldens re-baseline).**

References: plan-88-A; `src/target/shared/code/data_objects.rs`
(`push_error_message_address`); the 11 symbol-path files (below);
`src/target/shared/code/app.rs` (`prepend_wrong_mode_gate`, already takes a
`function_id`); `.ai/compiler.md`.

## Prerequisites

See plan-88-A §Prerequisites (shared). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-88-B complete | `ls planning/completed/plan-88-B-*` (exists) | NOT MET (B pending) |
| No `emit_*_return` **call sites** remain | `grep -rc 'self\.emit_[a-z_]+_return()' src/target/shared/code/*.rs \| awk -F: '{s+=$2} END{print s}'` → 0 | NOT MET until B |

## 1. Goal

- Zero remaining `push_error_message_address(.., ERR_*_SYMBOL, ..)` calls: every
  symbol-path error emission replaced by `raise_error(func_id, name)` (helper
  implements a named builtin) or `raise_error_bare(name)` (shared runtime helper
  with no owner), each recording into the used-set. Every builtin whose helper
  raises an error declares that error in its `BuiltinFunction.errors`.

### Non-goals

- Do **not** delete `ERR_*` constants, wrappers, or the manual gating — that is D.
- Do **not** activate used-set-driven emission — that is D. In C the manual
  `push_string_value` gating in `data_objects.rs` still emits the strings; C
  changes how the error is *raised* (through the two methods).
- **Runtime behavior is preserved** (same code+message). Codegen output **does**
  change (the helper shape unifies) — that is intended, and goldens re-baseline.
  What must not change is the error a program observes at runtime.

## 2. Current State (delta from B)

After B, all per-call-site emitters use `raise_error`. The remaining error
emitters are the symbol path:

| What | Count | Command |
|---|---|---|
| `push_error_message_address` calls | 49 | `grep -rcE 'push_error_message_address' src/target/shared/code/*.rs \| awk -F: '$2>0{s+=$2} END{print s}'` → 49 |
| files (excl. `data_objects.rs`) | 11 | `app.rs, datetime.rs, io_stdin.rs, float_format.rs, entry.rs, io_stdout.rs, native_helpers.rs, io_terminal.rs, runtime_helpers_thread.rs, runtime_helpers.rs, term.rs` |
| distinct `ERR_*_SYMBOL` referenced | 41 | `grep -rhoE 'ERR_[A-Z_]+_SYMBOL' src/target/shared/code/*.rs \| grep -v error_constants \| sort -u \| wc -l` → 41 |

### Verified / to-verify properties

- **Some symbol-path files implement named builtins** (have a
  `BuiltinFunction`): `datetime.rs` (`datetime.*`), `io_stdin.rs` (`io.input`/
  `pollInput`/…), `io_stdout.rs`/`io_terminal.rs` (`io.*`), `term.rs` (`term.*`),
  `app.rs` (`app.*` + the shared `prepend_wrong_mode_gate`). These → `raise_error(
  func_id, name)`.
- **Some are shared runtime helpers with no single owner**: `native_helpers.rs`,
  `runtime_helpers.rs`, `runtime_helpers_thread.rs`, `entry.rs`, `float_format.rs`.
  These → `raise_error_bare(name)`.
- **The per-site owner classification is UNVERIFIED per site** and is the first
  task of C: for each of the 49 sites, `grep` its enclosing `pub(super) fn` and
  decide func-vs-bare (as in B). `prepend_wrong_mode_gate` already carries a
  `function_id` (worktree change) → `raise_error(function_id, "ErrWrongMode")`.

### The emission-shape change (central to C) — and why byte-identity is NOT the gate

Today the helper sites emit a **different, lighter shape** than the per-call-site
path: the fixed helper sets the result return registers directly
(`RESULT_VALUE_REGISTER` = code, `RESULT_TAG_REGISTER` = err-tag) and calls
`push_error_message_address` only to load the message address into
`RESULT_ERROR_MESSAGE_REGISTER`, then returns — **no `ErrorLoc`, no
`_mfb_make_error_result` call.** The per-call-site path (and therefore
`raise_error`) builds the full `Error` via `_mfb_make_error_result`, *with* a
source location.

Converting the helper sites to `raise_error`/`raise_error_bare` therefore **will
change the emitted bytes** (and give those errors a source loc they lacked, and
route them through `_mfb_make_error_result`). **That is intended** — it is the
unification that makes invariant #1 (two entry points) true. Preserving the old
fragment shape would require a third emission method, which is exactly what the
feature forbids. So C does **not** chase byte-identity; it re-baselines the
affected goldens and proves **runtime behavior is preserved** — the same error
code + message is raised — plus the intended, documented deltas (source loc now
present; `_mfb_make_error_result` now used). The correctness risk is that a
converted helper raises the *wrong* error or corrupts its result registers, not
that its bytes moved.

## 3. Design

- **No new primitive.** Every helper site is rewritten to call the *same* two
  methods everything else uses — `raise_error(func_id, name)` or
  `raise_error_bare(name)`. There is deliberately no fragment primitive; adding
  one would be a third emission path and break invariant #1. The helper's own
  inline register-setting + `push_error_message_address` sequence is deleted and
  replaced by the single `raise_error` call, which owns the whole error return.
- Migrate **one file at a time**; re-baseline that file's goldens and verify
  runtime behavior (same code+message; the intended source-loc / make-error
  deltas).
- For each helper implementing a named builtin, add its raised errors to that
  builtin’s `BuiltinFunction.errors` (`src/builtins/*.rs`).
- `prepend_wrong_mode_gate` already has `function_id`: replace its hardcoded
  `ERR_WRONG_MODE_CODE` + `push_error_message_address(ERR_WRONG_MODE_SYMBOL)` with
  `raise_error(function_id, "ErrWrongMode")`, and add `"ErrWrongMode"` to the
  `errors` of the gated builtins (`io.input`/`readLine`/`readChar` and the gated
  `term.*` set — enumerate from the two call sites in `mod.rs`).

## Phases

### Phase 1 — classify + pilot conversion

- [ ] Classify all 49 sites: for each, record enclosing `fn` and func-vs-bare
      (a table in this plan’s Corrections). Command: `grep -nE
      'push_error_message_address' <file>` then read the enclosing `fn`.
- [ ] Convert ONE pilot site fully to `raise_error`/`raise_error_bare` (delete its
      register-setting + `push_error_message_address` sequence), re-baseline its
      golden, and prove via a runtime test that the pilot raises the identical
      error code + message. Confirm the only deltas are the intended ones (source
      loc present; `_mfb_make_error_result` used).

Acceptance: the 49-row classification table is complete AND the pilot site, run
end-to-end, raises the same code+message as before (goldens re-baselined, not
held constant). No fragment primitive is introduced.
Commit: —

### Phase 2 — migrate named-builtin helpers

- [ ] Convert the symbol-path sites in `datetime.rs`, `io_stdin.rs`,
      `io_stdout.rs`, `io_terminal.rs`, `term.rs`, `app.rs` (incl.
      `prepend_wrong_mode_gate`) to `raise_error(func_id, name)`; add each raised
      error to the owning builtin’s `errors` in `src/builtins/*.rs`.
- [ ] Tests (per-site gate): for each converted helper, a `tests/rt-error/**`
      fixture triggers its error and asserts `Error.code`, green **before** the
      conversion (add if missing — incl. an app-mode wrong-mode fixture and a
      stdin-from-unsubscribed-thread `ErrInvalidContext` fixture). Re-run after.

Acceptance (per-site gate): no `push_error_message_address` calls remain in those
6 files (`grep -rc` → 0 each); every converted helper’s rt-error fixture raises the
same `Error.code` after as before (message unchanged, or on the Phase-0
consolidation list); `cargo test --bin mfb` green.
Commit: —

### Phase 3 — migrate shared runtime helpers (bare)

- [ ] Convert the symbol-path sites in `native_helpers.rs`, `runtime_helpers.rs`,
      `runtime_helpers_thread.rs`, `entry.rs`, `float_format.rs` to
      `raise_error_bare(name)`.
- [ ] Tests (per-site gate): a `tests/rt-error/**` fixture per shared-helper error
      path (where one can be triggered from source), asserting `Error.code`, green
      before and after. For helpers not reachable from a program, assert via the
      emit-inspection unit test that the raised code matches.

Acceptance (per-site gate): `grep -rc 'push_error_message_address'
src/target/shared/code/*.rs` = 0 outside `data_objects.rs`’s definition; the
rt-error fixtures raise the same `Error.code` after as before; `cargo test --bin
mfb` green.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` after each phase; the `io`/`term`/`datetime`/
  `thread`/crypto runtime tests exercise the converted helpers.
- Coverage check: confirm those runtime tests are in the bin suite denominator
  (some resource/network tests are pre-existing reds — filter by name-glob and
  compare against the baseline, per the acceptance-preexisting-reds note).
- Runtime proof: an app-mode program that hits the wrong-mode gate, and a stdin
  read from an unsubscribed thread (`ErrInvalidContext`), still raise the same
  codes/messages after C.
- Codegen goldens: `scripts/artifact-gate.sh` after each phase to see the delta,
  then **re-baseline** the affected goldens (the byte change is the intended
  unification, not a regression — AGENTS.md’s “never edit a golden until proven
  wrong” is satisfied here by the runtime proof that the same error is raised).
  Inspect the diff to confirm it is only error-emission sites, nothing unrelated.
- Doc sync: none.
- Acceptance: `cargo test --bin mfb` green + runtime proof that every converted
  helper raises the same code+message. After C, verify `used_errors` is populated
  by BOTH paths (a test that the module’s used-set is non-empty on a program
  exercising a symbol-path helper) — the precondition D relies on.

## Open Decisions

- None. (The former "fragment primitive" decision is resolved: **no** fragment
  primitive — the helper sites converge on the two canonical methods even though
  their bytes change. Preserving the fragment shape is explicitly rejected because
  it would be a third emission path, violating invariant #1.)

- **To confirm in the pilot (not a design fork):** that giving helper errors a
  source location and routing them through `_mfb_make_error_result` is acceptable
  behavior (it is the intended unification). If any helper error must stay
  source-loc-free for a runtime reason, record it in Corrections — but the default
  is: all errors are built one way.

## Corrections

<Filled in during execution — the 49-site classification table, and any site
whose runtime error changed (code or message) rather than only its bytes (that is
a real regression to fix; a byte-only change is the expected unification).>

## Summary

C moves the 49 symbol-path emitters across 11 fixed-helper files onto
`raise_error`/`raise_error_bare` — deleting each helper's bespoke register-set +
`push_error_message_address` fragment rather than preserving it. This deliberately
**changes the bytes** (helper errors gain a source loc and go through
`_mfb_make_error_result`); goldens re-baseline and the gate is a runtime proof
that the same error is raised, not zero delta. That unification is the point: it
collapses the third emission path so only the two methods remain. After C the
used-set is fed by every error emission in the tree — the precondition D needs.
Untouched: the `ERR_*` constants, wrappers, and manual gating (all deleted in D),
and `ErrWrapped`.
