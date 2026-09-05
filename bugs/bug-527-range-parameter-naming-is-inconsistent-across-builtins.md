# bug-527: range parameters are spelled five different ways across the built-ins, and `endIndex` means two different things

Last updated: 2026-09-04
Effort: large (3h–1d)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `tests/` — a registry pin asserting the range-parameter vocabulary

There is no language-wide convention for naming the ends of a range. A census of
every registry parameter (`grep -rhoE '"(start|stop|end|endIndex|from|to|first|last|count|length)"' src/codegen/builtins/`)
finds five different spellings for "where the range ends":

| pair | members |
| --- | --- |
| `start` + `count` | `strings::mid`, `collections::mid` |
| `start` + `endIndex` | `astrings::addAttribute`, `astrings::removeAttribute` |
| `start` + `finish` | `datetime::between` |
| `start` + `last` | the internal `astrings::AttrSpan` record |
| `start` alone (a scan origin) | `strings::find`, `regex::find`, `regex::findAll`, `collections::find`, `collections::findIndex` |
| `endIndex` alone (a scan origin) | `collections::findLastIndex` |

Two things make this worse than untidiness.

**1. `endIndex` means two different things.** In `astrings::addAttribute` it is
the inclusive *end of a range*: "the **inclusive** scalar range
`[start, endIndex]` (length `endIndex − start + 1`)". In
`collections::findLastIndex` it is the *origin of a backward scan* — "Zero-based
index at which the backward scan begins" — which is a range *start*, spelled
`endIndex` because the scan runs the other way.

**2. Negative indices are accepted by one sibling and rejected by the other.**
`collections::findIndex`'s `start`: "a negative value is out of range, **not**
an offset from the end." `collections::findLastIndex`'s `endIndex`: "A negative
value is resolved as `len(value) + endIndex`, so `-1` is the last element."
Same package, adjacent members, mirror-image operations, opposite rules — and
`-1` is `findLastIndex`'s *default*.

The proximate cause of the odd spelling is real and documented:
`src/codegen/builtins/astrings/func_add_attribute.rs:5` — "The end-of-range
parameter is `endIndex` (not `end`, a reserved keyword)". `end` is in the
lexer's reserved-word list (`src/lexer.rs:1508`), so the obvious name is
unavailable and each author picked a different escape.

The single correct behavior a fix produces: one documented vocabulary for range
bounds, applied across every built-in, with one rule for negative indices.

References:

- `src/lexer.rs:1508` — `end` is reserved
- `src/codegen/builtins/astrings/func_add_attribute.rs:5,14-24`
- `src/codegen/builtins/collections/func_find_index.rs:170`
- `src/codegen/builtins/collections/func_find_last_index.rs:386`
- `src/codegen/builtins/datetime/func_between.rs:88,95`
- `src/codegen/builtins/strings/func_mid.rs:105,112`

## Failing Reproduction

The defect is a census, not a crash:

```
grep -rn 'name: "start"\|name: "endIndex"\|name: "finish"\|name: "count"\|name: "last"' \
  src/codegen/builtins/ --include='*.rs'
```

- Observed: the table above — five end-of-range spellings, and `endIndex`
  carrying two meanings.

The negative-index divergence, which is the part that produces wrong answers
rather than merely confusion, is reproducible directly:

```
' Rejected: negative start is out of range.
collections::findIndex([1,2,3], isPos, start := -1)

' Accepted: negative endIndex counts from the end; -1 is the DEFAULT.
collections::findLastIndex([1,2,3], isPos, endIndex := -1)
```

Phase 1 owns turning this into a fixture that records what each currently does.

Contrast cases that are correct and set the standard:

- `start` + `count` (`strings::mid`, `collections::mid`) is unambiguous: an
  offset and a length, no inclusivity question at all. It is arguably the right
  answer everywhere and is already the most-used pair.
- `astrings::addAttribute` documents its inclusivity precisely, including the
  consequence — "because it is inclusive, **empty text has no valid range at
  all** — even `0, 0` is out of range". That is the standard of clarity a
  convention should make unnecessary to restate.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | naming census; target-independent |

## Root Cause

No convention was ever written down, and the natural name is unavailable.

