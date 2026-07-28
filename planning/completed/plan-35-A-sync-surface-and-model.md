# plan-35-A: `term::sync` surface + shadow model scaffold

Last updated: 2026-07-11
Overall Effort: huge (>3d)  — the whole plan-35 feature (sub-plans A–F)
Effort: medium (1h–2h)
Depends on: nothing

Add the `term::sync()` builtin end-to-end as a **present hook that is initially a
no-op**, and specify the shadow cell/grid model plus the console grid-storage
layout — without changing any rendering behavior yet. This is the low-risk
scaffold every later sub-plan builds on: after A, `term::sync()` compiles,
type-checks, documents, and runs as a clean no-op on the console and both `-app`
backends, and the spec describes the cell model and the arena header block that
Phase B will allocate.

Shared design of record: `planning/plan-35-shadow-grid-unify.md` (§3 design, §4.1
cell model, §4.2 grid storage — read first). Resolved decisions this sub-plan
encodes: **D1** `sync()` is mandatory (no auto-present); **D2** one arena header
block pointed to by reserved term-state slot 48; **D3** neutral `abi::` codegen.

## 1. Goal

- `term::sync() AS Nothing` exists as a first-class `term::` builtin: parsed,
  arity/type-checked, resolved, lowered, and emitted as a no-op runtime helper on
  console + macOS app + GTK app.
- The spec (`04_term-backend.md`) documents the shared cell model, the shadow
  cursor + current-attribute set, the **mandatory-present** contract ("nothing is
  displayed until `term::sync()`"), and the D2 console header-block layout.
- `mfb man term sync` renders; `package.md` lists `sync` in the synopsis.

### Non-goals

- **No rendering change.** All existing `term::` calls and console `io::write`
  keep their current immediate behavior; `term::sync` does nothing at runtime
  yet. Byte-gate (`scripts/artifact-gate.sh`) must stay green.
- No allocation of the grid yet (that is Phase B) — A only *specifies* the
  header-block layout in the spec and reserves slot 48 for it.

## 2. Current State

`term::` builtins are declared in `src/builtins/term.rs` (const per call +
`is_term_call` + `call_param_names`/`param_types`/`arity`/`call_return_type_name`),
have runtime specs in `src/target/shared/runtime/term_specs.rs` registered in
`src/target/shared/runtime/catalog.rs`, are emitted for console in
`src/target/shared/code/term.rs` (`lower_term_helper`) and for app via the
per-target `emit_app_term_helper` tables (macOS `src/target/macos_aarch64/app/app_io.rs:712`,
GTK `src/target/linux_gtk/app_io.rs:9`). The dispatch fork is
`src/target/shared/code/mod.rs:1110` (`app_mode`) / `:1118` (term). The set of
term-call enumerators that must recognize a new call: `src/syntaxcheck/builtins.rs`,
`src/syntaxcheck/helpers.rs`, `src/resolver/mod.rs`, `src/monomorph/lower.rs`,
`src/ir/lower.rs`, `src/ir/verify/mod.rs`. See plan-35 master §2.6 for the full
census. Man tooling: `scripts/update_man.sh` (function pages) +
`scripts/update_man_package.sh` (package overview).

## 3. Design

`term::sync` follows the exact declaration/registration path every other 0-arg,
`Nothing`-returning term call uses (`term::clear` is the closest template). The
console emit arm and both app arms are `RESULT_OK` no-ops in this sub-plan —
placeholders replaced in Phases C/D/E. The only substantive work is *documenting*
the model so B/C/D/E share one contract:

- **Cell** = glyph (u32 unichar; 0/space = blank) + fg + bg (packed
  `r|g<<8|b<<16`) + bold + underline.
- **Shadow cursor** (row, col) zero-based from top-left; **current attributes** =
  the existing global fg/bg/bold/underline slots (offsets 8/16/24/32) — reused,
  not new.
- **D2 console header block** (allocated in B): one arena block laid out
  `[rows:u64, cols:u64, cursorRow:u64, cursorCol:u64 | back cells… | front
  cells…]`, base pointer stored in reserved term-state slot **48**; slot 56 stays
  free.
- **Mandatory present:** drawing mutates the grid; only `term::sync()` presents,
  and `term::off` implies a final present. Document this prominently — it is the
  porting footgun.

## Phases

### Phase 1 — declare + register `term::sync`

- [ ] `src/builtins/term.rs`: add `SYNC = "term.sync"`; include in `is_term_call`,
      `call_param_names`/`param_types` (`&[]`), `arity` ((0,0)),
      `call_return_type_name` (`"Nothing"`); update the module unit-test tables
      (`ALL`, `NO_ARG`, and the group assertions).
- [ ] `src/target/shared/runtime/term_specs.rs`: add `TERM_SYNC_SPEC`
      (`symbol: "_mfb_rt_term_term_sync"`, no params, `Nothing`,
      `abi::IO_PRINT_CLOBBERS`).
- [ ] `src/target/shared/runtime/catalog.rs`: register `TERM_SYNC_SPEC`.
- [ ] Thread `term.sync` through the enumerators in `src/syntaxcheck/builtins.rs`,
      `src/syntaxcheck/helpers.rs`, `src/resolver/mod.rs`, `src/monomorph/lower.rs`,
      `src/ir/lower.rs`, `src/ir/verify/mod.rs` (mirror how `term.clear` is
      listed in each).

Acceptance: a program `IMPORT term` / `term::on()` / `term::sync()` / `term::off()`
compiles and type-checks; passing a wrong arity is rejected with the standard
diagnostic. Commit: —

### Phase 2 — no-op emit on all three backends

- [ ] `src/target/shared/code/term.rs`: add a `"term.sync"` arm to
      `lower_term_helper` that gates on `active` and returns `RESULT_OK` (no
      output).
- [ ] Add `"term.sync"` no-op arms returning `RESULT_OK` to both
      `emit_app_term_helper` tables (macOS `app/app_io.rs`, GTK `app_io.rs`).
- [ ] Tests: `tests/func_term_sync_valid` (call on + sync + off, and sync while
      off — both clean); `tests/syntax/term/*` accepts `term::sync()`.

Acceptance: `func_term_sync_valid` passes on the host; the same program built with
`mfb build -app` (macOS + GTK) runs and exits 0; `scripts/artifact-gate.sh` green.
Commit: —

### Phase 3 — docs

- [ ] `src/docs/man/builtins/term/sync.txt` (per `.ai/man_template.md`) + add
      `sync` to `package.md` synopsis (`scripts/update_man.sh` /
      `update_man_package.sh`). State the mandatory-present contract.
- [ ] `src/docs/spec/app/04_term-backend.md`: add the shared cell model, shadow
      cursor/current-attrs, the D2 header-block layout (slot 48), and the
      mandatory-present rule.

Acceptance: `mfb man term sync` and `mfb spec app term-backend` render with the
new content; docs describe "invisible until `sync()`". Commit: —

## Validation Plan

- Tests: `func_term_sync_valid`, `tests/syntax/term/*`; the whole `func_term_*`
  suite still green (no behavior change).
- Byte-gate: `scripts/artifact-gate.sh` (no codegen change for existing calls).
- Acceptance: `scripts/test-accept.sh` on host; a smoke `-app` build both targets.
- Doc sync: man `sync.txt` + `package.md`, spec `04_term-backend.md`.

## Summary

Pure additive scaffold: one new no-op builtin wired through the standard
declaration/registration/doc path, plus the spec that locks the shared model for
B–F. Zero runtime behavior change; the risk is only "did every enumerator learn
the new call" (caught by the compile + `func_term_sync_valid`).
