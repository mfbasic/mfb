# bug-532: `regex` reports where a match starts and nothing else, so a general pattern's match text cannot be extracted

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: `tests/` — new `rt_regex_span` fixture (Phase 1)

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
