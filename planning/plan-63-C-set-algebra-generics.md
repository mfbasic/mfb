# plan-63-C: Set-algebra source generics

Last updated: 2026-07-25
Effort (Human): medium (1h–2h)
Effort (AI): small (<1h)
Depends on: plan-63-B (the native `add`/`remove`/`contains`/`toList` members and
`FOR EACH x IN set` must exist and be green)
Produces: the set-algebra surface — `union`, `intersection`, `difference`,
`symmetricDifference`, `isSubset`, `isSuperset`, `isDisjoint`, and `toSet`
(build a set from a list) — as generic MFBASIC functions injected on
`IMPORT collections`. After C, `collections::` offers the full set toolkit and no
further codegen is required for it.

Prerequisites: see plan-63-A §Prerequisites. C adds: **plan-63-B is complete** —
`grep -n '"add"' src/builtins/collections.rs` shows `add` in `NATIVE_MEMBERS` and
the B set fixtures are green. If B is not complete, C cannot start, full stop. C
writes **only** MFBASIC source — it emits no Rust codegen and touches no runtime
helper.

References (read first):

- `src/builtins/collections_package.mfb` — the 19 existing `__collections_*`
  source generics (`sort`, `distinct`, `merge`, `partition`, …). C's functions
  are authored in exactly this style.
- `src/builtins/collections.rs:20` `FUNCTIONS` array and the
  `collections::<name> → __collections_<name>` monomorph rewrite
  (`collections.rs:70-95`).
- `mfb spec language collections` — the "source generics" group description that
  C extends.

## 1. Goal

- `collections::union(a, b)` returns a set containing every element of `a` and
  `b`; `collections::intersection(a, b)` every element in both;
  `collections::difference(a, b)` every element of `a` not in `b`;
  `collections::symmetricDifference(a, b)` elements in exactly one.
- `collections::isSubset(a, b)` is `TRUE` iff every element of `a` is in `b`;
  `isSuperset(a, b) = isSubset(b, a)`; `isDisjoint(a, b)` is `TRUE` iff they
  share no element.
- `collections::toSet(list)` returns a `Set OF T` with the list's distinct
  elements (dedup, first-occurrence order).
- All eight are pure: they return new values and mutate no argument, exactly like
  every other `collections::` helper.

### Non-goals (explicit constraints)

- **No codegen, no runtime helper.** If any function in C cannot be expressed in
  MFBASIC over B's primitives, that is a gap in B — record it as a B correction
  and add the missing primitive there, not a special case here. (Expectation:
  none is needed; all eight decompose into `add`/`contains`/`toList`/`FOR EACH`.)
- **No new type.** C introduces no type; `Set OF T` and `List OF T` come from
  A/B.
- **Element-type constraint is inherited, not re-stated.** These generics
  instantiate only for comparable `T` because `Set OF T` already requires it;
  C adds no separate constraint.

## 2. Current State

Source generics live in `src/builtins/collections_package.mfb` as
`FUNC __collections_<name>(...)` definitions and are listed in the `FUNCTIONS`
array (`src/builtins/collections.rs:20`). On `IMPORT collections` they are
injected and a call `collections::sort(x)` is rewritten to `__collections_sort(x)`
and monomorphized like any generic (`collections.rs:70-95`,
`mfb spec language collections`). Precedents C mirrors directly:

- `__collections_distinct` — already builds a de-duplicated result by scanning;
  `toSet` is its Set-valued sibling.
- `__collections_merge` — already folds one collection into another; `union` is
  the two-set analogue.
