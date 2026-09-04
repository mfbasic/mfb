# bug-533: `strings::replace` and `regex::replace` do exactly opposite things with an empty needle

Last updated: 2026-09-04
Effort: small (<1h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: `tests/` — new `rt_replace_empty_needle_parity` fixture (Phase 1)

Two members with the same name, the same shape and the same argument order give
opposite results for the same input:

```
strings::replace("abc", "", "-")  ->  "abc"       ' empty needle never matches
regex::replace("abc", "", "-")    ->  "-a-b-c-"   ' zero-width match everywhere
```

Both are documented, and both are right for their own model.
`src/codegen/builtins/strings/func_replace.rs:31` — "If `old` is the empty
string, nothing can match and a copy of `value` is returned."
`src/codegen/builtins/regex/func_replace.rs:33` — "each scalar and once at the
end: `regex::replace("abc", "", "-")` is `"-a-b-c-"`." A literal empty needle
has no occurrence; an empty *pattern* has a zero-width match at every position,
which is what every regex engine does.

The hazard is that the needle is usually not a literal. It arrives at run time
from a config file, a form field, a CLI flag, or a `--replace` argument — and an
empty one is a normal accident. Routed to `strings::replace` it is a harmless
no-op; routed to `regex::replace` it rewrites the entire string. Same call
shape, same empty input, opposite blast radius, and nothing at the boundary
says so.

The single correct behavior a fix produces: a caller cannot hit the
whole-string rewrite by accident — either the pages make the pairing
unmissable at the point of use, or `regex::replace` refuses an empty pattern.

References:

- `src/codegen/builtins/strings/func_replace.rs:31,105`
- `src/codegen/builtins/regex/func_replace.rs:33`
- `mfb man regex` — "A zero-length match is valid; iteration advances one
  scalar past an empty match so it always terminates."
- Spike: `spikes/api-review/bug-533-empty-pattern-replace/`
- Related: bug-529 (the empty needle means four different things inside
  `strings` alone), bug-531 (the same `strings`/`regex` seam for absence)

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-533-empty-pattern-replace
./spikes/api-review/bug-533-empty-pattern-replace/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
strings::replace("abc", needle, "-") = "abc"
regex::replace("abc", needle, "-")   = "-a-b-c-"
```

  where `needle` is a runtime `String` that happens to be empty — the shape the
  hazard actually takes.

- Expected: the divergence is either impossible to reach accidentally, or is
  named on both pages at the point a reader chooses between the two members.

Contrast cases, correct today:

- For any *non-empty* needle the two members agree on the common cases, which
  is what makes them feel interchangeable.
- `regex`'s zero-width handling is correct and well-specified, including the
  termination rule. This bug does not claim the behavior is wrong.
- `strings::count` already rejects the empty needle outright (bug-529), which
  is a third answer inside the same family and shows the tree has no rule here.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | pure software; expected identical |

## Root Cause

Not a defect — two correct models with an unguarded seam, the same shape as
bug-531.

`strings::replace` implements literal substring replacement, where the empty
string has no occurrence, so a guard returns `value` unchanged.
`regex::replace` implements regular-expression replacement, where the empty
pattern matches at every position including the end, so `n+1` replacements
happen for an `n`-scalar input.

Neither package knows about the other's answer. The names, parameter names and
argument order are deliberately parallel — which is good for learnability and is
exactly what makes the substitution look free.

## Goal

- Both `replace` pages state the other's empty-needle behavior, at the point a
  reader is choosing.
- A caller passing a runtime-supplied pattern to `regex::replace` has a
  documented, one-line way to be safe.
- A test pins both behaviors, so neither drifts into the other silently.

### Non-goals (must NOT change)

- `regex::replace`'s zero-width semantics for an explicitly empty pattern, or
  for any pattern that *can* match empty (`"a*"`, `"(?:)"`). Changing the
  general zero-width rule would break the documented termination guarantee and
  every pattern with an optional element.
- `strings::replace`'s no-op, which is correct for a literal.
- The parallel naming, which is a feature.
- **Tempting wrong fix, forbidden:** making `regex::replace` treat an empty
  pattern as a no-op "to match `strings`". An empty pattern is a legitimate
  regex with a defined meaning; special-casing it makes the package's own
  zero-width rule have an exception that fires only for one spelling — while
  `"(?:)"` and `"a*"` on an empty string still match. That is a worse trap than
  the current one, because it *looks* fixed.

## Blast Radius

- `src/codegen/builtins/strings/func_replace.rs` — page cross-linked.
- `src/codegen/builtins/regex/func_replace.rs` — page cross-linked; possibly
  gains a rejection (see Fix Design).
- `strings::count`, `strings::contains`, `strings::find` — the same empty-needle
  question one member over; **bug-529 owns them**, and the two bugs must land a
  consistent story. Note the dependency.
- `regex::find`, `regex::findAll`, `regex::match` with an empty pattern —
  Phase 1 must measure each. If `findAll("abc", "")` returns four starts, the
  same surprise exists in a member this bug does not currently name.
- `astrings::replace` — must agree with `strings::replace`.
- In-tree callers passing a *runtime* pattern to `regex::replace` —
  `grep -rn "regex::replace" src/ examples/ benchmark/ repository/`. Any call
  whose pattern is not a literal is a live instance of this hazard and must be
  checked, not just counted.

## Fix Design

Two parts; the first is uncontroversial.

**1. Cross-link, with the values.** Each `replace` page gains a short paragraph
naming the other member and showing both results on the same input. The spike's
two lines are the content. Cheap, and it is the honest minimum — the current
pages each document their own behavior correctly and neither mentions that the
sibling does the opposite.

**2. The runtime-pattern guard.** The realistic failure is a pattern that is
empty *by accident*. Options:

- **Document the guard.** `IF pattern <> "" THEN …`. Zero cost, protects only
  readers.
- **Reject an empty pattern in `regex::replace`** with `ErrInvalidFormat`.
  Narrow: it targets the literal empty string only, not "patterns that can match
  empty", so it does not create the false-safety trap named in Non-goals —
  provided the page says exactly that. It also matches `strings::count`'s
  existing precedent of rejecting an empty needle. Cost: it makes a valid regex
  illegal in one member, which is a real wart.

**Recommend part 1 now, and decide part 2 from the Phase 1 caller sweep.** If
in-tree code passes runtime patterns to `regex::replace`, the rejection earns
its wart; if every in-tree pattern is a literal, the cross-link is enough and
the wart is not worth it.

Rejected: a `regex::replaceLiteral`. That is `strings::replace`.

Rejected: aligning by making `strings::replace` interleave. It would change a
correct, documented no-op into a whole-string rewrite — converging on the
dangerous behavior rather than away from it.

## Phases

### Phase 1 — measure + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-533-empty-pattern-replace/` (done).
- [ ] Extend it to measure `regex::find`, `regex::findAll` and `regex::match`
      with an empty pattern, and `astrings::replace` with an empty needle.
      Record the results — the bug currently names two members and the family
      may be larger.
- [ ] `grep -rn "regex::replace" src/ examples/ benchmark/ repository/` and
      classify each call by whether its pattern is a literal. A non-literal is
      a live hazard.
- [ ] Add a fixture pinning both current behaviors.
- [ ] Decide part 2 from the sweep.

Acceptance: the family-wide empty-pattern table is measured; every in-tree
`regex::replace` call is classified; the part-2 decision is recorded.
Commit: —

### Phase 2 — cross-link

- [ ] Add the paragraph to both `replace` pages, with both results shown.
- [ ] Make sure the wording is consistent with whatever bug-529 decides for
      `strings`' internal empty-needle rule — the two must not contradict.

Acceptance: each page names the other's behavior and shows it.
Commit: —

### Phase 3 — the guard, if decided

- [ ] Reject an empty pattern in `regex::replace` with `ErrInvalidFormat`, and
      state precisely that this covers the empty *spelling* only, not every
      pattern that can match empty.
- [ ] Fix any in-tree caller from Phase 1.

Acceptance: the empty-pattern call raises; `"a*"` and `"(?:)"` still behave as
documented.
Commit: —

### Phase 4 — validation

- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh strings --run`, `regex --run`.

Acceptance: full suite green; the pinned behaviors are unchanged except where
Phase 3 deliberately changed them.
Commit: —

## Validation Plan

- Regression test: a fixture asserting both members' empty-needle results, plus
  `"a*"` and `"(?:)"` to prove the zero-width rule is intact.
- Runtime proof: `spikes/api-review/bug-533-empty-pattern-replace/`.
- Doc sync: both `replace` pages; the `regex` intro if Phase 1 finds `find`/
  `findAll` need the same note.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Whether `regex::replace` rejects an empty pattern. **Decide from the Phase 1
  sweep.** Recommend rejecting only if a non-literal pattern reaches it in-tree.
- Ordering against bug-529. **Recommend bug-529 first** — it settles the
  `strings` side's internal rule, and this bug's cross-link should quote a
  settled answer rather than one of four.

## Summary

Small, and mostly a documentation fix with one real decision behind it. The
part worth doing carefully is the Phase 1 sweep: the two-member framing may be
too narrow, since `regex::find` and `findAll` with an empty pattern have not
been measured and would carry the same surprise. The zero-width rule itself is
correct and must survive untouched.
