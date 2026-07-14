# plan-24-B: EXPORT valid only in `kind: "package"`

Last updated: 2026-07-05
Effort: small

`EXPORT` means "part of this package's public API, written into the compiled `.mfp`."
An executable has no importers, so `EXPORT` there is meaningless. Reject it: an `EXPORT`
top-level declaration in a project whose manifest `kind` is `"executable"` is a compile
error. Depends on plan-24-A (rename/default landed first).

It complements:

- `./mfb spec diagnostics error-codes` (`src/docs/spec/diagnostics/02_error-codes.md` — the build input for `errorCode::`)
- `./mfb spec language modules-and-packages` (EXPORT semantics; `src/docs/spec/language/13_modules-and-packages.md`)
- `./mfb spec tooling project-manifest` (`kind`; `src/docs/spec/tooling/01_project-manifest.md`)

## 1. Goal

- A new ERROR diagnostic fires when any top-level decl (FUNC/SUB/TYPE/UNION/ENUM/
  MUT/LET/RES/RESOURCE/FuncAlias) is declared `EXPORT` in a project with `kind == "executable"`.
- Points at the `EXPORT` keyword with a clear message ("`EXPORT` is only valid in a
  package project; this project is an executable").
- No effect on `kind == "package"` projects.

### Non-goals (explicit constraints)

- Does not change what `EXPORT` does in a package (still the `.mfp` export gate).
- Does not touch PRIVATE/PUBLIC handling.

## 2. Current State

- `project_kind(manifest)` → `"executable"|"package"` at `src/manifest/mod.rs:338-343`.
- Manifest/kind is available in `src/cli/build.rs` (lines ~159,263,294,376) but is NOT
  threaded into resolver/syntaxcheck (per plan-24 surface map §8). No current EXPORT-context check.
- Diagnostics are a hand-coded table `src/rules/table.rs` keyed by code `X-YYY-ZZZZ`; the
  registry mirror lives in `src/docs/spec/diagnostics/02_error-codes.md` and `build.rs`
  generates `errorCode::` constants from it. Syntaxcheck emits via `self.report(...)`.
- 36 test `.mfb` files use `EXPORT`; sampled ones are all `kind:"package"` (blast radius
  expected ≈ 0 executable violations, but must verify all 36).

## 3. Design Overview

The check is a syntaxcheck pass rule (syntaxcheck already walks top-level items and has
`self.report`). It needs the project `kind`. Thread `kind` (or a `bool is_package`) into
the syntaxcheck context at construction (`check_project*` in `src/cli/build.rs:210`
already has the manifest). Then: for each top-level item whose `visibility == Export`,
if `!is_package`, report the new error at the item's line.

Lowest-risk placement is syntaxcheck (post-monomorph, total-elaboration friendly), but the
error is about source-declared visibility, so it must run on a representation that still
carries the original `Export` (concrete AST does). Confirm `Export` survives monomorph
into the concrete AST items (it does — visibility is copied through `into_project`).

## Phases

### Phase 1 — Diagnostic + check

- [ ] Add error rule to `src/rules/table.rs` (next free code in the syntaxcheck range
      `1-104-XXXX`), e.g. `EXPORT_IN_EXECUTABLE`, severity Error, message about EXPORT
      requiring a package project.
- [ ] Mirror it into `src/docs/spec/diagnostics/02_error-codes.md` Constant Registry table
      (build input — required so `errorCode::` generates).
- [ ] Thread project `kind`/`is_package` into the syntaxcheck context
      (`src/syntaxcheck/mod.rs` constructor + `src/cli/build.rs:210` call site).
- [ ] Emit the error for every top-level `Export` item when `!is_package`
      (walk items in `src/syntaxcheck/mod.rs`).
- [ ] Verify none of the existing 36 EXPORT tests are executables (grep their project.json
      `kind`); migrate/annotate any that are (expected: none).
- [ ] Tests: `tests/visibility-export-in-executable-invalid` (executable + `EXPORT FUNC`
      → the new error) and confirm a `kind:package` `EXPORT` still compiles.

Acceptance: the invalid test emits `EXPORT_IN_EXECUTABLE` with the right code/line; all
package EXPORT tests still pass; `scripts/test-accept.sh` green.
Commit: —

## Validation Plan

- Function tests: n/a (language rule, not a builtin). Covered by `tests/visibility-export-*`.
- Doc sync: `src/docs/spec/diagnostics/02_error-codes.md` (new code) +
  `src/docs/spec/language/13_modules-and-packages.md` (state EXPORT is package-only).
- Acceptance: `scripts/test-accept.sh`.

## Open Decisions

- Report at first EXPORT only, or every EXPORT occurrence — recommend every occurrence
  (consistent with other per-decl diagnostics). (§Phase 1)

## Summary

Small, self-contained rule. Only real work is threading `kind` into syntaxcheck; the check
and diagnostic are routine. Blast radius on existing tests expected to be zero.
