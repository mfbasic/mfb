# bug-534: `regex` has no `split`, no `count`, and no `AttributedString` overloads, all of which `strings` has

Last updated: 2026-09-05
Effort: large (3h–1d)
Severity: MEDIUM
Class: Footgun

Status: **FIXED** (2026-09-05, `fe7903170`)
Regression Test: `tests/rt-behavior/regex/regex-surface-parity-rt`

**Two claims in this document were wrong and are corrected below where they
appear**, both found by re-running the census rather than reading:

- The Failing Reproduction said `mfb man regex` lists "exactly `find`, `findAll`,
  `match`, `replace`". It listed SIX: bug-532's `findMatch` and `findAllMatches`
  were already there. (`scriptOf` is `internal_only: true` — `mfb man regex
  scriptOf` answers "unknown regex function", so it is not on the public surface
  either.) The gap was three members short of the four the doc implied, not four.
- Fix Design Stage 3 said to match "`strings::split`'s behavior [for `limit`],
  whatever it is". **`strings::split` has no `limit` parameter** — its descriptor
  registers `value` and `delimiter` (alias `separator`) and nothing else
  (`src/codegen/builtins/strings/func_split.rs:register`). So `regex::split` gets
  none either, and the third of Stage 3's "three semantic decisions" was not a
  decision at all.

**bug-532 landed, so Phase 4's prerequisite is satisfied.** `regex::findMatch`
and `regex::findAllMatches` now report each match's `start`, `endIndex`, `text`
and capture groups (`regex::MatchInfo` / `regex::Group`), so `split` can be
written the way this document asks for it — one pass, no re-running the engine
per piece, no delimiter workaround. Concretely: iterate
`regex::findAllMatches(value, pattern)`, emit `strings::mid(value, cursor,
m.start - cursor)` per match and set `cursor = m.endIndex`, then emit the tail.
bug-532 deliberately did NOT take `split`, because this document owns the three
semantic decisions Stage 3 lists (the zero-width-pattern rule, whether
leading/trailing empty pieces are kept, and `limit`) and deciding them elsewhere
would have decided them without the analysis. Stages 1 and 2 (`AttributedString`
overloads, `count`) were already independent and remain so.

One thing bug-532 settled that Phase 1 should not re-derive: the zero-width rule
lives in exactly one place, `__regex_matchResults`, which `findAll`,
`findAllMatches` and `replace` all consume. A `split` built on
`findAllMatches` inherits it rather than restating it.

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

- `mfb man regex` — the function table (six members before this change, eight after)
- `mfb man strings split`, `strings count`, and the `AttributedString`
  paragraph on every `strings` query member
- `src/codegen/builtins/regex/`, `src/codegen/builtins/astrings/`
- Depends on: bug-532 (span/extraction — `split` cannot be built without it)

## Failing Reproduction

```
./target/release/mfb man regex
./target/release/mfb man strings | grep -E "split|count"
```

- Observed (MEASURED, not read): `regex` lists `find`, `findAll`,
  `findAllMatches`, `findMatch`, `match`, `replace` — six, not four.
  `strings` lists `split` and `count`, and every `strings` query member carries
  an `AttributedString` paragraph that no `regex` member has. The three real gaps
  are the ones this document names; the member COUNT in the line above was stale.

  The RED probes, captured before each member existed:

  ```
  error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: Built-in package `regex` does not export `regex.count`.
  error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: Built-in package `regex` does not export `regex.split`.
  error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: Call to `regex.match` has argument type(s) (AttributedString, String), expected String, String.
  error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: Call to `regex.find` has argument type(s) (AttributedString, String), expected String, String[, Integer].
  error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: Call to `regex.findAll` has argument type(s) (AttributedString, String), expected String, String[, Integer].
  error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: Call to `regex.findMatch` has argument type(s) (AttributedString, String), expected String, String[, Integer].
  error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: Call to `regex.findAllMatches` has argument type(s) (AttributedString, String), expected String, String[, Integer].
  ```

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
- **bug-532 was a hard prerequisite for `split`. It landed on 2026-09-05**, so
  `split` is implementable now. `count` and the `AttributedString` overloads
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
- **A `limit`.** ~~`strings::split`'s behavior here should be matched, whatever
  it is; Phase 1 records it.~~ **There is nothing to match: `strings::split` has
  no `limit` parameter.** So `regex::split` has none, and every piece is always
  returned. Recorded on both pages.

