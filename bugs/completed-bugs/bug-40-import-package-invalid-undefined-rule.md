# bug-40: A corrupt imported `.mfp` reports a garbage `0-000-0000 UNKNOWN_RULE` diagnostic — the emit site uses an undefined rule name while a reserved rule goes unused

Last updated: 2026-07-08
Effort: small (<1h)

`src/resolver/packages.rs::install_package_type_names` (`:70`) reports
`"IMPORT_PACKAGE_INVALID"` when `binary_repr::read_package_type_exports` fails on a
malformed `packages/<name>.mfp`. But `"IMPORT_PACKAGE_INVALID"` is **not defined**
in the rule table (`src/rules/table.rs`) nor in the embedded spec
(`src/docs/spec/diagnostics/01_rule-codes.md`) — it is referenced only at that one
call site. `rules::rule_for` matches on `rule.name` and, finding none, returns the
sentinel `Rule{ code:"0-000-0000", name:"UNKNOWN_RULE", message:"unknown diagnostic
rule" }` (`src/rules/mod.rs:115-125`). So the user importing a corrupt `.mfp` sees
`error[0-000-0000 UNKNOWN_RULE]: unknown diagnostic rule` followed by the real
detail line — a garbage diagnostic identity. (`had_error` is still set, so the build
correctly fails; only the reported code/name/message is wrong.)

The mirror image: `IMPORT_MISSING_PACKAGE` (code `2-201-0001`) **is** defined in the
table (`table.rs:179-183`) and the spec, but is **never emitted** anywhere in the
tree — a dead rule. Its slot 2-201-0001 is the natural home for the malformed-`.mfp`
case, strongly suggesting the emit site and the table drifted apart (a rename that
updated one side only).

The single correct behavior a fix produces: importing a corrupt `.mfp` reports a
**defined** import diagnostic (proper code/name/message), and every rule in the
table is either emitted or intentionally reserved — no emit site references an
undefined rule name.

Severity MEDIUM: a user-facing wrong diagnostic on a real, reachable error path
(corrupt package import); the dead-rule half is LOW.

References:

- `src/resolver/packages.rs:70` (`report("IMPORT_PACKAGE_INVALID", …)` — undefined
  name), `:345` (existing test `present_mfp_that_is_garbage_is_reported` writes
  `b"not a real package"` and only asserts `had_error`, so it never caught the wrong
  identity).
- `src/rules/mod.rs:115-125` (`rule_for` → `UNKNOWN_RULE`/`0-000-0000` sentinel).
- `src/rules/table.rs:179-183` (`IMPORT_MISSING_PACKAGE` 2-201-0001, defined but
  never emitted).
- `src/docs/spec/diagnostics/01_rule-codes.md:287` (spec entry for 2-201-0001).
- Contrast: sibling import diagnostics in `packages.rs` use defined rules
  (`IMPORT_PACKAGE_NOT_DECLARED`/`_NOT_INSTALLED`/`_MANIFEST_INVALID`/
  `_NAME_MISMATCH`/`_KIND_INVALID`), so a malformed source-package *directory*
  reports cleanly — only the malformed *.mfp binary* path is broken.
- Per `.ai/compiler.md`: any error-code add/rename must update the embedded
  `mfb spec diagnostics` in the same change.
- Found during goal-01 review of `src/resolver/**` + `src/rules/**`.

## Failing Reproduction

Project declares `shape` in `project.json` `packages`, with a `packages/shape.mfp`
containing garbage bytes; build it (or run the existing
`present_mfp_that_is_garbage_is_reported` test but assert the diagnostic identity):

- Observed: `error[0-000-0000 UNKNOWN_RULE]: unknown diagnostic rule` + the detail.
- Expected: a defined import diagnostic, e.g.
  `error[2-201-0001 IMPORT_MISSING_PACKAGE]: …` (or a new `IMPORT_PACKAGE_INVALID`
  rule), with a message describing the malformed package.

Contrast: a garbage source-package directory manifest reports a proper code today.

## Root Cause

The emit site and the rule table drifted: `packages.rs:70` emits a name absent from
`RULES`, while the reserved `IMPORT_MISSING_PACKAGE` slot is never emitted. `rule_for`
silently degrades an unknown name to the `0-000-0000` sentinel instead of failing
loudly.

## Goal

- The corrupt-`.mfp` path emits a defined diagnostic; no emit site references an
  undefined rule name; the table, spec, and `errorCode::` constants stay in sync.

