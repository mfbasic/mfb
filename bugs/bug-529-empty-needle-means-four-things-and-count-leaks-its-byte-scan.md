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

**Decided (2026-09-04): the empty needle is PRESENT at position 0 for query
members; counting and rewriting members refuse it.**

| member | empty needle | changes? |
| --- | --- | --- |
| `strings::contains(v, "")` | `TRUE` | no — already |
| `strings::find(v, "")` | `0` | no — already |
| `strings::count(v, "")` | raises `ErrInvalidArgument` | no — already |
| `strings::replace(v, "", r)` | raises `ErrInvalidArgument` | **yes** — bug-533 |

The rule, stated once:

> An empty needle occurs at every position, beginning at 0. A member that
> **answers a question about** an occurrence reports it; a member that
> **counts or rewrites** occurrences refuses, because "every position" has no
> useful count and rewriting at every position destroys the input.

- The rule is written once, in the `strings` package intro, and referenced from
  each member rather than restated.
- Every needle-taking member in the package conforms, or has a recorded
  exception with a reason.
- `mfb man strings count` states what it returns, not how it walks the buffer.
- A test asserts the whole family against the rule.

**Scope consequence.** Three of the four members named in this bug's title
already behave as the rule requires — the observed inconsistency was real, but
the fix is mostly *writing the rule down* rather than changing behavior. The
one behavior change (`strings::replace`) is owned by bug-533. What keeps this
bug from being purely documentation is the census: `split`, `startsWith`,
`endsWith` and the `trim` family have never been measured with an empty
argument, and any that disagree are fixed here.

### Non-goals (must NOT change)

- `strings::contains` and `strings::find`'s current empty-needle answers, which
  the decision ratifies.
- The non-overlapping rule for non-empty needles, or any result for a non-empty
  needle. This bug is scoped to the empty case.
- The scalar-index return of `strings::find`.
- `regex`'s zero-width rule for its **query** members, which is the same rule
  arriving from the other direction and is correct.
- The *performance* of the byte scan. Removing the byte cursor from the prose
  does not mean removing it from the code.
- **Tempting wrong fix, forbidden:** aligning the family by making `contains`
  and `find` reject an empty needle too. `find`'s `ErrNotFound`-on-absence
  contract means rejection would raise two different errors for two flavours of
  "no match", and `contains("x", "")` returning `TRUE` is the mathematically
  standard answer. It is also a gratuitous break: those two members are already
  correct under the decided rule.
- **Also forbidden:** changing `strings::replace` in *this* bug. bug-533 owns
  that change and its caller sweep; doing it here splits the migration across
  two commits.

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

The rule is **present-at-every-position for queries, refuse for counters and
rewriters**. Three of the four members already implement it, so the work is
mostly writing it down and proving the rest of the package agrees.

**Why this beats the uniform alternatives.** Three uniform rules were on the
table — matches everywhere, never matches, reject everywhere — and each is
wrong somewhere:

- *Matches everywhere*, applied uniformly, makes `replace(s, "", x)` a
  whole-string rewrite. That is the single most destructive answer available
  for a needle that was empty by accident.
- *Never matches*, applied uniformly, forces `contains(s, "") == FALSE` and
  makes `find` raise, both of which are gratuitous breaks of members that are
  already right, to defend against a hazard that only exists on the rewrite
  side.
- *Reject everywhere* forces a TRAP onto `contains` and `find` callers who
  currently get a correct, standard answer.

Splitting by what the member *does* takes the good half of each: queries keep
the mathematically standard answer, and the accident is stopped at exactly the
two members where it could do damage. It also explains `strings::count`'s
existing rejection as the rule rather than as the odd one out, and it agrees
with `regex`'s zero-width matching for free — `regex::find(v, "")` and
`strings::find(v, "")` both returning `0` becomes one rule instead of a
coincidence.

The classification is the part that needs care, because "query" and "rewriter"
are not always obvious. `split` is the case to think about: it neither answers
a question nor rewrites in place, and an empty separator yields either one
piece or `n` pieces depending on an arbitrary choice. Phase 1 classifies it.

Separately and independently: rewrite `func_count.rs`'s `DESC` to specify the
result — non-overlapping occurrences, leftmost-first, exact byte comparison with
no normalization or case folding — and drop the cursor narration. Keep the
sentence explaining that a match can never land mid-scalar; that *is* a
contract, and a useful one.

Rejected: keeping the divergence and documenting it on all four pages. That is
the current state on one page, and it has produced four pages that each describe
a different rule.

