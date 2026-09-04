# bug-529: the empty needle gets four different answers from four `strings` members, and `count` leaks its byte scan into the contract

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `tests/` — new `rt_strings_empty_needle` fixture (Phase 1)

Four `strings` members take a needle and run the same byte scan. They disagree
about what an empty needle means:

| call | result |
| --- | --- |
| `strings::contains("hi", "")` | `TRUE` |
| `strings::find("hi", "")` | `0` |
| `strings::count("hi", "")` | raises `ErrInvalidArgument` (77050002) |
| `strings::replace("hi", "", "x")` | `"hi"` — a no-op |

Read together: the empty needle *is present*, *occurs at index 0*, *has no
well-defined occurrence count*, and *never matches*. The first two and the
fourth cannot all be true of the same scan.

`mfb man strings count` names the split but does not resolve it:

> The empty `needle` has no well-defined occurrence count and is rejected with
> `ErrInvalidArgument` — note that this differs from `strings::contains` and
> `strings::find`, which both accept an empty needle.

The premise is arguable — if the empty needle occurs at position 0 for `find`,
it has an occurrence count, and the standard answer (`len + 1`) is the one
`regex::replace` already implements for the zero-width case (bug-533). But the
inconsistency matters more than the choice: an empty needle usually arrives at
run time from a config value, a form field or a CLI flag, and the same value
routed to two of these members produces a raise from one and a silent no-op
from the other.

The same page also states its scan as its contract:

> The scan starts at the first byte of `value` and compares the bytes of
> `needle` at the current offset. On a match the count is incremented and the
> cursor advances past the whole matched needle; **on a mismatch the cursor
> advances by a single byte.**

That is a description of the implementation, in a package whose stated model is
Unicode scalars — `strings::find` returns a scalar index, and the package's own
prose is careful about the distinction. The behavior is safe (UTF-8 is
self-synchronizing, so a match cannot land mid-scalar, and the page says so),
but the byte cursor is not part of the contract and should not be in it.

The single correct behavior a fix produces: one documented rule for the empty
needle across all four members, and a `count` page that specifies a result
rather than an algorithm.

References:

