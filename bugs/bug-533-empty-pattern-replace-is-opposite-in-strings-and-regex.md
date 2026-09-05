# bug-533: `strings::replace` and `regex::replace` do exactly opposite things with an empty needle

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Fixed — both `strings::replace` and `regex::replace` refuse an empty
needle/pattern with `ErrInvalidArgument` (77050002). BREAKING on the `strings`
side (a documented no-op became a raise) with no compile-time signal.
Regression Test: `tests/rt-behavior/regex/replace-empty-pattern-rt`

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
whole `strings` family, and it has now been decided in a way that this
rejection completes rather than conflicts with:

> An empty needle occurs at every position, beginning at 0. A member that
> **answers a question about** an occurrence reports it; a member that
> **counts or rewrites** occurrences refuses.

So `strings::contains(v, "")` stays TRUE and `strings::find(v, "")` stays `0` —
both already correct — while `count` (already) and `replace` (this bug) refuse.
`strings::count`'s existing rejection stops being an exception and becomes the
precedent for the refusal half.

That framing also makes the cross-package story simple instead of apologetic:
`regex`'s zero-width matching *is* "present at every position", so
`regex::find(v, "")` returning `0` and `strings::find(v, "")` returning `0` are
now the same rule rather than two models that happen to agree. The only
divergence left between the packages is on the rewrite side, and this bug
removes it by making both refuse.

**bug-529 lands first**; this bug quotes its rule and supplies the one behavior
change the rule implies on the `strings` side.

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

- [x] Land `spikes/api-review/bug-533-empty-pattern-replace/` (done).
- [x] Measure `regex::find`, `regex::findAll` and `regex::match` with an empty
      pattern, and `astrings::replace` with an empty needle.
- [x] Classify every in-tree `regex::replace` caller.
- [x] Add a fixture asserting the rejection from both members, plus `"a*"` and
      `"(?:)"` asserted to still interleave. Confirm the first two fail today and
      the last two pass.
- [x] Sweep `strings::replace` callers as well.

**The measured table.** The prediction held: the `regex` query members keep their
zero-width answers, and nothing beyond the two `replace` members needed to move.

| call | before | after |
| --- | --- | --- |
| `strings::replace(v, "", r)` | `v` — a silent no-op | raises `ErrInvalidArgument` |
| `regex::replace(v, "", r)` | `"-a-b-c-"` — the whole string | raises `ErrInvalidArgument` |
| `strings::replace(a, "", r)` (`AttributedString`) | `a` | raises — inherited, see below |
| `regex::match(v, "")` | `TRUE` | `TRUE` |
| `regex::find(v, "")` | `0` | `0` |
| `regex::findAll(v, "")` | 4 matches over `"abc"` | 4 matches |
| `regex::findMatch(v, "").start` | `0` | `0` |
| `regex::replace(v, "a*", r)` | `"-b-c-"` | `"-b-c-"` |
| `regex::replace(v, "(?:)", r)` | `"-a-b-c-"` | `"-a-b-c-"` |
| `regex::replace(v, "x?", r)` | `"-a-b-c-"` | `"-a-b-c-"` |
| `collections::replace(l, "", x)` | replaces the empty ELEMENT | unchanged |

The `astrings` overload needed no separate change: `__astrings_replace`'s second
statement *is* `strings::replace(text, old, new)`, so the two members cannot
disagree and the raise propagates before the attribute overlay is touched.

`collections::replace` is the containment case worth naming. It shares the bare
native target and the same `lower_replace`, and it is untouched — an empty
*element* in a list is an ordinary value, not a degenerate needle, and the `List`
branch returns before the guard.

**The caller sweep.** `grep -rn "strings::replace(\|regex::replace(" src/ examples/
benchmark/ repository/ tests/`:

- **Product code passes only literal, non-empty needles.** `examples/browser/**`
  (`"\t"`, `"\r"`, `"\n"`, `" "`, `">"`, `" +"`, `" ?\n ?"`, `"\n{3,}"`) and
  `benchmark/mfb` (`"l"`). None can be empty.
