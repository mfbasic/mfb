# bug-531: `strings::find` raises on absence, `regex::find` returns `-1`

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: Fixed — `regex::find` raises `ErrNotFound` on absence. This is a
BREAKING change with no compile-time signal; see the migration note below.
Regression Test: `tests/rt-behavior/regex/regex-find-absence-rt`

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

- [x] Land `spikes/api-review/bug-531-find-absence/` (done).
- [x] `grep -rn "regex::find" src/ examples/ benchmark/ tests/ repository/` —
      enumerate **every** call site and classify each.
- [x] Record the absence contract of every `find`-family member across
      `strings`, `regex`, `collections` and `astrings`, **measured**.
- [x] Add a fixture pinning the desired behavior. Confirm it fails today.

**The measured census.** `regex::find` was the only sentinel in the tree — the
split was 4-to-1, not the 1-to-1 the report describes:

| member | absence | measured |
| --- | --- | --- |
| `strings::find(v, n)` | raises `ErrNotFound` (77050004) | ✓ |
| `collections::find(l, x)` | raises `ErrNotFound` (77050004) | ✓ |
| `collections::findIndex(l, p)` | raises `ErrNotFound` (77050004) | ✓ |
| `collections::findLastIndex(l, p)` | raises `ErrNotFound` (77050004) | ✓ |
| `regex::find(v, p)` | returned `-1` | ✗ — the one converged here |

