# plan-59-C: STATE opacity — the narrowing rule

Last updated: 2026-07-20
Effort: small
Depends on: nothing
Produces: a new rule rejecting an opaque-`STATE` value bound or returned under a
concrete `STATE`, plus a corrected §15.5. Consumed by plan-59-E, which cannot
land until this does — `E` is what lets a parameter escape, and this is the rule
that keeps an opaque `STATE` from being laundered into a concrete one once it can.

`res.md` §4 records the `STATE` visibility model as **"Unresolved"** under Track
B, on the grounds that the table's soundness "rests on the borrow rule". That
assessment predates `TYPE_STATE_MISMATCH` (2-203-0129) in its current form and
**overstates the problem**. Empirically (see Verified properties), `STATE`
agreement is already a whole-program invariant checked independently at bindings,
parameters, and returns; it does not rest on the escape rule. What actually
breaks under Track B is one narrow case: the bare **parameter**, the only
position where the checker deliberately does not know the concrete `STATE`.

Behavioral outcome: an opaque-`STATE` resource cannot be returned or bound under
a concrete `STATE` type, and §15.5 says what the compiler actually does.

References:

- `planning/res.md` §4 (the "Unresolved" note this sub-plan resolves), §3.4
- `./mfb spec language resource-management` §15.5 — the table being corrected
- `src/rules/table.rs` — `TYPE_STATE_MISMATCH` (2-203-0129)
- Prerequisites: see plan-59-A.

## 1. Goal

- Returning or binding a resource whose `STATE` is opaque (a bare `RES`
  parameter) under a concrete `STATE T` is rejected at compile time.
- §15.5's table and its justification paragraph describe the implemented
  behavior: bare **asserts** no state and is checked, rather than "carrying no
  state" or stripping it.

### Non-goals (explicit constraints)

- **No runtime `STATE` type tag.** This was considered and rejected — see §3.
  `STATE` stays fully compile-time checked.
- **No change to `TYPE_STATE_MISMATCH`'s existing behavior.** Its three current
  arms (binding, parameter, return) keep their messages and codes.
- **No removal of the bare-parameter opt-out.** `FUNC closeFile(RES f AS T)`
  accepting any state is what every close op depends on
  (`bindings/libsnd/src/lib.mfb:139`, `bindings/sqlite3/src/lib.mfb`).

## 2. Current State

`TYPE_STATE_MISMATCH` (2-203-0129) enforces that a resource's `STATE` type is
fixed at its owning binding and every other declaration must agree. Verified
empirically by compiling a probe (not by reading alone):

```basic
RES a AS File STATE Cur = fs::open("/tmp/x.txt", "w")
RES b AS File           = a   ' error 2-203-0129: "binding `b` is bare but its
                              '   initializer carries STATE `Cur`; a bare binding
                              '   asserts the resource has no state"
RES c AS File STATE Other = a ' error 2-203-0129: "declares `STATE Other` but its
                              '   initializer carries STATE `Cur`"
FUNC takeOther(RES f AS File STATE Other)   ' call with `a`:
                              ' error 2-203-0129: "a parameter observes a
                              '   resource's state, it cannot re-type it"
FUNC takeBare(RES f AS File)                ' call with `a`: ACCEPTED (opaque)
```

So three of the four positions already reject disagreement, and none of those
three checks consults the escape rule. Only the bare parameter is permissive, and
deliberately so.

### Measured populations