- `src/codegen/builtins/strings/func_count.rs:13-33`
- `src/codegen/builtins/strings/func_contains.rs`, `func_find.rs`, `func_replace.rs`
- Spike: `spikes/api-review/bug-529-empty-needle/`
- Related: bug-533 (`regex::replace` and `strings::replace` are opposites on the
  same input)

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-529-empty-needle
./spikes/api-review/bug-529-empty-needle/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
strings::contains("hi", "") = TRUE
strings::find("hi", "")     -> index=0
strings::count("hi", "")    -> RAISED code=77050002
strings::replace("hi", "", "x") = "hi"
```

- Expected: one rule. Either all four treat the empty needle as matching at
  every position (`contains` TRUE, `find` 0, `count` 3, `replace` `"xhxix"`), or
  all four reject it, or all four treat it as never matching. Any of the three
  is defensible; the current mixture is not.

Contrast case that is correct today: the non-overlapping rule for a *non-empty*
self-similar needle is consistent and well-documented — `count("aaa", "aa")` is
1, `count("aaa", "a")` is 3, and `replace` uses the same advance. That part of
the model is coherent; only the empty case is not.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | pure software scan; expected identical. Confirm in Phase 3 |

## Root Cause

Two independent defects.

**1. No shared rule.** The four members were specified separately. `contains`
and `find` fall out of a scan that trivially succeeds at offset 0 with a
zero-length needle. `replace` explicitly guards it — "If `old` is the empty
string, nothing can match and a copy of `value` is returned"
(`func_replace.rs:31`) — choosing *never matches*. `count` explicitly guards it
the other way, choosing *reject*. So two members have deliberate, opposite
special cases and two have none.

**2. The `count` page specifies an algorithm.** `func_count.rs`'s `DESC`
describes the cursor's byte-level advance because that is how the member is
implemented. Nothing else in the package does this. It is not wrong — the page
also explains why the byte scan is safe for UTF-8 — but it commits the member to
an implementation and invites the reader to reason in bytes about a package
that otherwise reasons in scalars.

## Goal

- One documented rule for the empty needle, applied by `contains`, `find`,
  `count` and `replace` alike.
- `mfb man strings count` states what it returns, not how it walks the buffer.
- A test asserts the four members agree.

### Non-goals (must NOT change)

- The non-overlapping rule for non-empty needles, or any result for a non-empty
  needle. This bug is scoped to the empty case.
- The scalar-index return of `strings::find`.
- `regex`'s zero-width rule, which is correct for a regex engine and is a
  separate model (bug-533 covers the cross-package pairing).
- The *performance* of the byte scan. Removing the byte cursor from the prose
  does not mean removing it from the code.
- **Tempting wrong fix, forbidden:** aligning the four by making `contains` and
  `find` reject an empty needle too. `find`'s `ErrNotFound`-on-absence contract
  means rejection would raise two different errors for two flavours of "no
  match", and `contains("x", "")` returning `TRUE` is the mathematically
  standard answer. Rejection is the weakest of the three candidate rules.

## Blast Radius

`grep -rn "empty" src/codegen/builtins/strings/func_*.rs` in Phase 1. Known:

- `func_contains.rs`, `func_find.rs` — no explicit guard; the scan's natural
  answer. Fixed by this bug if the rule changes.
- `func_count.rs` — explicit rejection; fixed, and its prose rewritten.
- `func_replace.rs` — explicit no-op; fixed if the rule changes.
- **Every other `strings` member taking a needle** — `startsWith`, `endsWith`,
  `split`, `indexOf`-alikes, `trim`-family. Phase 1 must enumerate them; each
  has an empty-argument answer that is currently unstated, and a rule that
  covers only four members is not a rule.
- `astrings` overloads of the same members — each must give the same answer as
  its `String` counterpart.
- `collections::find`/`contains` — the list analogues. An empty *list* needle
  is the same question one type up; check whether they agree with whatever is
  decided.
- Acceptance goldens containing empty-needle results — enumerated once the rule
  is chosen.

## Fix Design

Choose one rule and apply it. The three candidates:

- **A — matches at every position.** `contains` TRUE, `find` 0, `count`
  `len(value) + 1`, `replace` interleaves. Mathematically standard, matches
  `regex`'s zero-width behavior, and makes the two packages agree (which
  bug-533 would like). Cost: `replace`'s behavior changes, and
  `replace(s, "", x)` becoming a whole-string rewrite is a sharp edge for a
  runtime-supplied needle.
- **B — never matches.** `contains` FALSE, `find` raises `ErrNotFound`, `count`
  0, `replace` unchanged. Safest for a runtime-supplied empty needle — every
  member becomes a no-op rather than a surprise. Cost: `contains("x", "")`
  returning FALSE is unusual, and `find` raising is a behavior change.
- **C — reject everywhere.** All four raise `ErrInvalidArgument`. Most explicit,
  and makes the "empty needle arrived from a config file" case loud instead of
  silent. Cost: the largest behavior change, and it forces a TRAP onto callers
  who currently get a sensible answer.

**Recommend B.** The population at risk is a needle that is empty
*unintentionally*, and B makes every member a no-op for it — the least
destructive outcome. A also has a real argument (cross-package consistency with
`regex`), but it converts an accidental empty needle into a whole-string
rewrite, which is the worst outcome of the three.

Separately and independently: rewrite `func_count.rs`'s `DESC` to specify the
result — non-overlapping occurrences, leftmost-first, exact byte comparison with
no normalization or case folding — and drop the cursor narration. Keep the
sentence explaining that a match can never land mid-scalar; that *is* a
contract, and a useful one.

Rejected: keeping the divergence and documenting it on all four pages. That is
the current state on one page, and it has produced four pages that each describe
a different rule.

## Phases

### Phase 1 — census + decision (no behavior change)

- [ ] Land `spikes/api-review/bug-529-empty-needle/` (done).
- [ ] Enumerate every `strings` member taking a needle/separator/pattern and
      record its current empty-argument behavior, measured. Extend the spike;
      do not read it off the pages.
- [ ] Do the same for the `astrings` overloads and for `collections::find`/`contains`.
- [ ] Choose the rule and write it into this file with its rationale.

Acceptance: a measured table of every member's current empty-argument answer;
the rule is chosen.
Commit: —

### Phase 2 — the rule

- [ ] Apply it to every member from Phase 1, `String` and `AttributedString`.
- [ ] State the rule once, in the package intro, and reference it from each
      member rather than restating it.

Acceptance: all members agree; the spike prints one consistent answer.
Commit: —

### Phase 3 — the `count` prose + validation

- [ ] Rewrite `func_count.rs`'s `DESC` to specify the result, keeping the
      "never lands mid-scalar" guarantee and dropping the byte cursor.
- [ ] Regenerate goldens; `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh strings --run` and `astrings --run`.
- [ ] Confirm on Linux and Windows.

Acceptance: full suite green; golden deltas are only empty-needle results.
Commit: —

## Validation Plan

- Regression test: one fixture calling every needle-taking member with an empty
  needle and asserting the single rule.
- Runtime proof: `spikes/api-review/bug-529-empty-needle/` printing four
  consistent answers.
- Doc sync: the `strings` package intro (the rule), `func_count.rs` (prose
  rewrite), and every member whose page states an empty-argument behavior.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Rule A, B or C. **Recommend B (never matches)**, on the grounds that the
  realistic failure is an unintentionally-empty needle and B makes that a no-op
  everywhere. Revisit if Phase 1 finds in-tree callers depending on
  `contains(s, "") == TRUE`.
- Whether `regex` should be brought in line. **Recommend not** — a regex
  engine's zero-width match is a different and correct model; bug-533 covers
  documenting the pairing rather than merging it.

## Summary

The `count` prose rewrite is free. The empty-needle rule is the real work, and
its risk is entirely in the census: a rule applied to the four members named
here, while `split`, `startsWith` and the `astrings` overloads keep their own
unstated answers, would leave the package exactly as inconsistent as it is now
with more text asserting otherwise.