### Non-goals (must NOT change)

- The build-fails behavior (`had_error` already set) — only the diagnostic identity.
- The deliberate dual-code entries at 2-205-0001/0002 (documented, harmless).

## Blast Radius

- `packages.rs:70` (the one undefined-name site). A tree-wide sweep should confirm
  no other emit site references a name absent from `RULES` (consider a build-time or
  test-time assertion that every reported rule name resolves).

## Fix Design

Reconcile the two: either (a) point `packages.rs:70` at `IMPORT_MISSING_PACKAGE`
(2-201-0001) and give it a message covering the malformed-`.mfp` case, or (b) add a
new `IMPORT_PACKAGE_INVALID` rule to `RULES` + the spec + the `errorCode::` registry
and keep `IMPORT_MISSING_PACKAGE` for the "declared but no file" case. Recommended:
(a) if the two are semantically the same, else (b). Add a test asserting the emitted
code/name (not just `had_error`). Consider making `rule_for` on an unknown name a
debug assertion so this class fails loudly.

## Phases

### Phase 1 — failing test + audit

- [x] Strengthen `present_mfp_that_is_garbage_is_reported` to assert the diagnostic
      code/name; confirm it is `0-000-0000`/`UNKNOWN_RULE` today.
- [x] Sweep all `report(...)`/`show_*` call sites for names absent from `RULES`.
- [x] Table/spec confirmation complete (above).

### Phase 2 — the fix

- [x] Reconcile the emit name with the table (option a or b); update table + spec +
      errorCode registry together.

### Phase 3 — validation

- [x] `scripts/test-accept.sh`; the strengthened test passes; `mfb spec diagnostics`
      matches.

## Validation Plan

- Regression test(s): the identity-asserting corrupt-`.mfp` test + the sweep.
- Doc sync: update `src/docs/spec/diagnostics/01_rule-codes.md` and the errorCode
  registry per `.ai/compiler.md`.
- Full suite: `scripts/test-accept.sh`.

## Summary

An import diagnostic name drifted from its table entry, degrading a real error path
to the `UNKNOWN_RULE` sentinel while leaving `IMPORT_MISSING_PACKAGE` dead;
reconciling them (and asserting emitted identities) fixes both, with a spec update.

## Resolution

Reconciled the emit site and the table by **reusing the reserved slot** (a merge of
options a/b): renamed the dead rule at `2-201-0001` from `IMPORT_MISSING_PACKAGE`
to `IMPORT_PACKAGE_INVALID` — the exact name the emit site
(`resolver/packages.rs:70`) already used — with the message "imported package binary
could not be read". The emit site itself was already correct and needed no change;
only the table half had drifted. This eliminates the dead rule and makes the
corrupt-`.mfp` path report a defined `error[2-201-0001 IMPORT_PACKAGE_INVALID]`
diagnostic instead of `0-000-0000 UNKNOWN_RULE`.

To make this class of drift fail loudly, `rules::rule_for` now `debug_assert!`s when
a name is absent from `RULES` (release builds keep the graceful sentinel fallback).
A tree-wide sweep of all rule-name string literals confirmed `IMPORT_PACKAGE_INVALID`
was the only emit-site name reaching `rule_for` that was undefined (the
`PACKAGE_BINARY_REPRESENTATION_VERIFY_*` names never reach `rule_for`: the package
path embeds them in an `Err` string and the source path filters to
`RELOCATED_TO_IR_VERIFY`).

The diagnostic rule registry (`01_rule-codes.md`) is a manual mirror of `RULES` and
is **not** the `errorCode::` build input — that is `02_error-codes.md` (runtime error
codes, a disjoint namespace), which needed no change here.

Files changed:

- `src/rules/table.rs` — rename `2-201-0001` entry to `IMPORT_PACKAGE_INVALID` +
  new message.
- `src/rules/mod.rs` — `debug_assert` in `rule_for`; `#[cfg(test)]` `code_and_name`
  helper; new `tests` module (identity + dead-name-gone + name-uniqueness).
- `src/resolver/packages.rs` — strengthened `present_mfp_that_is_garbage_is_reported`
  to assert the emitted identity (the emit site was already correct).
- `src/docs/spec/diagnostics/01_rule-codes.md` — mirror the renamed rule.

Tests: `present_mfp_that_is_garbage_is_reported` panicked on the undefined name
before the table fix (proving the bug reachable) and passes after; full
`cargo test --bin mfb` is green (2425 passed).
