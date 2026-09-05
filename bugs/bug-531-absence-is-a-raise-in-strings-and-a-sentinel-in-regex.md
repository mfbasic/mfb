# bug-531: `strings::find` raises on absence, `regex::find` returns `-1`

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `tests/` — new `rt_find_absence_parity` fixture (Phase 1)

The two search packages report "not found" in incompatible ways.

`strings::find("abc", "z")` raises `ErrNotFound` (77050004). Its page is
emphatic about this being deliberate:

> … rather than returning a sentinel such as `-1`. When `needle` does not occur
> at or after `start` it raises `ErrNotFound`.

`regex::find("abc", "z")` returns `-1`. The `regex` package intro is equally
emphatic in the other direction:

> No regex function fails on the absence of a match: `match` returns FALSE,
> `find` returns `-1`, `findAll` returns an empty list … **`ErrNotFound` is never
> raised by this package.**

Both positions are defensible on their own terms. Literal search is usually
`contains`-guarded first, so absence is exceptional; regex search treats absence
as the common case, and `-1` is unambiguous because every real index is `>= 0`.
The packages even document the difference and point at each other.

The hazard is the migration. Swapping a literal search for a pattern search —
the single most common edit in this area — silently converts a raising call into
one that returns a sentinel. Code that was correct because a TRAP caught the
absence becomes code that feeds `-1` into an index expression:

```
LET i AS Integer = regex::find(text, pattern)
LET tail AS String = strings::mid(text, i, 5)     ' i is -1 when absent
```

Nothing warns. `strings::find`'s discipline — absence is an error you must
handle — is exactly what stops working when the needle becomes a pattern.

The single correct behavior a fix produces: either the two packages agree, or
the difference is impossible to cross accidentally — a `regex` member that
raises for callers who want the `strings` contract, and a diagnostic or
documented guard at the boundary.

References:

- `src/codegen/builtins/strings/func_find.rs:34-43,129`
- `src/codegen/builtins/regex/mod.rs` — the package intro's "never raised"
- Spike: `spikes/api-review/bug-531-find-absence/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-531-find-absence
./spikes/api-review/bug-531-find-absence/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
present:
  strings::find("abc", "b") -> returned 1
  regex::find("abc", "b")   -> returned 1

absent:
  strings::find("abc", "z") -> RAISED code=77050004
  regex::find("abc", "z")   -> returned -1

mid("abc", regex::find(...), 1) with no guard would slice at -1
```

  Note the first block: for a *present* match the two are interchangeable and
  return the identical value. That is what makes the substitution look safe.

- Expected: one contract, or a mechanism that makes the substitution visible.

Contrast cases, correct today:

- `regex::match` returns a `Boolean` and is the guard the regex model intends.
  A caller who uses it is fine; the trap is only for a caller who does not.
- `collections::find` — Phase 1 must record which contract *it* uses. If it
  raises, the sentinel is a `regex`-only exception; if it returns a sentinel,
  the split runs deeper than two packages.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | pure software; expected identical |

## Root Cause

Not a defect in either package — two coherent error models meeting at a seam
with nothing on it.

`strings` treats absence as exceptional and uses the language's error channel;
its page argues the case explicitly, and `strings::contains` exists as the
guard. `regex` treats absence as ordinary and returns a sentinel; its intro
argues that case explicitly, and `regex::match` exists as the guard.

Neither package is wrong in isolation. What is missing is any construct that
notices when a caller moves between them. The type is `Integer` on both sides,
so the compiler sees no change; only the error behavior differs, and error
behavior is not in the signature.

## Goal

**Decided (2026-09-04): converge on `ErrNotFound`. `regex::find` raises on
absence, matching `strings::find`.**

- `regex::find(value, pattern, [start])` raises `ErrNotFound` (77050004) when
  no match exists, and never returns `-1`.
- `strings::find`, `regex::find` and `collections::find`'s index-returning
  members are covered by one written rule, with any exception stated and
  justified.
- The `regex` package intro's "`ErrNotFound` is never raised by this package"
  is deleted, and the per-member absence behaviors are restated.

### Non-goals (must NOT change)