Rejected: applying the rule by member *name* rather than by what it does. A
name-keyed rule breaks the moment a member is added, and this package already
has the sibling hazard where two adjacent members answer the same question
oppositely (bug-527's `findIndex`/`findLastIndex`).

## Phases

### Phase 1 — census + classification (no behavior change)

- [ ] Land `spikes/api-review/bug-529-empty-needle/` (done).
- [ ] Enumerate every `strings` member taking a needle/separator/pattern and
      record its current empty-argument behavior, **measured**. Extend the
      spike; do not read it off the pages. `split`, `startsWith`, `endsWith`
      and the `trim` family are the unmeasured ones.
- [ ] Do the same for the `astrings` overloads and for
      `collections::find`/`contains`.
- [ ] Pin the two unmeasured edges of the decided rule: `strings::find("", "")`
      and `strings::contains("", "")`. The rule implies `0` and `TRUE`; confirm
      rather than assume.
- [ ] Classify each member as **query** or **counter/rewriter**, and write the
      verdict into this file. `split` needs an argued answer, not a guess.

Acceptance: a measured table of every member's current empty-argument answer;
every member classified; the two edge cases pinned.
Commit: —

### Phase 2 — write the rule down

- [ ] State the rule once in the `strings` package intro, and reference it from
      each member rather than restating it.
- [ ] Bring into line any member the Phase 1 census found disagreeing —
      `String` and `AttributedString` alike. `contains`, `find` and `count`
      already conform and must not be touched.
- [ ] Do **not** change `strings::replace` here; bug-533 owns that change and
      its caller sweep.

Acceptance: the rule is stated in one place; every member either conforms or
carries a recorded exception with a reason.
Commit: —

### Phase 3 — the `count` prose + validation

- [ ] Rewrite `func_count.rs`'s `DESC` to specify the result, keeping the
      "never lands mid-scalar" guarantee and dropping the byte cursor.
- [ ] Add the family-wide pin: every needle-taking member asserted against the
      rule, by member name rather than by index (global test state).
- [ ] Regenerate goldens; `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh strings --run` and `astrings --run`.
- [ ] Confirm on Linux and Windows.

Acceptance: full suite green. **Golden deltas should be empty or near-empty** —
this phase changes prose and adds a pin, and three of the four named members
were already correct. A large golden diff here means something was changed that
should not have been.
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

**Updated 2026-09-04 — bug-533 was decided in a way that reshapes the rule
here, and this bug is now its hard prerequisite.**

bug-533 settled that `strings::replace` and `regex::replace` both **reject** an
empty needle/pattern with `ErrInvalidArgument`, matching what `strings::count`
already does. That is not a fourth independent answer — it is rule D, and it
splits the family by what the member does rather than treating all four alike:

> **A query member answers for an empty needle; a member that counts or
> rewrites refuses it.**

Under that framing:

| member | empty needle |
| --- | --- |
| `contains` | answers — TRUE or FALSE per the rule chosen below |
| `find` | answers — `0` or `ErrNotFound` per the rule chosen below |
| `count` | **refuses** (already does) |
| `replace` | **refuses** (bug-533) |

This is more defensible than any of A/B/C applied uniformly: it explains
`count`'s existing rejection as the precedent rather than the exception, and it
keeps the mathematically-standard answers available on the members where an
empty needle has one and cannot do damage.

**The query half is decided: the empty needle is PRESENT.** `contains` returns
TRUE and `find` returns `0` — which is what both already do, so the decision
ratifies existing behavior rather than changing it. I had recommended the
opposite (absent) on the grounds that an unintentionally-empty needle should
never report a match; that argument is answered better by the refusal half,
which stops the empty needle before it can *do* anything, while leaving the
query members their mathematically standard answers.

It also makes the rule agree with `regex` for free: a regex engine's zero-width
match is present at every position, so `regex::find(v, "")` returning `0` and
`strings::find(v, "")` returning `0` are now the same rule rather than a
coincidence.

Still open:

- **Which members are queries and which are rewriters.** Phase 1's census must
  classify every needle-taking member. `split` is the interesting case: it
  neither answers a question nor rewrites in place, an empty separator has no
  useful meaning, and it probably belongs with the refusers. `startsWith`,
  `endsWith` and the `trim` family are queries by shape but have never been
  measured.
- **Two unmeasured edges of the decided rule**, both to be pinned in Phase 1:
  `strings::find("", "")` — is position 0 valid in an empty string? — and
  `strings::contains("", "")`. The rule implies `0` and `TRUE`; neither has
  been run.
- Whether `regex`'s query members need any change. **Recommend not** — they
  already implement the decided rule via zero-width matching.

**Sequencing: this bug lands before bug-533**, which quotes the rule rather
than inventing one, and which owns the single behavior change the rule implies.

## Summary

With the rule decided, this bug shrank. Three of the four members it names —
`contains`, `find`, `count` — already do what the rule requires, and the fourth
(`replace`) is bug-533's to change. So the deliverable here is the rule itself,
the `count` prose rewrite, and a pin.

The risk is entirely in the census, and it is a real one: a rule stated in the
package intro while `split`, `startsWith`, `endsWith` and the `astrings`
overloads keep their own unmeasured answers would leave the package exactly as
inconsistent as it is today, with a paragraph asserting otherwise. Phase 1 is
the bug. Phase 2 is bookkeeping.

Two things to hold onto while doing it: the classification of `split` needs an
argument rather than a guess, and a near-empty golden diff in Phase 3 is the
expected result — a large one means something was changed that should not have
been.
