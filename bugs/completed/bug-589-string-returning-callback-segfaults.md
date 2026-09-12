# bug-589: a `String`-returning function used as a CALLBACK double-frees and SIGSEGVs

Last updated: 2026-09-12
Effort: none — no code change
Severity: HIGH (as filed)
Class: Memory / correctness (double free)

Status: **CLOSED — DUPLICATE of [bug-562](completed/bug-562-string-callback-tostring-identity-segfaults.md),
fixed 2026-09-07 in `1bba27392`, five days before this document was filed.**

The defect was real and the analysis below is CORRECT — it was verified
empirically, not waved away. It was simply already fixed. See "Verification"
for the measurements, including a negative control that reproduces the crash on
demand.

Regression Test: already committed with bug-562 —
`tests/rt-behavior/collections/callback-string-return-identity-rt` (plus four
`cargo test` guards, listed under "The guards are live").

## How this was filed against a fixed tree

- 2026-09-06 `b845db0de` — bug-536 shape B-2 lands. While fixing it, three
  defects are measured and written into **bug-536's own document**.
- 2026-09-06 `8c57683f3` — those same three are filed as **bugs 560, 561, 562**.
- 2026-09-07 `1bba27392` — **bug-562 is FIXED** and archived (`0c9e3ea8a`).
- 2026-09-12 `1cee4a8bf` — a session re-reads bug-536's document, whose item 3
  still carries the 09-06 text verbatim ("**The fix is one word:** drop the
  `callback_referenced` arm"), and files it a **second** time as bug-589.

bug-536's doc says "All three reproduce unchanged on the base commit". That was
true on 2026-09-06. It was not re-measured on 2026-09-12, and this document
inherited the claim — which is exactly the failure mode its own Reproduction
section warns about ("NOT independently confirmed").

**The same applies to its two siblings filed in the same commit** — see
"Siblings" below.

## Reproduction (as filed)

```basic
IMPORT io
IMPORT collections
FUNC identish(s AS String) AS String
  RETURN toString(s)
END FUNC
SUB main()
  MUT xs AS List OF String = []
  MUT k AS Integer = 0
  WHILE k < 3
    xs = collections::append(xs, "n" & toString(k))
    k = k + 1
  END WHILE
  LET c AS List OF String = collections::transform(xs, identish)   ' [exit 139]
  io::print("c=" & collections::get(c, 0))
END SUB
```

## Verification

Run verbatim, as a console executable, against `target/release/mfb` built from
`9956912f1` (main).

| binary | result |
|---|---|
| main (`9956912f1`) | `c=n0`, **exit 0**, stable over **50 consecutive runs** |
| main + the `callback_referenced` arm re-added (negative control) | **exit 139**, no output, on 5/5 runs |

The negative control is the point. Re-adding *only* the one arm this document
asks to remove —

```rust
if callback_referenced.contains(&f.name) {
    return false;
}
```

— to `function_returns_fresh_string`
(`src/codegen/engine/function/function_lowering.rs:230`) restores the crash
exactly. So:

1. the defect described here is **real**, and this document's root-cause
   attribution is **correct in every particular**;
2. the fix already on main is **load-bearing**, not incidental;
3. there is **nothing left to do**.

### The guards are live

The committed regression fixture was run against the negative-control binary and
went RED with the filed signature — `scripts/test-accept.sh` exit **1**, the
build.log diff collapsing twenty-one lines of expected output into a single
`+[exit 139]`. It is a live guard, not a stale one.

Also committed with bug-562, all still present:

- `tests/codegen/codegen_string_return_freshness.rs::being_used_as_a_callback_never_removes_the_return_copy`
  — the invariant as an owner count;
- `…::a_callback_whose_result_is_already_fresh_is_not_copied_twice` — the
  positive pin this document asks for;
- `tests/runtime/rt_scope_drop_leaks.rs::a_callback_invoked_through_a_user_function_returns_an_owned_block`
  — the same defect with no `collections` import at all;
- `…::a_direct_call_to_a_callback_referenced_string_callee_runs_at_constant_rss`
  and `…::a_direct_call_to_a_plain_string_callee_still_runs_at_constant_rss`.

### The enumeration this document asks for is total, and now enforced

This document asks for a wildcard-free `match` over every producer that can reach
a `FunctionRef` return slot. The landed fix is **stronger than that**: it does
not enumerate producers at all. `lower_returned_value`'s final arm is a catch-all
keyed on the lowered TYPE, not on a recognised shape —

```rust
if self.current_returns_fresh_string && lowered.type_ == ParameterType::String {
```

— so an unrecognised producer is **copied, not assumed fresh**. A new NIR value
variant cannot silently become a fresh-assumed return; it falls into the copy.
That is the fail-closed direction, and it removes the recogniser/measurer drift
risk rather than guarding it.

The one callback source that genuinely is invisible to the predicate — a builtin
passed directly as a `FunctionRef`, which is not in `module.functions` — was a
fact about a list ("all eight admitted names return `Boolean`, so none can hand
back a block") until bug-569 turned it into an enforced invariant:
`src/codegen/builtins/general/mod.rs::every_builtin_that_can_be_a_callback_returns_boolean`
asserts the admitted set is exactly 8 and that every member returns `Boolean`
across twelve argument spellings. Admitting a `String`-returning name goes red.

### Contract

§14.3 "Returning a value moves it into the caller's return slot", under §14's
"each live value is owned by exactly one binding, container slot, temporary,
closure environment, thread message, or return slot". `identish`'s block had two
owners; the HOF freed its one. The fix only ever ADDS a copy — §14.1 licenses
replacing a semantic copy with a move only when the source is provably unused
after, and lowering was taking that elision without the proof.

## Doc-vs-code drift this closed

`.ai/codegen-invariants.md` §"A `.mfb` callee's `String`" still described the
predicate as excluding callback-referenced functions and still said "Dropping the
arm is the fix; it wants its own callback-ABI audit" — **five days after the arm
was dropped**. The code was right and the doc was wrong; the doc is the proximate
cause of this re-filing. Corrected in the same commit as this closure.

## Siblings — likely the same re-filing

`1cee4a8bf` filed three bugs from bug-536's stale item list. All three map to
bugs filed from the *same* item list on 2026-09-06 and since completed:

| re-filed | original | original status |
|---|---|---|
| bug-587 `s = s & <expr>` self-append leak | `bugs/completed/bug-560-mut-string-self-append-leaks.md` | completed |
| bug-588 `Result OF T` bound through `TRAP` never freed | `bugs/completed/bug-561-trap-result-binding-never-freed.md` | completed |
| bug-589 (this) | `bugs/completed/bug-562-…` | completed, **verified above** |

Only bug-589 was re-measured here. **bug-587 and bug-588 should each be
re-measured before any work starts on them** — both are RSS-growth leaks
requiring a 200k/400k iteration measurement, so neither can be settled by
inspection.
