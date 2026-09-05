# bug-534: `regex` has no `split`, no `count`, and no `AttributedString` overloads, all of which `strings` has

Last updated: 2026-09-04
Effort: large (3h–1d)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `tests/` — new `rt_regex_surface_parity` fixture per member added

`regex` is deliberately shaped as the pattern-matching mirror of `strings`:
`match`↔`contains`, `find`↔`find`, `replace`↔`replace`. Three members of that
mirror are missing, and each is missing for a different reason.

**1. `regex::split`.** `strings::split` is literal-only. Splitting on a
*pattern* — whitespace runs, one-or-more delimiters, an alternation — is the
canonical reason to reach for a regex at all, and it has no member. A caller
cannot even fake it with `findAll`, because faking it needs each match's end
(bug-532) and `findAll` returns starts only.

**2. `regex::count`.** `strings::count` exists, so people will look for the
pattern version. It is trivial given `findAll` — `len(regex::findAll(v, p))` —
which is an argument for *not* adding it, except that the same argument applies
to `strings::count` and it exists anyway. The asymmetry is the defect, and the
resolution may be a documented pointer rather than a member.

**3. `AttributedString` overloads.** Every `strings` query member has one:
`mfb man strings displayWidth`, `padLeft`, `count`, `contains` and the rest all
end with "`value` may also be an `astrings::AttributedString`: the query runs on
its visible text and returns exactly what the `String` overload returns."
`regex` has none, so an `AttributedString` cannot be searched by pattern at all
without extracting its text first — losing the association with the attributes,
which is the entire point of the type.

The single correct behavior a fix produces: `regex` covers the pattern
equivalent of every `strings` operation it is a mirror of, or its intro states
which operations are deliberately absent and why.

References:

- `mfb man regex` — the four-member function table
- `mfb man strings split`, `strings count`, and the `AttributedString`
  paragraph on every `strings` query member
- `src/codegen/builtins/regex/`, `src/codegen/builtins/astrings/`
- Depends on: bug-532 (span/extraction — `split` cannot be built without it)

## Failing Reproduction

```
./target/release/mfb man regex
./target/release/mfb man strings | grep -E "split|count"
```

- Observed: `regex` lists exactly `find`, `findAll`, `match`, `replace`.
  `strings` lists `split` and `count`, and every `strings` query member carries
  an `AttributedString` paragraph that no `regex` member has.

- Expected: `regex::split` exists; `regex::count` exists or its absence is
  stated; `regex` members accept an `AttributedString` where `strings` members do.

The `split` gap made concrete, to become a Phase 1 fixture:

```
' Tokenize on runs of whitespace -- the textbook regex split.
' strings::split takes a literal, so it cannot collapse runs.
' regex has no split at all.
' Faking it with findAll needs each match's END, which findAll does not report.
```

Contrast cases, correct today:

- `regex::replace` *is* the pattern mirror of `strings::replace` and works.
- `strings::split`'s literal-only behavior is correct and documented for what
  it is; this bug does not ask it to grow patterns.
- `astrings`' design — "the query runs on its visible text and returns exactly
  what the `String` overload returns" — is a clean, uniform rule that a `regex`
  overload can adopt verbatim for the query members.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | surface census; target-independent |

## Root Cause

Not a defect — an incomplete mirror, with one hard dependency underneath it.

`split` is absent because it cannot be written with the current surface:
producing the pieces between matches requires each match's extent, and the
package reports only starts (bug-532). So `split` is blocked on the span work,
not on a decision.

`count` is absent because it is one call away from `findAll`, which is a
reasonable omission taken alone — it is the *asymmetry* with `strings::count`,
which is equally one call away from a scan, that makes it a discoverability
problem.

The `AttributedString` overloads are absent because the two packages were built
against different type sets: `strings` and `astrings` co-evolved, and `regex`
was specified over `String` only. Its intro says "The package defines no new
types. `pattern` and `replacement` are ordinary runtime `String` values" — true,
and it also never mentions `AttributedString`, so a reader has no signal that
the mirror stops here.

## Goal

- `regex::split(value, pattern, [limit])` exists and splits on pattern matches.
- The `regex::count` question is settled: either the member exists, or the
  `regex` intro says `len(findAll(...))` is the count and why there is no member.
- Every `regex` **query** member (`match`, `find`, `findAll`) accepts an
  `astrings::AttributedString` under the same rule `strings` uses — the query
  runs on the visible text and returns exactly what the `String` overload
  returns.
- The `regex` intro states which `strings` operations have no pattern
  equivalent, and why.

### Non-goals (must NOT change)

- `strings::split`'s literal-only behavior.
- The existing four `regex` members' signatures and results.
- **A rewriting `AttributedString` overload for `regex::replace`.** Remapping
  attribute spans across a pattern rewrite is a genuinely hard problem —
  a match can span attribute boundaries and the replacement has no
  corresponding extent — and is correctly out of scope here. The query members
  are the tractable half; say so rather than leaving the omission unexplained.
- The zero-width match rule, which `split` must respect exactly (see Fix Design).
- **Tempting wrong fix, forbidden:** implementing `split` on top of
  `regex::replace` by rewriting matches to a delimiter and splitting on that.
  It breaks whenever the input contains the chosen delimiter, which is
  unbounded. `split` waits for real spans.

## Blast Radius

- `src/codegen/builtins/regex/` — three new members (or two plus an intro
  paragraph), and overloads on three existing ones.
- **bug-532 is a hard prerequisite for `split`.** It must land first, or `split`
  cannot be implemented correctly. `count` and the `AttributedString` overloads
  are independent and can land first.