`end` is a reserved keyword (`src/lexer.rs:1508`), so a parameter cannot be
called `end` — which means the "obvious" spelling that every other language uses
is off the table, and each author independently invented a replacement.
`astrings` chose `endIndex` and left a comment saying why; `datetime` chose
`finish`; `strings`/`collections` sidestepped it with `count`; the internal
`AttrSpan` record chose `last`.

The `endIndex` overload — range-end in `astrings`, scan-origin in
`collections::findLastIndex` — is a second-order effect: once `endIndex` existed
as "the end-ish parameter", it got reused for a parameter that is semantically a
*start*.

The negative-index divergence is independent of the naming and is the more
serious half: it is a behavioral inconsistency between two members that are
documented as mirrors of each other.

## Goal

- A single written convention for range and index parameters, in
  `.ai/man-content.md` or the spec, covering: an offset+length pair, an
  inclusive bound pair, an exclusive bound pair, and a scan origin.
- Every built-in parameter conforms, or has a recorded exception with a reason.
- One rule for negative indices across all of them.
- A pin that fails when a new parameter uses a spelling outside the convention.

### Non-goals (must NOT change)

- Inclusivity semantics. `astrings`'s `[start, endIndex]` inclusive range is a
  behavior with callers; renaming a parameter must not change what it means.
  If the convention favours exclusive bounds, that is a *separate* decision
  with a separate migration.
- `strings::mid`/`collections::mid`'s `start` + `count`, which are correct.
- Making `end` a legal identifier. Un-reserving a keyword is a language change
  far larger than this problem.
- **Tempting wrong fix, forbidden:** renaming parameters without deciding the
  negative-index rule. The names are cosmetic; the `findIndex`/`findLastIndex`
  divergence is a real behavioral trap, and a rename that leaves it in place
  makes the two members *look* more alike than they behave.

## Blast Radius

