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
| Signatures with a `RES` parameter, across `bindings/`, `tests/`, `src/builtins/` | 114 | `grep -rhoE "^\s*(FUNC\|SUB) [A-Za-z_]+\([^)]*RES [^)]*\)" --include="*.mfb" bindings/ tests/ src/builtins/ \| wc -l` → 114 |
| …of those, mentioning `STATE` on a parameter | 17 | same pipeline, `\| grep -c STATE` → 17 |
| …therefore **bare** `RES` parameters (the opt-out's real users) | **97** | 114 − 17 |
| Distinct registered `CLOSE BY` ops (case **a**) | 12 | `grep -rhoE "RESOURCE [A-Za-z_]+ CLOSE BY [A-Za-z_:]+" --include="*.mfb" bindings/ tests/ src/builtins/ \| sed 's/.*CLOSE BY //' \| sort -u \| wc -l` → 12 |
| **Case (c) — recovers a concrete `STATE` from an opaque one** | **0** | two independent searches, both empty; see Phase 1 and Verified properties |

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

- [x] Enumerate every bare `RES` parameter across `bindings/`, `tests/`, and
      `src/builtins/*.mfb`, and classify each as (a) close op, (b) pass-through,
      (c) something that recovers a concrete `STATE`. Record the counts and the
      command in Measured populations above. — done; 97 bare params, 12 distinct
      close ops, **0 in case (c)**. Commands recorded in the table.
- [x] If any case (c) exists, describe it here and reconsider the rule before
      proceeding — the design assumes (c) is empty. — **(c) is empty**, confirmed
      by two independent searches rather than one. See C1.

Acceptance: the bare-param population is measured and written into this document
with its command, and case (c) is either empty or enumerated.
**MET** — population measured into the table above with commands; case (c) is
empty, established by two independent searches (see C1).
Commit: 9f0404d60

### Phase 2 — Fix §15.5's wording

Independently valuable: §15.5 is wrong about current behavior regardless of
Track B.

- [x] In `src/docs/spec/language/15_resource-management.md` §15.5, change the
      Return and Binding rows from "carrying **no** state" to wording that says
      bare *asserts* no state and is checked against the resource's actual state,
      citing `TYPE_STATE_MISMATCH`. — both rows now read "**asserts** the resource
      has no state; rejected if it carries one", with the probe from §2 inlined as
      a worked example so the assertion is visible rather than asserted.
- [x] Rewrite the justification blockquote (currently: bare is sound "only
      because the alias cannot escape") to state the real reason: `STATE` is
      fixed at creation and every declaration is checked against it, independent
      of escape. — rewritten around *what the compiler can prove*, not lifetime.
      This mattered more than the plan implies: the old blockquote founded
      soundness on `TYPE_RESOURCE_INVALIDATE_NOT_OWNER`, **the rule plan-59-E
      deletes**, so §15.5 would have been left citing a retired rule. See C2.
- [x] Tests: `cargo test --bin mfb spec` — `every_rule_is_documented_in_the_spec`,
      `spec_links_resolve`, `spec_citations_resolve`. — 48 passed.

Acceptance: §15.5 describes the behavior the probe in §2 demonstrates; spec tests
green.
**MET** — §15.5 now states the assertion-and-check model, with the §2 probe
inlined as its worked example; `cargo test --bin mfb spec` → 48 passed.
Commit: 9f0404d60

### Phase 3 — Add the narrowing rule

- [x] Add a rule to `src/rules/table.rs` in the `2-203-xxxx` type family —
      suggested name `TYPE_STATE_OPAQUE_NARROWING`, message "an opaque resource
      STATE cannot be narrowed to a concrete STATE type". Take the next free code
      (verify with `grep -c 'code: "2-203-' src/rules/table.rs` and read the
      surrounding rows; do not reuse a retired code). — added as **`2-203-0133`**;
      highest existing was `0132` and `0133` was confirmed unused tree-wide before
      taking it.
- [x] Emit it where `TYPE_STATE_MISMATCH`'s return and binding arms live, for the
      case where the source's `STATE` is opaque rather than disagreeing. — both
      arms emit. Required a new piece of state, `current_opaque_params`, because
      an opaque value and a stateless one are **indistinguishable by type string**;
      see C1.
- [x] Tests: syntax fixtures `tests/syntax/resources/state-opaque-narrow-return-invalid`
      and `…-bind-invalid`, plus a positive fixture proving opaque → opaque still
      passes. — all three added and green. The positive fixture covers three
      shapes, not one: opaque→opaque, concrete→matching-concrete, and a producer
      naming its own state.
- [x] Document the rule in `src/docs/spec/diagnostics/01_rule-codes.md` and in
      §15.5. — both done.

Acceptance: the `launder` shape in §3 is rejected with the new code; opaque →
opaque and concrete → matching-concrete both still compile; `cargo test` green.
**MET** — the §3 `launder` shape is rejected with `2-203-0133` at the right line
and caret, in both the return and binding positions; opaque→opaque,
concrete→matching-concrete, and a producer naming its own state all still
compile (`state-opaque-narrow-valid`, exit 0). `cargo test` → 21 suites, 0
failed; acceptance → 109 tests across `state*` `resource*` `native*` `libsnd*`.
Commit: 9f0404d60

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

- ~~**Should the narrowing be rejected, or permitted with a runtime check?**~~
  **DECIDED (owner, 2026-07-20): reject.** `STATE` stays entirely compile-time
  checked; no runtime `STATE` machinery is added anywhere in plan-59.
- ~~**Does `res.md` §4's "Unresolved" get rewritten or annotated?**~~
  **DECIDED (owner, 2026-07-20): annotate.** Add a pointer to this sub-plan
  beside §4's "Unresolved" note; do **not** rewrite the surrounding analysis. §4
  is a record of the reasoning as it stood, and the same rule applied when
  `res.md`'s prose was deliberately left alone during the borrow→pointer purge
  (commit `a6f4bf282`): that document argues *about* the model, so editing its
  argument destroys the record. Phase 2's doc-sync task is an annotation only.

## Corrections

### C2 — §15.5's blockquote founded STATE soundness on the rule plan-59-E deletes (2026-07-20)

Phase 2 framed the blockquote rewrite as a wording fix. It was load-bearing. The
old text made `STATE` soundness rest on the escape rule *by name*:

> A **parameter** is a non-owning alias confined to the callee's frame (it cannot
> close, `RETURN`, or transfer the resource — **`TYPE_RESOURCE_INVALIDATE_NOT_OWNER`**,
> §15) … erasing STATE there ("opaque") is therefore unobservable.

plan-59-E retires `TYPE_RESOURCE_INVALIDATE_NOT_OWNER`. Had this paragraph been
left alone, §15.5 would have justified the `STATE` model with a rule that no
longer exists — and worse, the justification would have been *false*, since after
E a parameter **can** escape.

The rewrite re-founds it on what actually holds and is unaffected by E: a
resource's `STATE` type is fixed once at creation, carries no runtime tag, and
every declaration that *names* a type is checked against it. Opacity is sound not
because the alias cannot escape but because it is **unobservable** — `.state` is
inaccessible through a bare parameter, so nothing can read the payload under an
unchecked type.

This is the concrete form of the sub-plan's headline finding: `STATE` never
needed the escape rule, so §4's "Unresolved" was pessimistic. It also confirms
this sub-plan is a genuine **prerequisite** of E and not merely ordered before it.

### C3 — §15.5 gained the narrowing rule's own paragraph (2026-07-20)

Phase 3's doc task said "document the rule in … §15.5". Added as a closing
paragraph stating that opacity propagates as opacity and only a producer that
knows the type may name it, citing `TYPE_STATE_OPAQUE_NARROWING`.

Note the ordering hazard this created and how it was handled: the §15.5 paragraph
was written during Phase 2, *before* the rule existed in Phase 3, leaving a
dangling reference. `cargo test --bin mfb spec` passed anyway (48) because the
mention is prose, not a `[[…]]` citation — so the spec suite would **not** have
caught a permanently dangling rule name here. Phase 3 landed immediately after,
closing it. Worth knowing: prose rule names in the spec are unchecked.

### C4 — the rule needed provenance, not types: opaque and stateless are the SAME type string (2026-07-20)

§3 describes the rule as if it could be read off the types ("the checker knows
only that `f` has *some* state"). It cannot. A bare `RES f AS File` parameter and
a genuinely stateless `File` have **identical type strings**, so
`state_type_name` returns `None` for both and the existing agreement arms treat
an opaque value as provably stateless — which is precisely the laundering the
rule exists to stop.

The distinction is available only from **provenance**: is this value a read of a
bare `RES` *parameter*? So the implementation adds `current_opaque_params`
(`src/ir/verify/mod.rs`), populated per function alongside `current_owners`, and
a narrow predicate `is_opaque_state_value`.

The predicate is deliberately **only a direct local read**. Anything that has
passed through a call carries that call's declared return type, which names its
`STATE` or names none and is checked on its own terms. Widening it to follow
dataflow would be the whole-program aliasing analysis §3 and `res.md` §3.3
reject, and it would buy nothing.

### C5 — the rule was SILENTLY FILTERED until relocated; the return arm looked dead (2026-07-20)

Worth recording because it produced a wrong intermediate conclusion, and the
failure mode is invisible.

`ir::verify` maintains a list of rules it solely implements
(`src/ir/verify/mod.rs:125-150`); a rule **absent from that list is emitted and
then filtered out by the source path**, surfacing only via the package path with
no `file:line`. The first test of the return arm therefore printed
`TYPE_RETURN_MISMATCH` and `TYPE_RESOURCE_INVALIDATE_NOT_OWNER` but *not* the new
rule — indistinguishable from the rule never firing, and the natural reading was
"the return arm is unreachable because the escape rule catches it first".

That reading was wrong. After adding `TYPE_STATE_OPAQUE_NARROWING` to the list
beside `TYPE_STATE_MISMATCH`, the return arm fires, first, with a correct span:

```
src/main.mfb:9 error[2-203-0133 TYPE_STATE_OPAQUE_NARROWING]: …
src/main.mfb:9 error[2-203-0041 TYPE_RETURN_MISMATCH]: …
src/main.mfb:9 error[2-203-0086 TYPE_RESOURCE_INVALIDATE_NOT_OWNER]: …
```

**Generalisable lesson for the rest of plan-59:** a new `ir::verify` rule that
"doesn't fire" should be checked against this list *before* concluding anything
about reachability. plan-59-E adds no rule but plan-59-B's guard reporting and
bug-374's future work both touch diagnostics.

### C6 — `state-opaque-narrow-return-invalid`'s golden will churn in plan-59-E (2026-07-20)

The return fixture's golden currently records three errors, one of which is
`TYPE_RESOURCE_INVALIDATE_NOT_OWNER` (2-203-0086) — a rule **plan-59-E
deletes**. When E lands, this golden must be re-synced and will legitimately lose
that error.

Flagged here so E does not read the change as a regression, and so the re-sync is
a known expected delta rather than a judgement call. The fixture's *own* subject
— that `2-203-0133` fires on the narrowing — is unaffected by E and must survive
the re-sync; if it does not, that is a real regression in C's rule, not churn.

### C7 — a second trigger for bug-373, found incidentally (2026-07-20)

While building C's positive fixture, a program that declares `RES` parameters of
type `File` and imports `fs` but never *calls* an `fs::` function fails with the
same internal error bug-373 records:

```
error: NIR declares unused runtime helper 'fs'
```

So bug-373's trigger is broader than "a user resource shadows a built-in name":
it is **any program where a built-in resource helper is declared but no call
resolves to it**. Recorded in this plan rather than silently worked around; the
fixture avoids it by making a real `fs::open`/`fs::writeAll` call. bug-373's
Root Cause section should be widened accordingly when it is worked.

**Correction (2026-07-21, while working bug-373).** The diagnosis above is
wrong in its specifics, though right that a second trigger exists. A `RES`
parameter of type `File` declares no helper: `required_helpers` walks only
`function.body` (`src/target/shared/runtime/usage.rs:129-131`) and never
inspects params. The program described here builds clean; verified directly.
What actually triggers it is a local **`Bind`** of a built-in resource type —
`RES g AS File = f` — via `usage.rs:142-147`. C's fixture is therefore immune
for a different reason than stated: it has no aliasing rebind, not merely
because it makes a real `fs::` call.

The underlying defect is not the helper bookkeeping at all: codegen emits a
close for such a rebind, so it closes the *caller's* resource at the callee's
scope exit (`7-703-0004`, exit 255). Filed as **bug-375**; the internal error
is the compile-time barrier in front of it, not a false positive.

### C1 — case (c) is empty, checked two ways (2026-07-20)

§2's UNVERIFIED property — "whether anything legitimate needs opaque → concrete"
— is discharged. The narrowing has exactly two source-level routes, and both were
searched independently rather than inferring one from the other:

**Route 1, a stateful return from a bare-param function.** Zero:

```
$ grep -rhoE "^\s*(FUNC|SUB) [A-Za-z_]+\([^)]*RES [^)]*\) AS RES [A-Za-z_]+ STATE" \
    --include="*.mfb" bindings/ tests/ src/builtins/ | wc -l
0
```

No function in the tree takes a `RES` parameter and returns a resource carrying a
concrete `STATE` — which is the `launder` shape §3 is built around.

**Route 2, a stateful binding initialised from a bare parameter.** Zero. Listing
the initialiser of every `RES x AS T STATE S = …` binding in the tree returns
producer calls and nothing else — `fs::openFile`, `fs::createTempFile`,
`sql::open`, `snd::openFile`, `sndLink::open`, `openTagged`, `openSound`. Every
one is a function that *creates* the resource and therefore knows its `STATE`
type. Not one is a parameter.

So the design's assumption holds: opacity is only ever consumed as opacity today,
and forbidding the narrowing breaks nothing in-tree. Phase 3 may proceed as
written.

**Worth recording for whoever revisits:** this is a statement about the tree as
of 2026-07-20, not a proof that the pattern is unreasonable. The rule Phase 3
adds is what converts "nobody does this" into "nobody can" — which is the point,
since under plan-59-E a parameter may escape and route 1 becomes expressible for
the first time.

## Summary

This sub-plan is mostly a correction to the spec plus one small rule. The real
finding is that `res.md` §4's "Unresolved" was pessimistic: `STATE` never needed
the escape rule, so Track B does not require re-deriving the `STATE` model.

Untouched: `TYPE_STATE_MISMATCH`, the bare-parameter opt-out, and every close op.
No runtime machinery is added.
