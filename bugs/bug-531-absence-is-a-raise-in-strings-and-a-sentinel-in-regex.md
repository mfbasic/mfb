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

- A caller cannot accidentally acquire a `-1` where they previously had a
  raise, without something telling them.
- Whichever way it is resolved, `strings::find`, `regex::find` and
  `collections::find` are covered by one written rule, with any exception
  stated and justified.

### Non-goals (must NOT change)

- `regex::findAll` returning an empty list, or `regex::match` returning FALSE.
  Both are correct for "no match" and neither is a sentinel-in-an-index.
- `strings::contains`/`regex::match` as the guards.
- `ErrNotFound`'s code (77050004), which other members use.
- **Tempting wrong fix, forbidden:** changing `strings::find` to return `-1`.
  It is the safer of the two contracts — an unhandled absence becomes a TRAP
  rather than an out-of-range index — and its page documents the choice at
  length. Converging *downward* to the sentinel would remove the protection
  from the package that has it.

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

Three candidate shapes, in increasing cost:

**A — document the seam properly.** Cross-link the two `find` pages with an
explicit "if you are porting from the other, the absence contract changes"
paragraph, and say it in both package intros. Cheapest, changes no behavior,
and is worth doing regardless. Weakness: it protects only readers.

**B — add a raising `regex` member.** `regex::findOrRaise`, or better, an
optional `regex::find(value, pattern, [start], [onMissing])` — no. The cleaner
form is a separate member with the `strings::find` contract, so a caller who
wants "absence is an error" can have it without hand-writing the guard. Keeps
both models available, adds surface. This is the shape that makes the
substitution safe rather than merely documented.

**C — a diagnostic.** Warn when a `regex::find` result flows unguarded into an
index position. Strongest, and the only option that catches the existing
mistake — but it needs dataflow the value checker does not obviously have, and
a new rule code.

**Recommend A now and B next.** A is nearly free and is the honest minimum; B
gives the porting caller a mechanical answer instead of a discipline. C should
be recorded and not attempted from this document — if it is worth doing, it is
a plan.

Rejected: changing `regex::find` to raise. The package intro's argument is
sound — absence is the common case for a pattern search, and `ErrNotFound`
never being raised is a documented, load-bearing property of the whole package
("No regex function fails on the absence of a match"). Changing it would break
every `IF regex::find(...) >= 0` in existence.

Rejected: an `Optional`/nullable return. The language has no such type in this
position, and introducing one for this is far out of scope.

## Phases

### Phase 1 — census + decision (no behavior change)

- [ ] Land `spikes/api-review/bug-531-find-absence/` (done).
- [ ] Record the absence contract of every `find`-family member across
      `strings`, `regex`, `collections` and `astrings`, measured — extend the
      spike rather than reading the pages.
- [ ] `grep -rn "regex::find" src/ examples/ benchmark/ tests/` — check every
      in-tree caller for an unguarded `-1`. **Any hit is a real bug and is
      fixed in this change**, not deferred.
- [ ] Decide between A, A+B, and A+B+C.

Acceptance: a measured contract table; every in-tree caller checked; the
decision recorded.
Commit: —

### Phase 2 — the seam documentation (shape A)

- [ ] Cross-link `strings::find` and `regex::find`, each stating the other's
      contract and the porting hazard.
- [ ] State the rule (and any exception) in both package intros.

Acceptance: both pages name the difference and the guard to use.
Commit: —

### Phase 3 — the raising member (shape B), if decided

- [ ] Add the `regex` member with the `strings::find` contract.
- [ ] Man page, examples, `errors:` list including `ErrNotFound`.

Acceptance: a caller porting from `strings::find` has a drop-in with the same
absence behavior.
Commit: —

### Phase 4 — validation

- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh strings --run`, `regex --run`.
- [ ] Re-run the spike.

Acceptance: full suite green.
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

- A, A+B, or A+B+C. **Recommend A+B.** A alone leaves the porting caller
  writing the guard by hand every time; C is a plan, not a bug fix.
- What the raising member is called. `findOrRaise` is explicit but ugly;
  reusing `find` with a flag reintroduces the ambiguity. Decide in Phase 3.

## Summary

Neither package is wrong, which is why this has survived: there is no line of
code to point at. The value is in Phase 1 — an actual sweep of in-tree
`regex::find` callers for an unguarded `-1`. If that sweep is clean, this is a
documentation-and-surface item; if it is not, the severity is higher than
recorded here and the fix is urgent.