That answers the open question about `collections`: `findIndex`/`findLastIndex`
already raise (`func_find_index.rs:17` states it in as many words — "rather than
returning a sentinel index"), so this is a two-package convergence and nothing in
`collections` needed to move.

**The caller sweep, complete.** `grep -rn "regex::find("` over
`src/ examples/ benchmark/ tests/ repository/ spikes/`:

- **Product code: zero call sites.** `examples/`, `benchmark/` and `repository/`
  contain no `regex::find` at all — the five `regex::` hits under `benchmark/`
  are all `findAll`, which is unchanged. No user-visible program in the tree
  relied on `-1`.
- `tests/acceptance/src/regex.mfb` — 9 assertions expecting `-1`, migrated to
  `expectTrap(..., errorCode::ErrNotFound)`, plus one that needed more care (see
  below).
- `tests/rt_regex_bounds.rs` — the 85-row `MATCHER_CORPUS` and the 1.2 MB
  `LARGE_SUBJECT` memory probe. Both migrated through a `findOrMinusOne` TRAP
  wrapper **inside the MFBASIC program**, not by editing the expectations: the
  corpus has a single outer `TRAP`, so a bare raising `find` would have collapsed
  every non-matching row to `raised 77050004` and destroyed the
  `match`/`findAll`/`replace` coverage that row carries. The recorded `f=-1`
  observable is byte-identical after the migration.
- `tests/rt_regex_span.rs` — the `find`/`findMatch` cross-check, same wrapper for
  the same reason.
- `tests/byte-identity/regex`, `tests/rt-behavior/regex/regex-from-string-rt`,
  `src/ir/tests.rs:2646` — every pattern matches; no absence, no migration.
- `tests/syntax/regex/func_regex_find_invalid` — arity/type diagnostics only.

**One call site the grep classified wrongly, found by running the suite.**
`tests/acceptance/src/regex.mfb`'s `TCASE "in-range start does not trap"`
asserted `expectNTrap(findAt("hello", "l", 5))`. It reads as a *range* assertion
and it is one — but its observable was "no trap of any kind", which conflated
"`start` is in range" with "a match exists". Those were the same question while
absence returned `-1` and are two questions now. Corrected per AGENTS.md's
four-question gate by keeping the intent and fixing the observable: the boundary
`start == len(value)` is now probed with a zero-length pattern that genuinely
matches there, and a second assertion pins that the same in-range boundary with a
*non*-matching pattern raises `ErrNotFound` and **not** `ErrIndexOutOfRange` —
which is the discriminator proving the boundary was accepted rather than
rejected. The case can still fail; it was not weakened.

Acceptance: met.
Commit: (this commit)

### Phase 2 — the convergence

- [x] `regex::find` raises `ErrNotFound` on absence; `ErrNotFound` added to its
      descriptor's `errors:` list. The lowering is one line of the `__regex_find`
      MFBASIC body: `RETURN -1` became
      `FAIL error(77050004, "Requested item, key, file, or resource was not found.")`.
- [x] Migrate every call site from Phase 1.
- [x] Rewrite the `regex` intro's per-member absence paragraph. The replacement
      states the *reason* for the split rather than listing it: a member reports
      absence as a value when its return type has one, and `find` returns an
      index, where every `Integer` is a position some search could legitimately
      report. `match`/`findAll`/`findAllMatches`/`findMatch`/`replace` keep their
      answers.
- [x] Cross-link `strings::find` and `regex::find`; show the `TRAP`-to-`-1`
      wrapper for callers who want the old shape (on the `regex::find` page, and
      as the second worked example).
- [x] No `collections` member needed the convergence — Phase 1 measured all four
      as already raising.

**One consequence the decision did not name, and its resolution.** The `regex`
package intro and `regex::findMatch`'s page both stated an *equality*:
`findMatch(value, pattern, start).start` **is** `find(value, pattern, start)`.
That is now false on absence, where `find` raises and `findMatch` reports a
no-match `MatchInfo`. `findMatch` was **not** converged, and the reason is the
decision's own rule: a record has room for an absent value and an index does not,
so the `-1` in `MatchInfo.start` is a documented no-match *value*, not a sentinel
standing in for a missing error channel. Both pages now state the equality as
holding wherever a match exists, and say what happens where none does. The spec
carries the same narrowing.

Acceptance: met — the fixture passes; `findAll`, `match`, `replace` and
`findMatch` are unchanged (asserted in the same fixture); no in-tree caller
relies on `-1` except through the documented wrapper.
Commit: (this commit)

### Phase 3 — regenerate + validation

- [x] Check whether adding `ErrNotFound` to `regex::find`'s `errors:` flips any
      `TYPE_INLINE_TRAP_DEAD_HANDLER` warning. **It cannot.** That warning is
      driven by `builtins::inline_builtin_is_infallible`, a census of *inline*
      builtin lowerings; `regex::` members are `Body::mfb` source bodies, and
      `src/ir/fallible.rs` treats "an imported package's export" as fallible
      unconditionally. `regex::find` was already on the fallible side before the
      change, so no warning could move in either direction. Confirmed by the
      full acceptance run, which compares every diagnostic-bearing `build.log`.
- [x] Regenerate the `.ncodesum` goldens (`bash scripts/regen-ncodesum.sh`).
- [x] `cargo test --release --no-fail-fast`; `scripts/test-accept.sh`.
- [x] `scripts/man-run-examples.sh regex --run` (17/17) and `strings --run`
      (84/84); `man-census.sh --memory-scope` 0 unclassified hits.
- [x] Update the spike so it asserts the converged behavior.
- [x] Write the release note naming the break — recorded below; the repository
      carries no CHANGELOG file, so it lives in this document and on the
      `regex::find` page, which shows the migration wrapper.

**Golden delta, and why it is exactly this.** `regen-ncodesum.sh` refreshed 143
goldens and **5** moved: `regex_codegen_cover_rt.{macos-aarch64, linux-x86_64,
linux-aarch64, linux-riscv64, windows-x86_64}.ncodesum` — all five targets of the
one byte-identity fixture that emits `__regex_find`. Three `.ir` goldens moved
(`byte-identity/regex`, `rt-behavior/regex/regex-from-string-rt`,
`rt-behavior/regex/regex-posix-classes-rt`), each by **one line**, and that line
is the `return -1` → `fail error(77050004, …)` op at source line 2194 — the body
kept its line count, so nothing shifted downstream.
`tests/rt-behavior/threads/thread-regex-rt` did **not** move, which was checked
rather than assumed: its `.ir` golden is the main module only and contains no
`__regex_` symbol at all, because the `IMPORT regex` lives in a sibling source
file. The full `artifact-gate.sh all` confirms nothing outside that set moved.

Acceptance: met.
Commit: (this commit)

## The break, stated

`regex::find(value, pattern[, start])` used to return `-1` when no match existed
and now raises `ErrNotFound` (`77050004`). **The return type did not move**, so
every existing call still compiles and nothing catches an unmigrated caller at
build time; the failure is a `TRAP` at run time in code that never had one. Two
migrations, both on the member's page:

* guard with `regex::match` (the intended shape — it answers the same question
  with a `Boolean` and never fails on absence); or
* keep the old shape with a four-line wrapper:

```
FUNC findOrMinusOne(v AS String, p AS String) AS Integer
  RETURN regex::find(v, p)
TRAP(err)
  RETURN -1
END TRAP
END FUNC
```

Nothing else in `regex` changed: `match` still returns `FALSE`, `findAll` and
`findAllMatches` still return empty lists, `findMatch` still returns a `MatchInfo`
whose `start` is `-1`, and `replace` still returns `value` unchanged.

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
