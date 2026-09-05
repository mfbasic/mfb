# bug-532: `regex` reports where a match starts and nothing else, so a general pattern's match text cannot be extracted

Last updated: 2026-09-05
Effort: x-large (1d–3d)
Severity: HIGH
Class: Correctness

Status: **FIXED** — `regex::findMatch` / `regex::findAllMatches` return the
span, the matched text and the capture groups of every match, through two new
exported value records `regex::MatchInfo` and `regex::Group`. The four original
members keep their signatures, their results and their bodies.
Regression Test: `tests/rt_regex_span.rs`

The `regex` package has four members: `match`, `find`, `findAll`, `replace`.
`find` returns a start index. `findAll` returns a list of start indices. There
is no end index, no matched substring, no capture accessor, and no `Match`
record.

For a fixed-length pattern a caller can reconstruct the span. For a general one
they cannot:

```
regex::findAll("a1b22c333", "\d+")  ->  [1, 3, 6]
```

Those three matches are `"1"`, `"22"` and `"333"` — lengths 1, 2 and 3. Nothing
in the package reports the lengths, and slicing requires them. So the single
most common thing anyone does with a regular expression — *get the text that
matched* — has no supported route.

The workarounds are both bad:

- `regex::replace` with a `$0` template, writing the match into a delimiter-
  joined string, then splitting it. This is extraction through a rewriting
  member, and it breaks whenever the match text can contain the delimiter.
- Re-matching an anchored pattern at every candidate length, per start. The
  spike does this to prove the spans are recoverable at all; it is
  `O(text × maxlen)` extra work to recover information the engine already
  computed and discarded.

Named groups make the gap sharper: `regex::replace` supports `$N` in a
replacement template, so the engine *has* capture positions internally. They are
reachable only by rewriting the string.

The single correct behavior a fix produces: a caller can obtain the span (start
and end) and the matched text of each match, and the contents of its capture
groups, without re-running the engine.

References:

- `mfb man regex` — the four-member function table and "The functions differ
  only in what they report"
- `src/codegen/builtins/regex/func_find.rs`, `func_find_all.rs`, `func_replace.rs`
- `mfb man regex language` — the pattern dialect, including named groups
- Spike: `spikes/api-review/bug-532-regex-span/`
- Related: bug-534 (`split`, `count`, `AttributedString` overloads)

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-532-regex-span
./spikes/api-review/bug-532-regex-span/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
text    = a1b22c333
pattern = \d+
  match starts at scalar 1  -- length unknown
  match starts at scalar 3  -- length unknown
  match starts at scalar 6  -- length unknown

recovering the spans by brute force:
  [1, 2) = "1"
  [3, 5) = "22"
  [6, 9) = "333"

