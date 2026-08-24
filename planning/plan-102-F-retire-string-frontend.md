# plan-102-F: Retire redundant string-based front-end type work

Last updated: 2026-08-23
Effort: medium (1h–2h) — re-measure at kickoff; may grow to large
Depends on: plan-102-E (elaborate + monomorph are fully typed; the front-end's
string type analysis is now the redundant copy).

Once `elaborate` owns name resolution, type attachment, `Var` classification, and
overload resolution (plan-102-C/D) and monomorph is typed (plan-102-E), the
string-based type analysis still living in the resolver and `syntaxcheck` passes is
largely a second, redundant implementation. This sub-plan measures that redundancy
and consolidates the type checks onto HIR (or `ir::verify`), deleting the
string-duplicated logic. It is the cleanup that makes "everything below the AST is
`ParameterType`" actually true, rather than "typed, plus a shadow string
type-checker."

See plan-102-A §3 for the full layering and the byte-identity gate.

References:

- `src/resolver/` (4072 lines) — `resolve_project` (`src/cli/build/mod.rs:327`),
  `resolve_augmented` (`:337`).
- `src/syntaxcheck/` (14441 lines) — `check_project_collect` (`:387`).
- `src/ir/verify/` — the post-lowering checker that some of these checks migrate to.
- `.ai/testing-gates.md`.

## Prerequisites

See plan-102-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-102-E complete | monomorph unify/substitute are typed; no `name()` shim | NOT MET until E lands |

## 1. Goal

- The type-analysis logic that `elaborate`/HIR now performs is removed from the
  string-based resolver/`syntaxcheck` passes: no rule is checked twice, once on
  strings and once on HIR.
- Diagnostics are unchanged (same codes, same wording, same line-ordered output) —
  the checks *move*, they do not change what they reject.

### Non-goals (explicit constraints)

- No change to which programs are accepted/rejected, which diagnostics fire, or
  their wording/order. This is a relocation/dedup, guarded by the diagnostic
  goldens.
- No change to compiled output — byte-identical `.ncode`/`.ncodesum` for accepted
  programs.
- Do **not** touch the AST-syntax rules that only exist pre-lowering (named
  arguments, EXIT flavors, inline-trap boundaries — the constructs total lowering
  erases). Those legitimately stay in `syntaxcheck` on the AST (see the plan-20-Z
  split note at `src/cli/build/mod.rs:350`).

## 2. Current State

The semantic rules are split across two passes today (plan-20-Z, documented at
`src/cli/build/mod.rs:350-358`): `syntaxcheck` for source-syntax rules and
`ir::verify` for relocated rules. Both run string-based type work. After plan-102-C/
D/E, `elaborate` produces a fully typed HIR that already resolved names, types,
`Var`, and overloads — so any resolver/`syntaxcheck` logic that re-derives those on
strings is redundant.

### Measured populations

| What | Count | Command |
|---|---|---|
| resolver size | 4072 | `find src/resolver -name '*.rs' \| xargs wc -l \| tail -1` |
| syntaxcheck size | 14441 | `find src/syntaxcheck -name '*.rs' \| xargs wc -l \| tail -1` |
| scalar-name `==` compares in resolver+syntaxcheck | UNMEASURED | measure at kickoff: `rg -n '== "(Integer\|String\|Boolean\|...)"' src/resolver src/syntaxcheck \| wc -l` |
| distinct rule codes in syntaxcheck vs ir/verify | UNMEASURED | measure at kickoff: `rg -oh '"[A-Z][A-Z_]*"' src/syntaxcheck \| sort -u \| wc -l` and same for `src/ir/verify` |

**This sub-plan's first act is the redundancy census** — which resolver/`syntaxcheck`
type checks are now duplicated by `elaborate`/HIR, and which are genuinely
AST-syntax-only and must stay. The census sets F's real effort (it may grow to
large). Do not schedule the consolidation until the census is run.

### Verified properties

- **A pre-existing two-pass split already exists.** `src/cli/build/mod.rs:350-358`
  and `src/rules/mod.rs`. So relocating checks between passes is an established,
  golden-guarded operation here, not a novel one. VERIFIED (documented).

## 3. Design Overview

1. **Census** the resolver/`syntaxcheck` type checks; tag each as (a) redundant with
   `elaborate`/HIR, (b) an AST-syntax-only rule that stays, or (c) a semantic rule
   that should move to HIR/`ir::verify`.
2. **Move (c) onto HIR/`ir::verify`** where cheaper on the typed representation;
   **delete (a)**; **leave (b)**.
3. Keep the merged, line-ordered diagnostic stream (plan-20-Z) intact so goldens do
   not reorder.

This overlaps the separately-identified "finish the syntaxcheck→ir::verify
relocation" cleanup (todo survey item #1) — but here it is scoped to the type-check
duplication the HIR introduces, not the whole relocation.

### Rejected alternatives

- **Leave the string front-end checks in place.** Rejected: then "everything below
  the AST is `ParameterType`" is false — a shadow string type-checker remains, and
  the Q3 string-compare counts in the front end (measured in plan-102-B §2) survive.

## Compatibility / Format Impact

None. Diagnostics unchanged (goldens guard this).

## Phases

> Census first; then split at kickoff if the census shows F is large.

### Phase 1 — redundancy census

- [ ] Enumerate resolver/`syntaxcheck` type checks; tag each (a)/(b)/(c). Record the
      list in this file.
- [ ] Measure the scalar-compare / rule-code counts (§2 UNMEASURED rows).

Acceptance: a tagged inventory exists in this plan; F's effort is re-estimated from
it.
Commit: —

### Phase 2 — consolidate

- [ ] Delete the (a) redundant checks; move the (c) checks to HIR/`ir::verify`;
      leave the (b) AST-syntax rules.
- [ ] Tests: the full diagnostic golden suite (every `*-invalid` fixture) — same
      codes, wording, order.

Acceptance: diagnostic goldens byte-identical (no code/wording/order change);
`artifact-gate all` no NEW diff; `cargo test` green; `test-accept` no NEW mismatch;
the front-end scalar-compare census (§2) dropped (record the delta).
Commit: —

## Validation Plan

- Tests: the whole `*-invalid`/diagnostic golden corpus (accept/reject unchanged);
  full suite.
- Coverage check: every relocated check has a fixture that still exercises it (the
  same fixture that did before the move).
- Runtime proof: `artifact-gate all` byte-identical; diagnostic goldens byte-identical.
- Doc sync: `src/rules/mod.rs` split comment (plan-20-Z) and the diagnostics spec
  (`spec/diagnostics/`) if a rule's home changes; `.ai/testing-gates.md` if the
  check-pass topology changes.
- Acceptance: `cargo test`; `artifact-gate all`; `test-accept`; fmt both crates.

## Open Decisions

- **How much of the broader syntaxcheck→ir::verify relocation to fold in.** Recommend
  scoping F strictly to the type-check duplication the HIR creates, and leaving the
  rest of that relocation to its own cleanup — do not braid the two. (§3)

## Corrections

<Filled in during execution.>

## Summary

The closing cleanup that makes the feature's headline true: with the front-end's
redundant string type analysis retired, `ParameterType` is genuinely the type
representation everywhere below the AST. Lowest-risk of the feature (a golden-guarded
dedup), but its size is unknown until the census runs — which is its first task.