Rejected: `split` returning spans rather than strings. It is a different member
(and falls out of bug-532's `findAllMatches` for free), and a `split` that does
not return the pieces is not a `split`.

Rejected: adding `regex::split` before bug-532 by re-running the engine per
piece. Quadratic, and it duplicates matching logic that will need deleting.

## Phases

### Phase 1 — census + decisions (no behavior change)

- [x] Enumerate every `strings` member and mark whether `regex` has a pattern
      equivalent, whether it should, and why not if not. **Done — the table is
      now ON the `regex` package page** (`mfb man regex`, "What `strings` has
      that `regex` does not"), so it is a product artifact rather than a note in
      this file. Measured surface: `strings` exports 39 public members, `regex`
      exported 6. The verdict per member, in four groups:

      | group | `strings` members | `regex` equivalent |
      |---|---|---|
      | searching query | `contains`, `find`, `count` | `match`, `find`, **`count` (added)** |
      | searching decomposition | `split` | **`split` (added)** |
      | searching rewriter | `replace` | `replace` (already present) |
      | measurement / decomposition of the text itself | `byteLen`, `displayWidth`, `graphemes`, `graphemesCount`, `graphemeAt`, `toBytes`, `toScalars`, `fromScalars` | NONE, and correctly so — they ask about the text, not about a pattern in it |
      | position-anchored | `startsWith`, `endsWith`, `startsWithAny`, `endsWithAny`, `stripPrefix`, `stripSuffix`, `left`, `right`, `mid` | NONE — a pattern expresses these as `^p` / `p$` with `match`, and a span from `findMatch` is what `mid` slices |
      | non-searching rewriter | `upper`, `lower`, `caseFold`, `normalizeNfc`, `trim`, `trimStart`, `trimEnd`, `trimChars`, `padLeft`, `padRight`, `repeat`, `join`, `isDigit`/`isLetter`/`isLower`/`isUpper`/`isWhitespace` | NONE — they transform or classify the whole value, so there is nothing for a pattern to select |

      The list was NOT longer than the three gaps this document named. That is
      the census's finding, not an assumption.

- [x] Record `strings::split`'s `limit` and empty-piece behavior exactly, so
      `regex::split` can mirror it. **MEASURED by running programs**, not read:

      ```
      split(",a,,", ",")   n=4 [<>, <a>, <>, <>]      leading, interior and trailing empties KEPT
      split("abc", "|")    n=1 [<abc>]                no match -> one element
      split("", ",")       n=1 [<>]                   empty value -> one element
      split("XX", "X")     n=3 [<>, <>, <>]           adjacent matches -> empty between
      split("abc", EMPTY)  RAISED 77050002            empty delimiter refused
      ```

      **There is no `limit` parameter** (`func_split.rs:register` lists `value`
      and `delimiter` only). The rule is: N matches -> N+1 pieces, always, so the
      result is never empty and the input is reconstructible. `regex::split`
      reproduces all five lines; the fixture asserts the `,a,,` / `;a;;` pair on
      the same run so the two cannot drift.

- [x] `grep -rn "regex::" src/codegen/builtins/ examples/ benchmark/` — find
      in-tree code hand-rolling a pattern split. **Result: 16 hits, of which 11
      are code and 5 are prose. NO in-tree stdlib package uses `regex` at all** —
      `csv`, `json` and `http` are hand-written scanners with zero `regex::`
      hits, so the Blast Radius line calling them "validation cases for the new
      member" is refuted by its own prescribed command. What the grep DID find:

      * `benchmark/mfb/src/regexbench.mfb:32` and `:77`, and
        `benchmark/mfb/src/main.mfb:871` — literally `len(regex::findAll(...))`,
        the idiom `regex::count` replaces.
      * `examples/browser/display/src/lib.mfb:49` — `regex::replace(t, " +", " ")`
        to collapse whitespace runs before a literal `strings::split`. That IS
        the hand-rolled pattern split, done in two passes because there was no
        one-pass member. The fixture pins that the one-pass `regex::split(t, " +")`
        reproduces it byte-for-byte, including on a leading-run input where the
        empty first piece is the interesting case.

- [x] Confirm bug-532's status; `split` is blocked until it lands. **Landed
      2026-09-05** — see the note under Status for the exact shape `split` can
      now be built on.

Acceptance: MET. The parity table is complete with a verdict per member and now
lives on the package page; `strings::split`'s edge behavior is written down and
measured.
Commit: folded into the single commit below.

### Phase 2 — `AttributedString` overloads (independent)

- [x] Add the overloads to `regex::match`, `find`, `findAll`, following the
      builtin-overload seam checklist. **Also `findMatch`, `findAllMatches`, and
      the two new members `count` and `split` — all SEVEN query members**, so the
      tier is "every public member but `replace`" rather than an arbitrary subset.

**The seam investigation, which the Blast Radius expected to be the hard part.**
`strings` gets this from a hardcoded Tier-A/Tier-B table in
`src/codegen/builtins/strings/mod.rs` (`is_tier_a_query` / `is_tier_b_transform`
/ `tier_b_transform_impl`). There is no generic mechanism. But extending it to
`regex` turned out to be the SMALLEST of the four phases, not the largest,
because both consumers are single points:

* the return-type seam is ONE function. Every one of the ~20 call sites in
  `ir/shape.rs`, `ir/lower.rs` and `ir/verify/compat.rs` funnels through
  `builtins::resolve_call_return_type_typed`
  (`grep -rn resolve_call_return_type_typed src/`), which already dispatches
  per-owning-package. `regex` becomes the fourth package with a co-located
  resolver, beside `general`, `vector` and `strings` — 4 lines.
* the lowering seam is ONE predicate. `attributed_string_type()` appears in
  exactly one file tree-wide (`grep -rln attributed_string_type src/` →
  `src/ir/lower.rs`), and the Tier-A wrap is one `if`. Adding `|| regex::is_tier_a_query(...)`
  is 2 lines.

So the whole phase is `regex::is_tier_a_query` + `regex::resolve_return_type`
(new, ~35 lines with comments), a 4-line branch in `builtins/mod.rs`, and a
2-line condition in `ir/lower.rs`. No `os_alias` routing was needed (Tier-A never
re-targets the call — it rewrites the ARGUMENT to `toString(a)` and reuses the
`String` body), and the registry strict matcher's resource-vs-value gate does not
apply because no `regex` parameter is a resource. This is a defect with a small
fix, not a product decision.

- [x] Add the standard `astrings` paragraph to each page. Done on all seven
      query members, verbatim from `strings`: "`value` may also be an
      `astrings::AttributedString`: the query runs on its visible text and returns
      exactly what the `String` overload returns (same value, type, and errors)."
- [x] State in the intro why `regex::replace` has no such overload. Done —
      `mfb man regex`, "Attributed text".

Acceptance: MET. Measured on one run, each member compared against its own
`String` overload:

```
astr match agrees=TRUE  find agrees=TRUE  findAll agrees=TRUE
findMatch agrees=TRUE   findAllMatches agrees=TRUE  count agrees=TRUE  split agrees=TRUE
```

and `regex::replace(AttributedString, …)` still fails to build with
`TYPE_CALL_ARGUMENT_MISMATCH`, so the omission is enforced rather than merely
documented. Two new unit tests keep the two lists from drifting:
`every_public_query_member_is_tier_a_and_replace_is_not` derives the expected
membership FROM the registry (so a member added without a Tier-A row fails), and
`attributed_string_resolves_the_string_overloads_return` pins the resolved types.
Commit: folded into the single commit below.

### Phase 3 — `count` (independent)

- [x] Add `regex::count`, or the intro paragraph, per the Stage 2 decision.
      **The member**, as recommended. `regex::count(value, pattern[, start])`,
      a `Body::mfb` body that is `len(__regex_matchResults(...))` — the same walk
      `findAll` uses, so `count(v,p,s) = len(findAll(v,p,s))` holds by
      construction rather than by agreement. Signature mirrors `findAll`'s
      optional `start`, which `strings::count` does not have but every other
      `regex` search member does.

**The empty-pattern decision, from the landed precedent rather than from
scratch.** bug-529 wrote the rule into the `strings` package description ("The
empty needle") and it has a three-way split, of which the third group is
"**counts or rewrites every occurrence** — `count`, `split` and `replace` refuse,
raising `ErrInvalidArgument`". The same paragraph already says "There are no
exceptions. `regex::` reaches the same answers from the other direction." So
`regex::count` and `regex::split` fall in the refusing group by the rule as
written, alongside `strings::count`, `strings::split` (which already refused —
measured above) and, since bug-533, both `replace` members. Both new members
raise `77050002`. The rule paragraph in `strings/mod.rs` and its spec mirror in
`unicode/02_strings-model.md` now name `regex::count` and `regex::split`
explicitly, so the cross-package claim is not left implicit.

The one consequence worth stating, and it is stated on the page: `count` and
`len(findAll(...))` agree on every pattern EXCEPT the empty one, where `findAll`
answers and `count` refuses. That is not a new asymmetry — `strings::find`
answers an empty needle while `strings::count` refuses it, for the same reason.

Acceptance: MET.
Commit: folded into the single commit below.

### Phase 4 — `split` (after bug-532)

- [x] Implement over the span-returning member from bug-532. One pass over
      `__regex_matchResults` — the same helper `findAll`, `findAllMatches`,
      `count` and `replace` consume — emitting
      `strings::mid(value, cursor, mstart - cursor)` per match and setting
      `cursor = r.pos`, then the tail. It is `__regex_replace`'s loop with the
      replacement expansion dropped. The engine is never re-run per piece, and
      the forbidden replace-to-a-delimiter workaround was not used.
- [x] Decide and document the zero-width, empty-piece and `limit` rules:
      * **zero-width** — INHERITED from `__regex_matchResults`, not restated.
        `split("abc", "x*")` yields `["", "a", "b", "c", ""]` (4 matches, 5
        pieces) and terminates; `split("aba", "a*")` yields `["", "b", ""]`.
      * **empty pieces** — KEPT, leading, interior and trailing, exactly as
        `strings::split` keeps them. `regex::split(";a;;", ";")` and
        `strings::split(",a,,", ",")` both render `n=4 [<>, <a>, <>, <>]`, and
        the fixture asserts them on the same run.
      * **`limit`** — none, because `strings::split` has none. See the correction
        under Status.
      * **empty pattern** — refused with `77050002`, per Phase 3's citation.
- [x] Point `strings::split`'s page at it. Done, plus `strings::count` -> `regex::count`.

Acceptance: MET. Whitespace-run tokenization works
(`split("the   quick  brown", "\\s+")` -> `[the, quick, brown]`), and the
zero-width case terminates with the documented result.
Commit: folded into the single commit below.

### Phase 5 — validation

- [x] `cargo test --release --no-fail-fast`; `scripts/test-accept.sh`. Results in
      the commit message.
- [x] `scripts/man-run-examples.sh regex --run` 24/24, `strings --run` 85/85,
      `astrings --run` 17/17. `man-census.sh --memory-scope` reports 0
      unclassified hits for all three.
- [x] Confirm byte-identical results on all three platforms. The new members are
      `Body::mfb` — one MFBASIC body compiled by the same front end for every
      target, with no per-arch lowering — and `regen-ncodesum.sh` regenerated all
      five targets of the one affected fixture. Cross-target codegen is proved by
      the gate; cross-platform EXECUTION of these two members has not been run on
      the Linux/Windows boxes and is not claimed.

Acceptance: MET on macOS/aarch64 for execution, and on all five targets for
codegen.
Commit: folded into the single commit below.

## Validation Plan

- Regression tests: per member added. For `split` specifically: a whitespace-run
  case, a zero-width-pattern case asserted to terminate, a leading-match case
  asserting the empty first piece, and a non-ASCII case pinning scalar indices.
- Runtime proof: an in-tree tokenizer from Phase 1 rewritten onto `regex::split`
  and producing identical output.
- Doc sync: the `regex` intro (the parity statement and the `replace`
  exclusion), the new member pages, `strings::split`'s cross-reference.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions — all three SETTLED

- `regex::count` as a member vs. an intro pointer. ~~Recommend the member.~~
  **SETTLED: the member.** `benchmark/mfb` writes `len(regex::findAll(...))`
  three times, which is the discoverability gap made concrete.
- `split`'s empty-piece rule. ~~Recommend keeping leading/trailing empties.~~
  **SETTLED: kept**, and not as a preference — `strings::split` keeps them
  (measured), so keeping them is the mirror, and N matches -> N+1 pieces makes
  the input reconstructible.
- Whether a rewriting `AttributedString` overload for `regex::replace` is ever
  worth doing. **SETTLED: not here.** The omission is documented on the package
  page AND enforced by the type checker (a call fails to build), rather than
  silently dropping the attributes. Still a plan of its own if anyone wants it.

## Summary

Three gaps with three different costs. The `AttributedString` overloads and
`count` are independent and can land immediately; `split` is blocked on
bug-532 and carries all the semantic risk (zero-width patterns and empty
pieces). The most useful part of Phase 1 is the full parity table — the three
gaps here were found by reading, and a member-by-member census is the only way
to know the list is complete.