- `regex::findAll` returning an empty list, `regex::match` returning FALSE, or
  `regex::replace` returning `value` unchanged. **None of these is a
  sentinel-in-an-index**, and this is what makes the convergence narrow: an
  empty list is a correct representation of "no matches", a `Boolean` is a
  correct answer to a predicate, and an unchanged string is a correct result of
  rewriting nothing. Only `find` returns an index, and an index has no value
  that can mean "absent".
- `strings::contains`/`regex::match` as the guards.
- `ErrNotFound`'s code (77050004), which other members use.
- The `start` parameter's meaning, or the leftmost-unanchored search rule.
- **Tempting wrong fix, forbidden:** changing `strings::find` to return `-1`.
  It is the safer of the two contracts — an unhandled absence becomes a TRAP
  rather than an out-of-range index — and its page documents the choice at
  length. Converging *downward* to the sentinel would remove the protection
  from the package that has it.
- **Also forbidden:** keeping `-1` and adding a second raising member as the
  *final* state. That was this document's original recommendation and it is
  superseded; two members differing only in their absence contract is a choice
  every caller must now make correctly, which is the problem restated rather
  than solved.

## Blast Radius

`grep -rn '"find"' src/codegen/builtins/` and each member's `errors:` list, in
Phase 1:

- `src/codegen/builtins/strings/func_find.rs` — raises. Contract preserved.
- `src/codegen/builtins/regex/func_find.rs` — returns `-1`. The member this bug
  changes or augments.
- `src/codegen/builtins/collections/func_find.rs`,
  `func_find_index.rs`, `func_find_last_index.rs` — **verdicts required.**
  These decide whether the rule is "two packages disagree" or "the language has
  no rule". `findIndex`/`findLastIndex` return indices and are the closest
  analogue to both.
- `astrings` overloads of `strings::find` — must match whatever `strings::find`
  does.
- Every in-tree `regex::find` caller — `grep -rn "regex::find" src/ examples/
  benchmark/ tests/`; each is a place to check for an unguarded `-1`. Finding
  one in-tree would raise this bug's severity.
- `src/rules/table.rs` — if the fix is a diagnostic, it needs a rule code, and
  per the project's hazard note the *name* being free does not prove the *code*
  is.

## Fix Design

`regex::find` raises `ErrNotFound` on absence. `regex::match` is the guard, as
`strings::contains` is for `strings::find`, and both pages say so.

**This is a breaking change**, and it is the whole risk of the bug. Every
existing `IF regex::find(v, p) >= 0 THEN` and `LET i = regex::find(...)` still
compiles — the return type does not move — and now raises where it used to
return `-1`. There is no compile error to catch it; the failure is a TRAP at
run time in code that never had one. That makes the Phase 1 caller sweep a
**prerequisite**, not an audit: every in-tree call site must be migrated in the
same change, and the release note must name the break.

Migration for a caller who wants the old shape is one wrapper, and the page
should show it:

```
FUNC findOrMinusOne(v AS String, p AS String) AS Integer
  RETURN regex::find(v, p)
TRAP(err)
  RETURN -1
END TRAP
END FUNC
```

Two things must be reconciled with the change:

1. **The `regex` intro's global claim.** "No regex function fails on the
   absence of a match … `ErrNotFound` is never raised by this package" becomes
   false and must be rewritten per member — `findAll` empty, `match` FALSE,
   `replace` unchanged, `find` raises. The rewritten paragraph should say *why*
   `find` differs: it is the only member returning an index.
2. **`errors:` on the descriptor.** `regex::find` gains `ErrNotFound`, which
   feeds the rendered Errors table and any inline-TRAP reachability analysis.
   A member that previously could not fail now can, so check whether any
   `TYPE_INLINE_TRAP_DEAD_HANDLER` warning flips — in either direction.

Rejected: **keeping `-1` and documenting the seam** (this document's original
recommendation). It protects only readers, and the failure mode is a caller who
did not read either page because the substitution looked free.

Rejected: **keeping `-1` and adding a second raising member.** It leaves two
absence contracts in one package and pushes the choice onto every call.

