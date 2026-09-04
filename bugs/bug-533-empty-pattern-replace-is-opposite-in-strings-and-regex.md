# bug-533: `strings::replace` and `regex::replace` do exactly opposite things with an empty needle

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

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

**Decided (2026-09-04): converge — both members REJECT an empty needle/pattern.**

The instruction was to make the two the same and pick whatever works best.
"The same" turns out to point at a third answer rather than either current one,
because both existing behaviors fail as a convergence target:

- *Both no-op* (adopt `strings`' answer) requires `regex::replace` to
  special-case the empty pattern — which is the trap named in the original
  Non-goals below. It fires only for the empty *spelling*, while `"a*"` and
  `"(?:)"` still match at every position. It looks fixed and is not.
- *Both interleave* (adopt `regex`'s answer) converges on the destructive
  behavior: `strings::replace(text, needle, "-")` with an accidentally-empty
  `needle` would rewrite the whole string instead of doing nothing.

Rejecting is the third option, and it is already the package's own precedent:
`strings::count` raises `ErrInvalidArgument` on an empty needle today
(bug-529). That yields one rule with a reason behind it — **a query member
answers for an empty needle; a member that counts or rewrites refuses it.**

- `strings::replace(v, "", r)` raises `ErrInvalidArgument` (77050002),
  matching `strings::count`.
- `regex::replace(v, "", r)` raises the same code.
- Both pages state, explicitly, that this rejects the empty *spelling* only —
  it does not change what `"a*"` or `"(?:)"` do.
- A test pins both, plus `"a*"` and `"(?:)"`, so the narrow rule cannot widen
  into the zero-width rule by accident.

### Non-goals (must NOT change)

- `regex::replace`'s zero-width semantics for any pattern that *can* match
  empty (`"a*"`, `"(?:)"`, `"x?"`). Changing the general zero-width rule would
  break the documented termination guarantee and every pattern with an optional
  element. **The rejection is a guard on one input spelling, not a change to
  the matching rule**, and the page must say so or it recreates the trap.
- The parallel naming, which is a feature.
- `regex::find`/`findAll`/`match` with an empty pattern — unless Phase 1's
  measurement says otherwise, they keep their zero-width answers. They are
  query members, and the rule above only refuses on the rewrite side.
- **Tempting wrong fix, forbidden:** making `regex::replace` treat an empty
  pattern as a silent no-op "to match `strings`". An empty pattern is a
  legitimate regex with a defined meaning; a silent special case makes the
  package's own zero-width rule have an invisible exception. Refusing is
  honest; pretending it matched nothing is not.
- **Also forbidden:** making `strings::replace` interleave. That converges on
  the destructive reading of an input the caller did not mean to supply.

## Blast Radius

- `src/codegen/builtins/strings/func_replace.rs` — **behavior change**: the
  documented empty-needle no-op becomes `ErrInvalidArgument`. This is the
  larger half of the fix, because a no-op-to-raise change is silent success
  turning into a TRAP with no compile error in between.
- `src/codegen/builtins/regex/func_replace.rs` — **behavior change**: the
  empty-pattern interleave becomes `ErrInvalidArgument`, guarded to the empty
  spelling only.
- `astrings::replace` — must agree with `strings::replace`; changed here.
- `strings::count`, `strings::contains`, `strings::find` — the same empty-needle
  question one member over; **bug-529 owns them**, and the two bugs must land a
  consistent story. `strings::count` already rejects, which is the precedent
  this decision follows. **Hard dependency: land bug-529 first.**
- `regex::find`, `regex::findAll`, `regex::match` with an empty pattern —
  Phase 1 must measure each. Under the query-answers/rewriter-refuses rule they
  keep their zero-width answers, but that is a prediction and not yet a result.
- In-tree callers of **both** members —
  `grep -rn "regex::replace\|strings::replace" src/ examples/ benchmark/ repository/`.
  Classify each by whether its needle/pattern can be empty at run time; every
  one that can is migrated in the same change. A literal non-empty needle is
  unaffected.
- Acceptance goldens containing an empty-needle `replace` result — enumerated
  in Phase 3.

## Fix Design

Both members reject an empty needle/pattern with `ErrInvalidArgument`
(77050002), the code `strings::count` already uses for the same input.

**Why rejecting rather than agreeing on a value.** The population at risk is a
needle that is empty *unintentionally* — from a config file, a form field, a
`--replace` flag. For that caller, every valued answer is wrong in a different
way: a no-op hides a misconfiguration, and interleaving destroys the text. An
error is the only outcome that reports the thing that actually went wrong,
which is that no needle was supplied.

**The wart, stated plainly.** An empty pattern is a legitimate regex with
well-defined behavior, and `regex::replace` will now refuse it. That is a real
cost and the page must own it rather than imply the zero-width rule changed:

> `regex::replace` refuses an empty `pattern`. This is a guard on the empty
> pattern *string*, not a change to zero-width matching — `"a*"`, `"x?"` and
> `"(?:)"` still match at every position, and `regex::replace(v, "a*", "-")`
> still interleaves.

Without that sentence the fix recreates the trap it was meant to avoid: a
reader concludes zero-width matching was tamed, and is then surprised by the
first optional quantifier they write.

**Ordering against bug-529.** That bug settles the empty-needle rule for the
whole `strings` family and recommended *never matches* for the query members
(`contains` FALSE, `find` raises `ErrNotFound`). This decision is compatible
with it and sharpens it into one rule: **query members answer, counting and
rewriting members refuse.** bug-529 should adopt that framing, and
`strings::count`'s existing rejection stops being an exception and becomes the
precedent.

Rejected: a `regex::replaceLiteral`. That is `strings::replace`.

Rejected: aligning by making `strings::replace` interleave. It would change a
correct, documented no-op into a whole-string rewrite — converging on the
dangerous behavior rather than away from it.

Rejected: making `regex::replace` a silent no-op on the empty pattern. Narrower
than it looks and dishonest: it reports success for a call that matched
nothing, while every other zero-width pattern still matches everywhere.

Rejected: `ErrInvalidFormat` for the `regex` side. It is what `regex` raises for
a malformed pattern, and an empty pattern is not malformed — it is a
well-formed pattern this member declines. Matching `strings::count`'s
`ErrInvalidArgument` also means a caller wrapping either member needs one code,
not two.

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
- [ ] Add a fixture asserting the *desired* rejection from both members, plus
      `"a*"` and `"(?:)"` asserted to still interleave. Confirm the first two
      fail today and the last two pass.
- [ ] `grep -rn "strings::replace" src/ examples/ benchmark/ repository/` as
      well — `strings::replace` is changing from a no-op to a raise, which is
      the larger behavior change of the two and needs its own caller list.

Acceptance: the family-wide empty-pattern table is measured; every in-tree
caller of both members is classified; the rejection fixture fails for the
documented reason while the zero-width fixtures pass.
Commit: —

### Phase 2 — the convergence

- [ ] Reject an empty needle in `strings::replace` with `ErrInvalidArgument`.
- [ ] Reject an empty pattern in `regex::replace` with the same code.
- [ ] Apply the same to the `astrings::replace` overload.
- [ ] Migrate every in-tree caller from Phase 1 that can pass an empty value.
- [ ] Write the "guard on the spelling, not on zero-width matching" paragraph
      into `regex::replace`'s page, in the exact words from Fix Design. Without
      it the fix recreates the trap.
- [ ] Cross-link both pages, and keep the wording consistent with bug-529's
      query-answers/rewriter-refuses framing.

Acceptance: both members raise on an empty needle; `"a*"` and `"(?:)"` are
unchanged; each page states the narrowness of the guard.
Commit: —

### Phase 3 — regenerate + validation

- [ ] Both members gain an error they did not have; check whether any
      `TYPE_INLINE_TRAP_DEAD_HANDLER` warning flips.
- [ ] Regenerate the `.ncodesum` goldens the descriptor change shifts (run the
      regen scripts under **bash**).
- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh strings --run`, `regex --run`, `astrings --run`.
- [ ] Update the spike to assert the converged behavior.

Acceptance: full suite green; golden deltas are only the two members'; the
zero-width behavior is provably unchanged.
Commit: —

## Validation Plan

- Regression test: a fixture asserting both members' empty-needle results, plus
  `"a*"` and `"(?:)"` to prove the zero-width rule is intact.
- Runtime proof: `spikes/api-review/bug-533-empty-pattern-replace/`.
- Doc sync: both `replace` pages; the `regex` intro if Phase 1 finds `find`/
  `findAll` need the same note.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

**Decided (2026-09-04): both members reject, with `ErrInvalidArgument`.**
Neither current behavior was a safe convergence target — one requires a
dishonest special case in `regex`, the other adopts the destructive answer in
`strings` — so the third option wins, and it matches `strings::count`'s
existing precedent.

Still open:

- Ordering against bug-529. **Land bug-529 first.** It settles the whole
  `strings` empty-needle family, and this decision should slot into its rule
  ("query members answer, counting and rewriting members refuse") rather than
  arrive as a fifth independent answer. If bug-529 chooses a different framing,
  this decision must be revisited rather than layered on top.
- Whether `regex::find`/`findAll`/`match` need the same treatment.
  **Decide from the Phase 1 measurement**, which has not been taken. Under the
  rule above they are query members and should keep their zero-width answers,
  but that is a prediction, not a result.

## Summary

Now a behavior change to two members rather than a documentation fix, and the
risk moved with it. `strings::replace` going from a documented no-op to a raise
is the larger half — it is a silent-success-to-TRAP change with no compile
error to catch an unmigrated caller — so Phase 1 must sweep `strings::replace`
callers as carefully as `regex::replace` ones. The zero-width matching rule is
untouched, and the single most important line in the whole fix is the sentence
on `regex::replace`'s page saying so.