- `src/codegen/builtins/astrings/` — the overload seam. Adding a builtin
  overload has known traps (`os_alias` invisible to `resolve_func`; the registry
  strict matcher's resource-vs-value gate), so the `AttributedString` work is
  not as mechanical as it looks.
- `strings::split` — unaffected; check in Phase 1 whether its page should point
  at `regex::split` once it exists.
- `csv`, `json`, `http` — in-tree tokenizers that may be hand-rolling a pattern
  split today. `grep -rn "regex::" src/codegen/builtins/` in Phase 1; each is a
  validation case for the new member.

## Fix Design

Land in dependency order, smallest first.

**Stage 1 — `AttributedString` overloads on the query members.** Independent of
everything else, and it uses `astrings`' existing uniform rule verbatim. Follow
the builtin-overload checklist: a new overload's own body needs its `os_alias`
routing, and the registry strict matcher must be checked for the
`String`/`AttributedString` pairing.

**Stage 2 — the `count` decision.** Either add `regex::count(value, pattern,
[start])` as a thin wrapper over `findAll`, or add a sentence to the intro
directing the reader to `len(findAll(...))`. **Recommend the member**: it costs
almost nothing, it removes a discoverability gap, and `strings::count` sets the
precedent. A member that is one line and obvious is cheaper than a paragraph
explaining its absence.

**Stage 3 — `split`, after bug-532.** The semantics need three decisions, each
of which is a place to get it wrong:

- **A zero-width pattern.** `split("abc", "")` — the pattern matches at every
  position, so the pieces are `["", "a", "b", "c", ""]` or `["a","b","c"]`
  depending on the rule. Must be stated explicitly and must respect the
  package's existing "advance one scalar past an empty match" termination rule.
- **Leading and trailing empty pieces.** A match at position 0 produces an empty
  first piece. Keep it (faithful) or drop it (convenient)? **Recommend keep**,
  and let a caller filter — a `split` that silently drops pieces cannot be
  used to reconstruct the input.
- **A `limit`.** `strings::split`'s behavior here should be matched, whatever
  it is; Phase 1 records it.

Rejected: `split` returning spans rather than strings. It is a different member
(and falls out of bug-532's `findAllMatches` for free), and a `split` that does
not return the pieces is not a `split`.

Rejected: adding `regex::split` before bug-532 by re-running the engine per
piece. Quadratic, and it duplicates matching logic that will need deleting.

## Phases

### Phase 1 — census + decisions (no behavior change)

- [ ] Enumerate every `strings` member and mark whether `regex` has a pattern
      equivalent, whether it should, and why not if not. That table is the
      deliverable — the three gaps named here were found by reading, and the
      list may be longer.
- [ ] Record `strings::split`'s `limit` and empty-piece behavior exactly, so
      `regex::split` can mirror it.
- [ ] `grep -rn "regex::" src/codegen/builtins/ examples/ benchmark/` — find
      in-tree code hand-rolling a pattern split.
- [ ] Confirm bug-532's status; `split` is blocked until it lands.

Acceptance: the parity table is complete with a verdict per member;
`strings::split`'s edge behavior is written down.
Commit: —

### Phase 2 — `AttributedString` overloads (independent)

- [ ] Add the overloads to `regex::match`, `find`, `findAll`, following the
      builtin-overload seam checklist.
- [ ] Add the standard `astrings` paragraph to each page.
- [ ] State in the intro why `regex::replace` has no such overload.

Acceptance: each query member accepts an `AttributedString` and returns exactly
what its `String` overload returns.
Commit: —

### Phase 3 — `count` (independent)

- [ ] Add `regex::count`, or the intro paragraph, per the Stage 2 decision.

Acceptance: a reader looking for `strings::count`'s pattern equivalent finds an
answer on the `regex` package page.
Commit: —

### Phase 4 — `split` (after bug-532)

- [ ] Implement over the span-returning member from bug-532.
- [ ] Decide and document the zero-width, empty-piece and `limit` rules.
- [ ] Point `strings::split`'s page at it.

Acceptance: whitespace-run tokenization works; the zero-width case terminates
and matches the documented rule.
Commit: —

### Phase 5 — validation

- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh regex --run`, `astrings --run`, `strings --run`.
- [ ] Confirm byte-identical results on all three platforms — the dialect's
      portability guarantee applies to the new members too.

Acceptance: full suite green on macOS, Linux and Windows.
Commit: —

## Validation Plan

- Regression tests: per member added. For `split` specifically: a whitespace-run
  case, a zero-width-pattern case asserted to terminate, a leading-match case
  asserting the empty first piece, and a non-ASCII case pinning scalar indices.
- Runtime proof: an in-tree tokenizer from Phase 1 rewritten onto `regex::split`
  and producing identical output.
- Doc sync: the `regex` intro (the parity statement and the `replace`
  exclusion), the new member pages, `strings::split`'s cross-reference.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- `regex::count` as a member vs. an intro pointer. **Recommend the member.**
- `split`'s empty-piece rule. **Recommend keeping leading/trailing empties**,
  so the input is reconstructible.
- Whether a rewriting `AttributedString` overload for `regex::replace` is ever
  worth doing. **Recommend documenting the omission and stopping there** —
  span remapping across a pattern rewrite is a plan of its own.

## Summary

Three gaps with three different costs. The `AttributedString` overloads and
`count` are independent and can land immediately; `split` is blocked on
bug-532 and carries all the semantic risk (zero-width patterns and empty
pieces). The most useful part of Phase 1 is the full parity table — the three
gaps here were found by reading, and a member-by-member census is the only way
to know the list is complete.