- `__collections_partition` — already returns a compiler-owned record;
  no new record type is needed for set algebra (all eight return `Set OF T` or
  `Boolean`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Existing `__collections_*` source generics (C adds 8) | 19 | `grep -c '^FUNC __collections_' src/builtins/collections_package.mfb → 19` |
| New generics C adds | 8 | union, intersection, difference, symmetricDifference, isSubset, isSuperset, isDisjoint, toSet |

### Verified properties

- **Every C function decomposes into B primitives.** Verified by writing each
  body against `add`/`contains`/`toList`/`FOR EACH` (see §4). None needs indexed
  access, ordered access, or a value payload — the operations B ships are
  sufficient. (If B's `add` names differ from the plan, C adopts the landed
  names; the decomposition is unaffected.)

## 3. Design Overview

Eight small generic functions, each a `FOR EACH` fold over B's primitives. This
is the lowest-risk sub-plan in the feature: pure source, no memory management, no
codegen, instantiated by the existing monomorph path. The only "risk" is a
transcription error in a body, caught immediately by the per-function test.

Order within C is irrelevant (the eight are independent), so C is a single phase.

**Rejected alternative:** *emit set algebra as native codegen for speed.*
Rejected — set algebra is O(n·m) membership folds whether written in MFB or
codegen, the constant factor is dominated by the (native) `contains` probe C
already calls, and native versions would triple the runtime surface for no
measured win. Source generics are the right altitude, matching how `distinct`
and `merge` are already done.

## 4. Detailed Design (the eight bodies)

Authored in `collections_package.mfb` (names illustrative; match B's landed
member names). Each is `PUB`/injected per the file's existing convention.

- `__collections_toSet OF T(xs AS List OF T) AS Set OF T` — `MUT r AS Set OF T = Set OF T { }`; `FOR EACH x IN xs: r = add(r, x)`; return `r`.
- `__collections_union OF T(a AS Set OF T, b AS Set OF T) AS Set OF T` — start from a copy of `a`, `FOR EACH x IN b: r = add(r, x)` (add is idempotent).
- `__collections_intersection OF T(a, b) AS Set OF T` — empty `r`; `FOR EACH x IN a: IF contains(b, x) THEN r = add(r, x)`.
- `__collections_difference OF T(a, b) AS Set OF T` — empty `r`; `FOR EACH x IN a: IF NOT contains(b, x) THEN r = add(r, x)`.
- `__collections_symmetricDifference OF T(a, b) AS Set OF T` — `union(difference(a,b), difference(b,a))`, or an inline two-pass fold.
- `__collections_isSubset OF T(a, b) AS Boolean` — `FOR EACH x IN a: IF NOT contains(b, x) THEN RETURN FALSE`; return `TRUE`.
- `__collections_isSuperset OF T(a, b) AS Boolean` — `RETURN isSubset(b, a)`.
- `__collections_isDisjoint OF T(a, b) AS Boolean` — `FOR EACH x IN a: IF contains(b, x) THEN RETURN FALSE`; return `TRUE`.

Register all eight in `FUNCTIONS` (`src/builtins/collections.rs:20`).

## Compatibility / Format Impact

- Additive: eight new injected `collections::` members. No existing member,
  type, or format changes. Source generics are compiled into each importing
  program, so there is no `.mfp` id to assign (unlike B's type id).

## Phases

> Keep checkboxes current in the same commit as the work.

### Phase 1 — The eight generics

One line: author, register, and test the set algebra as pure source generics.

- [ ] Add the eight `FUNC __collections_<name>` definitions to
      `src/builtins/collections_package.mfb` (§4).
- [ ] Register the eight names in `FUNCTIONS` (`src/builtins/collections.rs:20`).
- [ ] Tests: an `rt-behavior` fixture per operation (or one fixture exercising
      all eight) asserting, for `Set OF Integer` inputs, the standard identities:
      `union` size/membership, `intersection`/`difference` membership,
      `symmetricDifference = union − intersection`, `isSubset`/`isSuperset`
      reflexive + strict cases, `isDisjoint` on overlapping vs. disjoint sets,
      `toSet([1,1,2]) → {1,2}`. Include one `Set OF String` case.

Acceptance: the fixture(s) run and print the expected results for all eight
operations; `cargo test` green.
Commit: —

## Validation Plan

- Tests: the per-operation `rt-behavior` fixtures above.
- Coverage check: assert *outputs*, not just "no crash" — each operation's test
  must check membership/size, since a body that silently returns its first
  argument would still run clean.
- Runtime proof: the fixture output is the proof; e.g. `union({1,2},{2,3})` prints
  a 3-element set, `isDisjoint({1},{2})` prints `TRUE`.
- Doc sync: none in C — the man pages for these eight land in D.
- Acceptance: re-seed goldens for the new fixtures and run the acceptance pass;
  confirm no unrelated golden churned.

## Open Decisions

- **`symmetricDifference` naming/inclusion.** Recommend including it (standard set
  op, trivially `union(diff(a,b), diff(b,a))`). Drop only if the feature owner
  wants a minimal surface. — recommended: include.

## Corrections

<Filled in during execution.>

## Summary

C is the cheap, high-value cap on the set operations: eight source generics that
turn B's membership primitives into the familiar set algebra, mirroring how
`distinct`/`merge`/`partition` are already authored. No codegen, no memory work —
the risk is a typo in a fold, caught by the per-operation test.