=> the engine computed each end position and then discarded it.
```

  The second block is the spike re-matching `\A\d+\z` against every candidate
  substring. It proves the answers exist and that the package will not give
  them to you.

- Expected: a member returning the spans directly — `[1,2)`, `[3,5)`, `[6,9)` —
  or the matched substrings.

Contrast cases, correct today:

- `regex::replace` with `$0` *does* have the match text; it just writes it into
  a new string instead of returning it. So the engine's internal state is
  sufficient — this is a surface gap, not a capability gap.
- `strings::find` has the same start-only shape and does not need more: for a
  literal needle the length is `len(needle)`, known to the caller. That is
  exactly why the same shape does not transfer to patterns.
- `regex::match` and `regex::findAll` are correct for the questions they answer
  ("is there one", "where do they start").

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | pure software engine; expected identical |

## Root Cause

Not a defect — a surface that was specified around the questions `strings`
answers rather than the questions a regex engine can answer.

The four members mirror `strings`: `contains`→`match`, `find`→`find`,
`count`-ish→`findAll`, `replace`→`replace`. For a literal needle that surface is
complete, because the caller supplies the length. For a pattern it is not,
because the length is an *output*.

The engine necessarily computes each match's end — `findAll` advances past a
match to find the next one, and `replace` splices the matched extent out. Both
therefore know the extent and neither returns it. `mfb man regex`'s own
framing, "The functions differ only in what they report", is precise: the
information is there and the reporting is what is missing.

## Goal

- A member reports each match's **span** — start and end — for `find` and
  `findAll`.
- A member reports the **matched text**, without a second pass.
- **Capture groups** are readable outside a replacement template, including
  named ones.
- The existing four members keep their current signatures and results.

### Non-goals (must NOT change)

- `regex::find`/`findAll`'s current returns. Existing callers depend on the
  `Integer` and `List OF Integer` shapes; the new capability is additive.
- The `-1`-on-absence contract (bug-531 owns that question).
- The pattern dialect, its Unicode pinning, or its portability guarantee.
- The zero-width-match iteration rule ("iteration advances one scalar past an
  empty match so it always terminates"), which any span-returning member must
  preserve exactly or `findAll` on `""` becomes non-terminating.
- **Tempting wrong fix, forbidden:** documenting the `replace`-with-`$0`
  workaround as the supported extraction route. It is not correct in general —
  any delimiter the caller picks can appear in the match text — and blessing it
  turns a gap into a trap.

## Blast Radius

- `src/codegen/builtins/regex/func_find.rs`, `func_find_all.rs` — extended, not
  changed.
- `src/codegen/builtins/regex/func_replace.rs` — the member that already
  resolves `$N`; its capture machinery is what a capture accessor must reuse,
  not reimplement.
- The engine core — `src/codegen/builtins/regex/` internals. Whether match ends
  and capture slots survive to the member boundary today is **the load-bearing
  unknown**, and Phase 1's first job.
- `regex`'s return types — a span or a match needs a record
  (`regex::Match`), which is new type surface in a package whose intro
  currently says "The package defines no new types." That sentence changes.
- `astrings` — bug-534 covers the overload question; a `Match` record would need
  to decide what it means for attributed text.
- Anything in-tree parsing text with `regex` — `grep -rn "regex::" src/
  examples/ benchmark/` in Phase 1. Each is a place currently paying the
  workaround cost, and a validation case.

## Fix Design

The shape question is whether to return a record or parallel lists.

**A — a `regex::Match` record.** `regex::findMatch(value, pattern, [start]) AS
regex::Match` and `regex::findAllMatches(...) AS List OF regex::Match`, with
`Match` carrying `start`, `endIndex`, `text`, and the captures. One value, one
lookup, and it extends naturally when captures arrive. Cost: the package gains
types, and a `List OF Match` for a large input allocates a record per match.

**B — parallel returns.** `regex::findSpan` returning a two-element list, or a
`findAllEnds` companion to `findAllStarts`. No new types, minimal surface. Cost:
the caller re-associates parallel lists by index, which is exactly the class of
bug this package should not be creating, and captures have no home at all.

**Recommend A.** The captures requirement decides it: `$1`, `$2` and named
groups need somewhere to live, and a record is the only shape that holds them.

Captures then need their own decision — a `List OF String` indexed by group
number, a `Map OF String TO String` for named groups, or both. **Recommend a
list plus a name lookup**, since the dialect supports both numbered and named
groups and a caller who wrote `(?<year>\d{4})` should not have to count
parentheses.

The correctness risk concentrates in two places:

1. **The zero-width rule.** A span-returning member must reproduce
   `findAll`'s "advance one scalar past an empty match" exactly, or a pattern
   like `"a*"` produces a different match sequence from the two members —
   which is worse than not having the member.
2. **Scalar vs. byte indices.** The package intro is emphatic: "Every position
   and index a regex function accepts or reports is a zero-based Unicode scalar
   index — never a byte offset." The engine's internal ends may well be byte
   offsets; converting them is where an off-by-one lands mid-scalar.

Rejected: adding an `end` output parameter to `find`. The language has no
out-parameters, and `end` is a reserved word (bug-527).

Rejected: `regex::extract(value, pattern) AS List OF String` as the whole fix.
It answers the common case cheaply, but it discards positions — so it cannot
serve the caller who needs to rewrite around a match, and captures still have
nowhere to go.

## Phases

### Phase 1 — establish what the engine already knows (no behavior change)

- [ ] Land `spikes/api-review/bug-532-regex-span/` (done).
- [ ] Read the engine core and record whether a match's end and its capture
      slots are available at the member boundary, or are discarded inside. This
      determines whether this is a surface change or an engine change, and
      therefore the true effort.
- [ ] Write the desired-behavior fixture: spans `[1,2) [3,5) [6,9)` for
      `findAll("a1b22c333", "\d+")`, plus a named-capture case. Confirm it does
      not compile today.
- [ ] `grep -rn "regex::" src/ examples/ benchmark/` — list in-tree callers
      currently working around the gap.

Acceptance: the engine's internal availability is established by reading code,
not assumed; the fixture fails; the caller list is written down.
Commit: —

### Phase 2 — promote to a plan

- [ ] This adds a public record type, changes the package intro's "defines no
      new types", and may require engine changes. Write `plan-NN` from Phase 1
      and execute there, with the zero-width rule and the scalar-index
      conversion called out as the two correctness risks.

Acceptance: a plan exists with the `Match` shape and the capture representation
decided.
Commit: —

## Validation Plan

- Regression tests: spans for a variable-length pattern; a zero-width pattern
  (`"a*"`) asserted to produce the *same* match sequence from `findAll` and the
  new member; a named-capture extraction; a match containing non-ASCII text, to
  pin the scalar-index conversion.
- Runtime proof: `spikes/api-review/bug-532-regex-span/` with the brute-force
  block deleted and the direct member in its place, producing identical spans.
- Doc sync: the `regex` package intro (it will define types), the new member
  pages, `mfb man regex language` if capture naming needs restating.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`, on all
  three platforms — the dialect's portability guarantee is byte-for-byte.