Parameter names are part of the public surface — every one is usable as a
named argument (`collections::findLastIndex([5,0,7], isPos, endIndex := -2)` is
in that member's own example), so a rename is a source-compatibility break.

From the census:

- `astrings::addAttribute`, `astrings::removeAttribute` — `start` + `endIndex`,
  inclusive. Renamed by this bug if the convention differs.
- `collections::findLastIndex` — `endIndex` used as a scan origin. **The
  clearest misnomer**; renamed regardless of which convention wins.
- `datetime::between` — `start` + `finish`. Renamed.
- `strings::mid`, `collections::mid` — `start` + `count`. Likely unchanged.
- `strings::find`, `regex::find`, `regex::findAll`, `collections::find`,
  `collections::findIndex` — `start` as scan origin. Likely unchanged.
- `strings::left`, `strings::right`, `collections::take`, `collections::drop`,
  `crypto::randomBytes`, `bits::*` shifts — `count` as a quantity, not a range
  bound. **Unaffected**; the convention must not sweep these up.
- `crypto::pbkdf2`, `crypto::hkdf`, `crypto::shake256` — `length` as an output
  size. **Unaffected**, same reason.
- `process::receive`, `process::poll`, `udp` — `from` as a source selector.
  **Unaffected**.
- `astrings::AttrSpan` (`start` + `last`) — internal (`export: false`), so it
  can be renamed freely; worth doing for consistency.
- Every acceptance fixture, example and man-page example using a named
  argument on a renamed parameter.

## Fix Design

Two independent decisions; the second is the one that matters.

**1. The vocabulary.** Given that `end` is reserved, the candidates are
`endIndex`, `stop`, `finish`, `last`, `endIdx`. **Recommend `endIndex`** — it is
already the most-used of the five, it is explicit that the value is an index
rather than a count, and `astrings` already carries the comment explaining why
it is not `end`. Pair it with `startIndex` **only where a matching bound
exists**; a bare scan origin stays `start`, which reads correctly and is already
uniform across five members.

That gives four documented shapes:

| shape | spelling |
| --- | --- |
| offset + length | `start` + `count` |
| inclusive bounds | `start` + `endIndex`, documented as inclusive |
| scan origin (forward) | `start` |
| scan origin (backward) | `start` — **not** `endIndex` |

Under this, `collections::findLastIndex`'s `endIndex` becomes `start`, which is
what it is: where the scan begins.

**2. The negative-index rule.** This is a behavior decision, not a naming one:

- *Negative counts from the end, everywhere.* Convenient, familiar from other
  languages, and already `findLastIndex`'s behavior — including its default of
  `-1`. Cost: every other index parameter changes behavior, turning what is
  currently an `ErrIndexOutOfRange` into a silent success at a different index.
  That is the dangerous direction.
- *Negative is out of range, everywhere.* Already the rule for `findIndex`,
  `mid`, `find` and the rest — the majority. Cost: `findLastIndex` cannot
  default to `-1` and needs a different default spelling for "the end".

**Recommend negative-is-out-of-range**, because it is already the majority rule
and because the alternative converts existing errors into silent wrong answers.
`findLastIndex` then needs an explicit way to say "from the end" — an omitted
argument meaning the last element is the natural answer, and it already has an
optional parameter.

Rejected: leaving the names and documenting the divergence more loudly. That is
the current state; `findLastIndex`'s page already spends four paragraphs on its
two-step resolution, and the divergence with `findIndex` survives it.

Rejected: renaming everything to `startIndex`/`endIndex` uniformly. It makes
`strings::mid(s, startIndex, count)` worse — `start` is not an index there in
any sense the pair implies — and churns five correct members.

## Phases

### Phase 1 — census + decisions (no behavior change)

- [ ] Complete the parameter census: every registry `Parameter` whose name is a
      range bound, an index, or a count, with its semantics and its
      negative-value rule. `grep` is the start, not the answer — read each `desc`.
- [ ] Write a fixture recording today's behavior for negative arguments to
      `findIndex`, `findLastIndex`, `mid`, `find`, `addAttribute`. It passes; it
      is the guard for Phase 3.
- [ ] Decide the vocabulary and the negative-index rule; write both into
      `.ai/man-content.md`.
- [ ] Count named-argument call sites for every parameter proposed for rename
      (`grep -rn "endIndex :=\|finish :=" tests/ examples/ benchmark/ src/`).

Acceptance: the census is complete with a verdict per parameter; both decisions
are written down; the rename cost is counted.
Commit: —

### Phase 2 — the negative-index rule

- [ ] Apply the decided rule to every divergent member. This is the behavioral
      half and lands first, alone, so its blast radius is not entangled with a
      rename.
- [ ] Give `collections::findLastIndex` a default that does not depend on `-1`.

Acceptance: the Phase 1 fixture is updated to the new rule and passes; every
index parameter answers a negative argument the same way.
Commit: —

### Phase 3 — the renames

- [ ] Rename the parameters that diverge from the vocabulary.
- [ ] Update every named-argument call site from Phase 1.
- [ ] Update the man pages, including the examples that use named arguments.

Acceptance: the vocabulary pin passes; all examples compile and run.
Commit: —

### Phase 4 — pin + validation

- [ ] Add a registry pin: every range/index parameter name is in the documented
      vocabulary, with an explicit exception list.
- [ ] Regenerate goldens; `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh` for every touched package.

Acceptance: full suite green; the pin rejects a new out-of-vocabulary name.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 negative-argument fixture, updated in Phase 2;
  the Phase 4 vocabulary pin.
- Runtime proof: `collections::findIndex` and `collections::findLastIndex`
  answering a negative argument identically.
- Doc sync: `.ai/man-content.md` (the convention), every renamed parameter's
  page, and `src/docs/spec/**` if it documents indexing rules.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- The vocabulary. **Recommend `start` / `endIndex` / `count`** as above.
- The negative-index rule. **Recommend out-of-range everywhere**, on the
  grounds that the alternative turns existing errors into silent wrong indices.
  This is the decision that should be made first and separately.
- Whether to split this document. Phase 2 (behavior) and Phases 3–4 (naming)
  are independent and have very different risk profiles. **Recommend splitting
  Phase 2 into its own bug** if the Phase 1 census shows more than a couple of
  divergent members.

## Summary

Two problems wearing one coat. The naming inconsistency is cosmetic, wide, and
a source-compatibility break to fix. The negative-index divergence between
`findIndex` and `findLastIndex` is narrow, behavioral, and the only part that
produces wrong answers — it is worth landing on its own, first, whatever
happens to the names.