- **In-tree stdlib callers**: `encoding::htmlEscape` (five literal entities),
  `csv::__csv_quoteField` (the dialect quote char), `astrings::__astrings_replace`
  (forwards the user's `old`, deliberately).
- `csv` needed a second look and is safe **because it validates first**:
  `__csv_firstCode` raises `ErrInvalidFormat` (`77050003`) for an empty
  `delimiter`/`quote` before `__csv_quoteField` runs. Measured:
  `csv::stringify(rows, ",", "")` still reports `77050003`, not `77050002`.
- Tests migrated: `tests/acceptance/src/{regex,general,astrings}.mfb`,
  `tests/rt-behavior/regex/regex-from-string-rt`, `tests/rt_regex_bounds.rs`
  (corpus row 49), `tests/rt_regex_span.rs` (the reconstruction cross-check), and
  bug-529's own family pin, whose golden records the change.

Acceptance: met.
Commit: (this commit)

### Phase 2 — the convergence

- [x] Reject an empty needle in `strings::replace` with `ErrInvalidArgument` —
      in the shared `lower_replace`, on its `String` path only.
- [x] Reject an empty pattern in `regex::replace` with the same code, as a guard
      at the top of `__regex_replace` before the pattern is compiled.
- [x] The `astrings::replace` overload inherits it (see Phase 1).
- [x] Migrate every in-tree caller from Phase 1.
- [x] Write the "guard on the spelling, not on zero-width matching" paragraph
      into `regex::replace`'s page, in the words Fix Design specified.
- [x] Cross-link both pages and keep the wording consistent with bug-529's rule.
      bug-529's package-level rule loses its recorded exception: `replace` now
      sits with `count` and `split`, and the sentence naming it as the one
      deviation is deleted from both the man page and the spec.

**The `"old` longer than `value`" path was NOT collapsed into the guard.** The
two conditions used one branch target; only the empty case moved, so an `old`
longer than `value` still copies `value` through as an ordinary no-match. That is
a separate documented behaviour and pinning it was the point of the fixture's
`noMatch=` line.

Acceptance: met — both members raise on an empty needle; `"a*"`, `"(?:)"`, `"x?"`
and `"\b"` are unchanged; each page states the narrowness of the guard.
Commit: (this commit)

### Phase 3 — regenerate + validation

- [x] Both members gain an error they did not have; check whether any
      `TYPE_INLINE_TRAP_DEAD_HANDLER` warning flips. **It did, and it was a
      MISCOMPILE — see below.** This was the most important line of the phase.
- [x] Regenerate the `.ncodesum` goldens (`bash scripts/regen-ncodesum.sh`).
- [x] `cargo test --release --no-fail-fast`; `scripts/test-accept.sh`.
- [x] `scripts/man-run-examples.sh` for `strings`, `regex`, `astrings`,
      `collections`, `csv`, `encoding` — all green.
- [x] Update the spike to assert the converged behavior. bug-529's spike was
      updated with it (its unguarded `strings::replace("hi","","x")` began
      aborting the program).

**The consequence the plan did not anticipate: a name-keyed infallibility census
turned this into a miscompile.** `strings::replace` and `collections::replace`
dequalify to the ONE bare native target `replace`
(`builtins::native_builtin_target`), and `replace` was on
`inline_builtin_is_infallible`'s name-keyed list. So after the guard landed, this
program compiled with a warning and then **died**:

```
LET s AS String = strings::replace("abc", empty, "-") TRAP(err)
  io::print("HANDLER RAN code=" & toString(err.code))
  RECOVER "recovered"
END TRAP
```

    warn[2-203-0104 TYPE_INLINE_TRAP_DEAD_HANDLER]: inline TRAP handler is
    unreachable — `strings.replace` cannot fail, so the handler is dead code.
    Error: 7-705-0002

The front-end asserted the call could not fail, elided a **live** handler, and
the error propagated past it. That is precisely the failure
`arg_type_makes_inline_builtin_fallible` exists to prevent (bug-486 found the same
shape in `toString`), and it is invisible to every behavioural test that wraps the
call in a *function-level* `TRAP` — which is what the fixture originally did.

The fix makes `replace`'s verdict argument-typed, as bug-486 did for `toString`,
and it **fails closed**: only a provable `List` first argument is infallible.
Being fallible also makes it raw-supported through the same early return, so the
inline `TRAP` traps the real error rather than being rejected for having no
lowering — which needed one more edit, wiring `Some("replace") => lower_replace`
into `lower_inline_builtin_raw`'s dispatch beside `find` and `mid`.

`codegen::builtins::tests::inline_builtin_fallibility_census` asserted
`strings.replace` was infallible and is corrected: the untyped verdict moves to
the fallible list, and a new `replace_is_fallible_only_on_a_string` gives the
per-overload verdict, mirroring `tostring_is_fallible_only_on_a_byte_list`. Under
AGENTS.md's four-question gate the row was **proven wrong by the repro above**,
not merely inconvenient.

**Golden delta, and why it is exactly this.**

* Behavioural (`build.log`): **three lines, tree-wide.**
  `strings-empty-needle-rt`'s `replace empty=` for the `String` and
  `AttributedString` overloads, and `regex-from-string-rt`'s `zw_rp_empty`. Every
  other line of every other fixture is byte-identical — including
  `zw_rp_astar` and `zw_fa_empty` in that same fixture, which is the containment
  proof that zero-width matching did not move.
* `.ir`: four `regex` fixtures, from the three-line guard shifting the embedded
  package source; plus the two fixtures whose own source this change edited.
* `.ncodesum`: **9 fixtures × 5 targets**, and the membership was PROVED rather
  than assumed. `lower_replace` is shared, so every binary emitting a `replace`
  carries the new guard. Grepping the built `.ncode` for the new label
  `replace_empty_old` gives exactly nine fixtures with a non-zero count —
  strings, csv, tls, resource-xfer-slots, regex, json, encoding, crypto,
  crypto-ec-valid — and those nine are exactly the nine whose sums moved. All 19
  other byte-identity fixtures count 0 and kept byte-identical sums.

Acceptance: met.
Commit: (this commit)

## The break, stated

`strings::replace(value, "", new)` returned a copy of `value` and now raises
`ErrInvalidArgument` (`77050002`). `regex::replace(value, "", replacement)`
returned `value` with the replacement interleaved at every position and now raises
the same code. Neither return type moved, so nothing catches an unmigrated caller
at build time.

A caller whose needle can be empty at run time either checks it or wraps the call
in a `TRAP`. A caller passing a literal non-empty needle is unaffected.

**The guard is on the empty needle/pattern STRING only.** `regex::replace` still
interleaves for every pattern that matches zero-width — `"a*"`, `"x?"`, `"(?:)"`,
`"\b"` — and every `regex` query member still answers for an empty pattern
(`match` `TRUE`, `find` `0`, `findAll` one match per position, `findMatch.start`
`0`).

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

- Ordering against bug-529. **Land bug-529 first** — settled, not open. Its
  rule is now decided ("an empty needle occurs at every position; query members
  report it, counting and rewriting members refuse"), and this bug supplies the
  one behavior change that rule implies on the `strings` side. Nothing here
  needs revisiting.
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