## Open Decisions

- `Match` record vs. parallel lists. **Recommend the record**, decided by the
  captures requirement.
- Capture representation: numbered list, named map, or both. **Recommend
  both**, since the dialect supports both.
- Whether `findAllMatches` should be lazy. The language has no iterator
  protocol in this position, so **recommend eager** and note the allocation
  cost in the page.

## Summary

The highest-value item in the regex set: not a wrong answer, but a missing one
that every non-trivial use hits immediately. The real unknown is whether the
engine surrenders match ends and capture slots at the member boundary or throws
them away internally — that single fact separates a surface change from an
engine change, and Phase 1 exists to answer it before anyone estimates the work.

---

# Resolution (2026-09-05)

## Phase 1's answer: the engine surrenders everything at the member boundary

The load-bearing unknown was "whether match ends and capture slots survive to the
member boundary today". They do, completely, and this was therefore a **surface**
change, not an engine change.

`__regex_run` returns `__regex_Result[ok, pos, caps]`. `pos` is the match end.
`caps` is the flat `List OF Integer` of `2 * (groups + 1)` scalar positions that
`__regex_initCaps` seeds with `-1`: `__regex_tryAt` writes slot `0` with the start
before the match runs, the outer `ContCap[0, ContDone]` writes slot `1` with the
end when the match succeeds, and each `Group` node writes slots `2k` / `2k+1`.
`__regex_matchResults` already hands `findAll` and `replace` a
`List OF __regex_Result` — the full sequence, ends and captures attached.
`__regex_findAll` was reading `caps[0]` out of each one and dropping the rest;
`__regex_replace` was reading `pos` and expanding `$N` out of the same slots.

So no engine code changed. Nothing new is matched, no search is re-run, and the
per-call budget bug-510 armed in `__regex_makeCtx` is untouched.

## What landed

Two exported value records, and two members that project the results the walk
already produced:

| Type | Fields |
|---|---|
| `regex::Group` | `start`, `endIndex`, `text` |
| `regex::MatchInfo` | `start`, `endIndex`, `text`, `groups AS List OF Group`, `names AS Map OF String TO Integer` |

| Member | Returns |
|---|---|
| `regex::findMatch(value, pattern[, start=0])` | `regex::MatchInfo` |
| `regex::findAllMatches(value, pattern[, start=0])` | `List OF regex::MatchInfo` |

- `src/codegen/builtins/regex/helper_make_match.rs` — `__regex_makeMatch`, the
  projection. Group `k`'s span is slots `2k`/`2k+1`, sliced with the **same**
  `strings::mid` call `__regex_lookupNum` uses to expand `$k`, so `groups[k].text`
  and `$k` are the same text by construction rather than by agreement. An unset
  slot stays `-1`, which is how a non-participating group is reported.
- `src/codegen/builtins/regex/helper_no_match.rs` — `__regex_noMatch`, the absence
  value: `start`/`endIndex` `-1`, empty `text`, empty `groups` and `names`.
- `src/codegen/builtins/regex/func_find_match.rs`, `func_find_all_matches.rs` —
  the members. `findMatch` calls `__regex_searchFrom`, the identical call
  `__regex_find` makes; `findAllMatches` iterates `__regex_matchResults`, the
  identical walk `__regex_findAll` and `__regex_replace` iterate.

**The two correctness risks the Fix Design named are answered structurally, not
by testing.** The zero-width rule is not reimplemented — `findAllMatches` consumes
the same `__regex_matchResults` helper, so "advance one scalar past an empty
match" is one implementation shared by three members and cannot drift. And there
is no scalar-vs-byte conversion anywhere in the new code: capture slots are
already scalar positions, and `strings::mid` is scalar-indexed.

### The `MatchInfo` spelling