Rejected: **a diagnostic warning when a `regex::find` result flows unguarded
into an index position.** It was the strongest option while `-1` stayed, and it
is unnecessary once `find` raises — there is no sentinel left to flow.

Rejected: **an `Optional`/nullable return.** The language has no such type in
this position.

## Phases

### Phase 1 — caller sweep + census (no behavior change)

- [ ] Land `spikes/api-review/bug-531-find-absence/` (done).
- [ ] `grep -rn "regex::find" src/ examples/ benchmark/ tests/ repository/` —
      enumerate **every** call site and classify each: guarded by a `>= 0` test,
      guarded by a preceding `regex::match`, or unguarded. This is a
      prerequisite for Phase 2, not an audit: each one raises after the change
      and must be migrated in the same commit.
- [ ] Record the absence contract of every `find`-family member across
      `strings`, `regex`, `collections` and `astrings`, measured — extend the
      spike rather than reading the pages. `collections::findIndex` /
      `findLastIndex` return indices and must be given a verdict: if they use a
      sentinel, they belong in this convergence too.
- [ ] Add a fixture pinning the desired behavior: `regex::find("abc", "z")`
      raises `ErrNotFound`. Confirm it fails today.

Acceptance: every in-tree caller classified; the family-wide contract table is
measured; the fixture fails for the documented reason.
Commit: —

### Phase 2 — the convergence

- [ ] `regex::find` raises `ErrNotFound` on absence; add `ErrNotFound` to its
      descriptor's `errors:` list.
- [ ] Migrate every call site from Phase 1.
- [ ] Rewrite the `regex` intro's per-member absence paragraph, replacing the
      "`ErrNotFound` is never raised by this package" claim.
- [ ] Cross-link `strings::find` and `regex::find`; show the
      `TRAP`-to-`-1` wrapper for callers who want the old shape.
- [ ] Apply the same convergence to any `collections` member Phase 1 found
      using a sentinel.

Acceptance: the Phase 1 fixture passes; `regex::findAll`, `match` and `replace`
are unchanged; no in-tree caller relies on `-1`.
Commit: —

### Phase 3 — regenerate + validation

- [ ] Check whether adding `ErrNotFound` to `regex::find`'s `errors:` flips any
      `TYPE_INLINE_TRAP_DEAD_HANDLER` warning — a member that could not fail
      now can.
- [ ] Regenerate the `.ncodesum` goldens the descriptor change shifts (run the
      regen scripts under **bash**).
- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh strings --run`, `regex --run`.
- [ ] Update the spike so it asserts the converged behavior.
- [ ] Write the release note naming the break.

Acceptance: full suite green; golden deltas are only regex's; the break is
documented.
Commit: —

## Validation Plan

- Regression test: a fixture asserting each `find`-family member's documented
  absence behavior, so a future change to either package fails a test rather
  than widening the split.
- Runtime proof: `spikes/api-review/bug-531-find-absence/`.
- Doc sync: `strings/func_find.rs`, `regex/func_find.rs`, both package intros,
  and `collections`' find-family pages if Phase 1 finds them divergent.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

**Decided (2026-09-04): converge on `ErrNotFound`.** `strings::find`'s contract
wins because an unhandled absence becomes a TRAP rather than a `-1` flowing
into an index expression, and because `find` is the only `regex` member whose
return type has no room for "absent".

Still open:

- Whether `collections::findIndex`/`findLastIndex` are in scope. **Decide from
  the Phase 1 census.** If they already raise, this is a two-package
  convergence; if they return a sentinel, the rule should cover them or state
  why lists differ from text.
- Whether to ship a `TRAP`-to-`-1` wrapper as a documented snippet or as a
  member. **Recommend the snippet** — a member exists only to undo the fix.

## Summary

Neither package was wrong in isolation, which is why this survived: there was no
line of code to point at, only a seam. The decision resolves it in favour of the
contract that fails loudly. The entire risk is now the breaking change — the
return type does not move, so nothing catches an unmigrated caller at compile
time, and the failure is a TRAP at run time in code that never had one. Phase 1's
call-site sweep is therefore a prerequisite, and the release note is part of the
fix.
