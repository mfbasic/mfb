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

- [x] Enumerate resolver/`syntaxcheck` type checks; tag each (a)/(b)/(c). Record the
      list in this file. **Tagged inventory (2026-08-23, all measured):**
      * **(a) redundant with `elaborate`/HIR: EMPTY (∅).** `elaborate`/HIR performs
        **zero** checking — `rg -c 'emit|report|diagnostic|show_diagnostic'
        src/hir/mod.rs` → 0. The *executed* feature deliberately kept elaboration
        check-free: C's Non-goals deferred check relocation, and D2 (overload
        relocation into elaborate) was proven moot because overload selection is
        instantiation-dependent (see plan-102-D Corrections). No rule is checked
        twice — once on strings and once on HIR — because HIR checks nothing. The
        premise of this sub-plan ("elaborate owns … overload resolution … the
        string-based type analysis is largely a second, redundant implementation")
        described the *envisioned* D, not the executed one.
      * **(b) AST-syntax-only rules (stay):** all of `syntaxcheck`'s source-path
        rules per the plan-20-Z split (`src/cli/build/mod.rs:350`) — named args,
        EXIT flavors, inline-trap boundaries, etc.
      * **(c) semantic rules that should move to HIR/`ir::verify`:** exactly the
        pre-existing plan-20-Z `syntaxcheck`→`ir::verify` relocation set — all 124
        `TYPE_*` codes appear in BOTH passes (`comm -12` of the two sorted code
        lists → 124; syntaxcheck-only → 0), with `RELOCATED_TO_IR_VERIFY`
        (`src/ir/verify/mod.rs:71`) governing which pass emits on the source path.
        Per this plan's own Open Decision, that relocation is its own ongoing
        cleanup and is NOT braided into F.
      * The **resolver** performs name/import/package resolution only — its 8
        diagnostic sites are import/package rules, not type checks.
- [x] Measure the scalar-compare / rule-code counts (§2 UNMEASURED rows).
      (Scalar-name `==` compares in resolver+syntaxcheck: **1** —
      `src/syntaxcheck/builtins.rs:39`; structural `strip_prefix("List OF …`)` ops in
      syntaxcheck: **5**; distinct rule codes: syntaxcheck **124**, ir/verify **124**,
      overlap **124**. The front end's string type-surface is tiny; the plan's
      "shadow string type-checker" does not exist as a *duplicate* — it is the
      plan-20-Z primary checker.)

Acceptance: a tagged inventory exists in this plan; F's effort is re-estimated from
it. VERIFIED — inventory above; effort re-estimate: **small** (the consolidation
phase is moot, see below).
Commit: 9dac5a8fc

### Phase 2 — consolidate

- [x] ~~Delete the (a) redundant checks; move the (c) checks to HIR/`ir::verify`;
      leave the (b) AST-syntax rules~~ — **moot by the census**: (a) is EMPTY
      (nothing to delete — `elaborate` checks nothing, so the HIR introduced zero
      duplication); (c) is the plan-20-Z relocation this plan's own Open Decision
      explicitly scopes OUT ("do not braid the two"); (b) stays by definition. There
      is no code change for F to make: the goal "no rule is checked twice, once on
      strings and once on HIR" ALREADY HOLDS (vacuously, and by measurement).
- [x] Tests: the full diagnostic golden suite — same codes, wording, order.
      (Trivially unchanged — no code change; the E full-suite run is the standing
      green: every diagnostic/golden binary passes, sole failure = the recorded
      `artifact_gate_all` baseline.)

Acceptance: diagnostic goldens byte-identical; `artifact-gate all` no NEW diff;
`cargo test` green; the front-end scalar-compare census dropped. VERIFIED —
goldens/gate/suite unchanged since E (no code change in F); the census "delta" is
recorded as the measured inventory above (nothing to drop: the front end held 1
scalar compare and 0 HIR-duplicated checks).
Commit: — (census-only; no code change)

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

- **The premise described the envisioned D, not the executed one (2026-08-23).**
  This sub-plan assumed `elaborate` would own overload resolution and enough type
  analysis to make the front end's string checks a "second, redundant
  implementation." The executed feature deliberately kept elaboration check-free:
  plan-102-D2 (overload relocation) was proven moot — overload selection is
  instantiation-dependent and cannot move above monomorph without changing results —
  and no type-checking was relocated into `elaborate`/HIR (C's Non-goals). The
  census therefore measured **zero** HIR-duplicated front-end checks
  (`src/hir/mod.rs` emits no diagnostics), 1 scalar type compare, and a 124/124
  syntaxcheck↔ir::verify rule-code overlap that is entirely the pre-existing
  plan-20-Z dual-pass split (out of F's scope per its own Open Decision). F's goal —
  "no rule is checked twice, once on strings and once on HIR" — holds already, so
  the consolidation phase is moot with measurement evidence, and F closes as a
  census-only sub-plan.

## Summary

The closing cleanup that makes the feature's headline true: with the front-end's
redundant string type analysis retired, `ParameterType` is genuinely the type
representation everywhere below the AST. Lowest-risk of the feature (a golden-guarded
dedup), but its size is unknown until the census runs — which is its first task.