`MATCH` is an MFBASIC keyword (`MATCH … END MATCH`, and keywords are
case-insensitive), so `regex::Match` cannot be a type name — it does not parse in
type position, and in a `LET x AS regex::Match` it resolves to the *function*
`regex::match`. The record is spelled `MatchInfo`, and the package page says why
so a reader does not spend time on it. The member names are unaffected.

## Why this is not a language-surface change

The constraint bars altering MFBASIC syntax or a *correct* program's observable
semantics. This adds surface; it changes none.

- **The two members.** Before this change `regex::findMatch` and
  `regex::findAllMatches` were `SYMBOL_UNKNOWN_IDENTIFIER` — "Built-in package
  `regex` does not export …". No program containing either token compiled, so no
  *correct* program could contain one. Same argument bug-521 made for its
  arity-dispatched overload.
- **The two types.** `regex::MatchInfo` and `regex::Group` were equally
  unresolvable. Built-in package types are qualified (`regex.Group`), so a program
  that already declares its own `TYPE Group` and imports `regex` keeps its own
  type and its own meaning — pinned by
  `a_program_with_its_own_group_record_still_compiles_alongside_regex`, which
  fails to compile on pre-fix main and passes after.
- **No existing member changed.** `find`, `findAll`, `match` and `replace` keep
  their signatures, their bodies byte-for-byte, and their results — the
  `-1`/`[]`/unchanged-`String` absence contract included. bug-531 still owns the
  raise-vs-sentinel question; `findMatch` follows the package's existing sentinel
  rather than pre-empting it.
- **The shape is the package's own.** `MatchInfo` is a flat exported value record
  holding a `List OF` a second exported record and a `Map` — the same shape
  `datetime`, `net` and `canvas` already export, not a new kind of thing.

## bug-534: what this takes and what it does not

bug-534 (`split`, `count`, `AttributedString` overloads) is **not** subsumed, and
deliberately so:

- **`split` is now unblocked and not taken.** bug-534's own analysis is that
  `split` "cannot be written with the current surface" because it needs each
  match's extent. It can now: `findAllMatches` gives the extents. But `split` is a
  different member answering a different question, and bug-534 Stage 3 lists three
  semantic decisions that are its to make and to document — the zero-width-pattern
  rule, whether leading/trailing empty pieces are kept, and what `limit` does.
  Taking `split` here would be deciding those in a bug that never analysed them.
  bug-534's Phase 4 prerequisite is satisfied; the work is its.
- **`count` and the `AttributedString` overloads never depended on this.** They
  are independent of spans by bug-534's own staging (Stages 1 and 2, explicitly
  "independent of everything else"), and nothing here makes them cheaper.

## The in-tree caller census (Phase 1)

`grep -rn "regex::" src/ examples/ benchmark/ packages/` finds no caller paying
the workaround cost, so there is no in-tree migration to make and none was made:

- `examples/browser/display/src/lib.mfb` (4 sites) — all `regex::replace`, a
  rewriting use that was already the right member.
- `benchmark/mfb/src/regexbench.mfb`, `benchmark/mfb/src/main.mfb` — `findAll`
  used for its **count** and `replace` for rewriting; neither wants the text.
- `src/**` uses `regex::` only inside the package's own generated Unicode
  companion.

The absence is itself the finding the bug predicted: the gap was severe enough
that nobody wrote the workaround in-tree — they wrote something else instead.

## Semantics-preservation evidence

1. **The RED half.** All three fixture programs in `tests/rt_regex_span.rs` fail
   to compile on pre-fix main (`a01896d7d`) with
   `SYMBOL_UNKNOWN_IDENTIFIER … does not export regex.findMatch` /
   `regex.findAllMatches` / `regex.MatchInfo` / `regex.Group`; all three pass
   after.
2. **The positive half, and it is the load-bearing half.**
   `the_new_members_agree_with_the_index_only_members_across_the_matcher_corpus`
   reuses bug-510's 85-case corpus verbatim and requires, per case:
   `findAll`'s indices equal the `start` of each `MatchInfo` one-for-one and in
   the same count; `findMatch(...).start` equals `find(...)` from `0` and from
   `1`; `groups[0]` restates the whole match; and — the check that cannot pass by
   accident — `replace`'s output can be **rebuilt** from the reported spans and
   groups. The reconstruction reads the subject through
   `MatchInfo.start`/`endIndex`/`Group.text` while `replace` reads it through the
   `$N` expander, so a wrong end position shows up as a wrong gap and a wrong
   group as wrong text. All 85 agree, including the eight that raise (the same
   codes at the same cases).
3. **`rt_regex_bounds.rs` is untouched**, so the four original members are still
   pinned byte-for-byte by their own corpus.