| What | Count | Command |
|---|---|---|
| `TYPE_STATE_MISMATCH` arms exercised by the probe above | 3 of 4 positions reject; bare param accepts | compiled `/tmp/st` probe, 2026-07-20 |
| Bare `RES` params in `LINK` blocks (the opt-out's real users) | UNMEASURED — Phase 1's first task | see Phase 1 |

### Verified properties

- **`STATE` agreement does not rest on the escape rule.** Verified by compiling
  the probe above: the binding and parameter arms fire with no `RETURN` and no
  escape involved. This is the finding that reduces §4's "Unresolved" from a
  re-derivation to a wording fix plus one rule.
- **Bare does not strip `STATE`; it asserts its absence.** Verified by the `RES b
  AS File = a` arm — a bare binding over a stateful resource is an *error*, not a
  silent erasure. §15.5's "binds a resource carrying no state" phrasing describes
  an assertion, not an operation, and is the sentence that misleads.
- **UNVERIFIED — whether anything legitimate needs opaque → concrete.** If some
  real binding pattern requires recovering a concrete `STATE` from a bare
  parameter, forbidding the narrowing breaks it. Phase 1 measures this before the
  rule is written.

## 3. Design Overview

One static rule: **an opaque-`STATE` value may not be returned or bound under a
concrete `STATE`.** Opacity propagates as opacity; only a producer that knows the
type may name it.

This is not type confusion — it is an *unprovable narrowing*. In:

```basic
FUNC launder(RES f AS SoundFile) AS RES SoundFile STATE Cursor
  RETURN f     ' f's STATE is opaque here: "some state or none"
END FUNC       ' the checker cannot prove this return carries a Cursor
```

the checker knows only that `f` has *some* state. Declaring a concrete return
`STATE` is a claim it cannot discharge, so it is rejected — the same shape as any
other unprovable narrowing, and it needs no runtime support.

**Rejected alternative — a runtime `STATE` type tag** (a type id in the spare 62
bits of the closed-flag word, checked on `.state`). This was the initial proposal
and is **withdrawn**: it solves a problem that does not exist, because `STATE`
disagreement is already caught statically at every position where the type is
known, and the one position where it is not known is fixed by forbidding the
narrowing. Cost avoided: a stable cross-package type id, which would have needed
the `.mfp` encoding to carry it.

**Rejected alternative — delete the bare-parameter row.** Would require one
overload of every close op per state type its callers might attach.

**Rejected alternative — `STATE` on the `RESOURCE` declaration.** Explicitly
decided against in plan-52-A's Non-goals; forfeits the bare-param opt-out.

**Where design uncertainty concentrates:** entirely in the UNVERIFIED property —
whether forbidding the narrowing breaks a real pattern. Phase 1 is that
measurement, and it is cheap.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT
> DONE.**

### Phase 1 — Measure whether the narrowing is ever needed

Falsifies the rule's premise before writing it.

- [ ] Enumerate every bare `RES` parameter across `bindings/`, `tests/`, and
      `src/builtins/*.mfb`, and classify each as (a) close op, (b) pass-through,
      (c) something that recovers a concrete `STATE`. Record the counts and the
      command in Measured populations above.
- [ ] If any case (c) exists, describe it here and reconsider the rule before
      proceeding — the design assumes (c) is empty.

Acceptance: the bare-param population is measured and written into this document
with its command, and case (c) is either empty or enumerated.
Commit: —

### Phase 2 — Fix §15.5's wording

Independently valuable: §15.5 is wrong about current behavior regardless of
Track B.

- [ ] In `src/docs/spec/language/15_resource-management.md` §15.5, change the
      Return and Binding rows from "carrying **no** state" to wording that says
      bare *asserts* no state and is checked against the resource's actual state,
      citing `TYPE_STATE_MISMATCH`.
- [ ] Rewrite the justification blockquote (currently: bare is sound "only
      because the alias cannot escape") to state the real reason: `STATE` is
      fixed at creation and every declaration is checked against it, independent
      of escape.
- [ ] Tests: `cargo test --bin mfb spec` — `every_rule_is_documented_in_the_spec`,
      `spec_links_resolve`, `spec_citations_resolve`.

Acceptance: §15.5 describes the behavior the probe in §2 demonstrates; spec tests
green.
Commit: —

### Phase 3 — Add the narrowing rule

- [ ] Add a rule to `src/rules/table.rs` in the `2-203-xxxx` type family —
      suggested name `TYPE_STATE_OPAQUE_NARROWING`, message "an opaque resource
      STATE cannot be narrowed to a concrete STATE type". Take the next free code
      (verify with `grep -c 'code: "2-203-' src/rules/table.rs` and read the
      surrounding rows; do not reuse a retired code).
- [ ] Emit it where `TYPE_STATE_MISMATCH`'s return and binding arms live, for the
      case where the source's `STATE` is opaque rather than disagreeing.
- [ ] Tests: syntax fixtures `tests/syntax/resources/state-opaque-narrow-return-invalid`
      and `…-bind-invalid`, plus a positive fixture proving opaque → opaque still
      passes.
- [ ] Document the rule in `src/docs/spec/diagnostics/01_rule-codes.md` and in
      §15.5.

Acceptance: the `launder` shape in §3 is rejected with the new code; opaque →
opaque and concrete → matching-concrete both still compile; `cargo test` green.
Commit: —

## Validation Plan

- Tests: three new syntax fixtures (two negative, one positive) under
  `tests/syntax/resources/`.
- Coverage check: syntax fixtures assert on `build.log`, so the rule text is in
  the denominator. Seed goldens for new fixtures before asserting.
- Runtime proof: none needed — this is a compile-time rule. The positive fixture
  compiling *is* the proof the opt-out survives.
- Doc sync: §15.5 and `diagnostics/01_rule-codes.md`; update `planning/res.md`
  §4's "Unresolved" note to point here.
- Acceptance: `cargo test`; `scripts/test-accept.sh target/debug/mfb <tmp>
  'state*' 'resource*'` with a hermetic `MFB_HOME`.

## Open Decisions

- **Should the narrowing be rejected, or permitted with a runtime check?**
  Recommend rejecting (§3). A runtime check would reintroduce the type tag this
  sub-plan exists to avoid.
- **Does `res.md` §4's "Unresolved" get rewritten or annotated?** Recommend
  annotating with a pointer here rather than rewriting — §4 is a record of the
  reasoning at the time. (Consistent with leaving `res.md`'s analysis prose
  intact during the terminology purge.)

## Corrections

<!-- Filled in during execution. -->

## Summary

This sub-plan is mostly a correction to the spec plus one small rule. The real
finding is that `res.md` §4's "Unresolved" was pessimistic: `STATE` never needed
the escape rule, so Track B does not require re-deriving the `STATE` model.

Untouched: `TYPE_STATE_MISMATCH`, the bare-parameter opt-out, and every close op.
No runtime machinery is added.