4. **Golden containment, measured against a clean baseline.** A detached
   worktree at the branch point (`a01896d7d`) gated at
   `1375 tests, 1540 build(s), 1910 golden(s) checked, 0 diff(s)` — so every
   diff on the branch is this change's. The branch then gated at exactly
   **8 diffs**, all three of them regex-importing fixtures and nothing else:

   | Fixture | Goldens that moved |
   |---|---|
   | `byte-identity/regex/regex_codegen_cover_rt` | `.ir` + all five `.ncodesum` |
   | `rt-behavior/regex/regex-from-string-rt` | `.ir` |
   | `rt-behavior/regex/regex-posix-classes-rt` | `.ir` |

   1902 of the 1910 goldens are byte-identical. **That containment is the
   semantics proof.** Within the eight, the delta is exactly (a) the two new type
   declarations, (b) the two new emitted functions and two helpers, and (c) the
   `ErrorLoc` line constants baked into the package's *existing*
   `FAIL error(...)` sites, shifted by the thirty source lines the two record
   declarations add above them. No instruction in any existing regex function
   changed. `bash scripts/regen-ncodesum.sh` refreshed 143 goldens and **only
   the five regex sums differed**; the re-run gate is `0 diff(s)`.
5. **Zero `.run` goldens moved, and zero `.ast`/`build.log` goldens moved** — no
   fixture's runtime output or diagnostics changed. `scripts/test-accept.sh`
   passed with **1397 test(s) ran**, the same count bug-514 and bug-539 recorded,
   so no fixture was lost or gained.

## Gate numbers

- `cargo test --no-fail-fast -- --skip artifact_gate_all` → **cargo's own exit 0**,
  135 result blocks, 0 `failures:` blocks. The lib binary alone is 3,820 passing.
  Includes `rt_regex_bounds::matching_semantics_are_unchanged` (the four original
  members, byte-for-byte) and the three new `rt_regex_span` cases.
- `scripts/artifact-gate.sh target/release/mfb all` → **0 diff(s)** over 1,910
  goldens, after regeneration; baseline on the branch point was also 0.
- `scripts/test-accept.sh` → **exit 0, 1397 test(s) ran**.
- `scripts/man-run-examples.sh regex --run` → 16 examples, 16 built, 16 ran, 0
  failed.
- `scripts/man-census.sh --memory-scope regex` / `--scope regex` → 0 and 0.

## Doc sync

- `mfb man regex` — the intro now says the package defines two types (it used to
  say it defined none), documents the two members, and states that the
  index-only and extracting members find the same matches.
- `mfb man regex findMatch`, `mfb man regex findAllMatches`,
  `mfb man regex types` — new pages, rendered and read.
- `scripts/man-run-examples.sh regex --run`: 16 examples, 16 built, 16 ran, 0
  failed.
- `scripts/man-census.sh regex`: 6 pages, intro/desc/example on all 6, 17/17
  parameter descriptions, 8/8 type descriptions, **0** banned memory-vocabulary
  hits and **0** internals-vocabulary hits.
- `src/docs/spec/stdlib/01_regex.md` — the Public Surface table gains both calls
  and a types table, and a new **Match Projection** section records that
  `findAll`, `findAllMatches` and `replace` share one walk. Two stale statements
  bug-510 left behind were corrected while there: the engine is no longer
  "continuation-passing", and `__regex_Ctx` no longer holds the subject twice.
- `spikes/api-review/bug-532-regex-span/` — the brute-force block is gone and the
  direct members are in its place, producing the same spans
  (`[1, 2) [3, 5) [6, 9)`) plus named-capture extraction and a rewrite around a
  match.

## Open Decisions — resolved

- **Record vs. parallel lists** → the record, as recommended. Captures decided it.
- **Capture representation** → both, as recommended: `groups` is indexed by group
  number (`groups[0]` is the whole match, mirroring `$0`), and `names` maps a
  named group's name to its number, so a caller who wrote `(?<year>\d{4})` never
  counts parentheses. Two shapes, one source of truth — the name resolves to an
  index into the same list, so a name and a number cannot disagree.
- **Lazy vs. eager `findAllMatches`** → eager, as recommended; the page states
  that the whole list is built before it returns and points at `findAll` when only
  the positions are wanted and `findMatch` when only the first match is.

## Noted, not fixed here

Every public member of an imported builtin package is emitted whether or not the
program calls it — a program that calls only `regex::match` still carries
`__regex_replace`, and now also carries the two new members. That is pre-existing
behaviour of builtin package emission, unchanged by this fix and not specific to
`regex`; it is the reason two new members move all five `.ncodesum` targets of a
fixture that never calls them.
