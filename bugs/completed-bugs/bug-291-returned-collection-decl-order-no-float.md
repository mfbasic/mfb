# bug-291: a returned collection declared *after* its resource silently gets no ownership float → returned closed handles + double-close

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness (resource lifetime)

Status: Fixed
Regression Test: tests/syntax/resources/resource-return-collection-order-invalid (rejection) + tests/rt-behavior/resources/resource-return-collection-order-rt (supported order)

Escape analysis floats a resource's ownership to a returned collection only when
the collection is declared strictly *before* the resource (phase 1's `order >=
res_order` skip; phase 2's `depth >= res_depth` skip). So
`RES f = open(...); MUT xs = []; xs = append(xs, f); RETURN xs` yields `f → Local`:
`f` is closed at the function's exit while the returned list still carries it, and
the caller-side owned list later attempts a second close. §15.6 promises that a
collection escaping via RETURN moves ownership out to the caller — but nothing
(resolver, `ir::verify`) rejects the declaration order the analysis cannot honor, so
the program compiles and then double-closes at runtime.

The single correct behavior a fix produces: returning a collection that holds a
resource is either honored regardless of the relative declaration order of the
resource and the collection, or rejected at compile time with a clear diagnostic —
never silently miscompiled into a double-close.

References:

- `src/docs/spec/language/15_resource-management.md` §15.6 (RETURN moves ownership
  to caller).
- `src/docs/spec/architecture/23_escape-analysis.md` (phase-1 order rationale).
- Found during goal-06 review of `src/escape.rs`.

## Failing Reproduction

```
FUNC openThem() AS List OF File
  RES f = fs::openFile("/tmp/x", "w")
  MUT xs AS List OF File = []
  xs = collections::append(xs, f)
  RETURN xs
END FUNC
' caller: LET items = openThem() : io.print("opened=" & toString(collections::count(items)))
```

- Observed: at runtime `Error: 7-703-0004 Resource handle is already closed.` plus
  `Cleanup failure: 7-703-0004` (double close), exit 255.
- Expected: prints `opened=1`, exit 0.

Contrast (correct today): declaring `MUT xs` *before* `RES f` prints `opened=1` and
exits 0 — the only change needed.

## Root Cause

`src/escape.rs:314-361` (`solve`): phase 1 only floats to a returned collection
declared strictly before the resource; phase 2 requires a strictly-outer scope. A
returned collection declared after its resource matches neither, so the resource
stays `Local` and is closed at function exit despite escaping in the returned list.

## Goal

- Either: lowering creates the returned collection's owned-list early enough that
  the float is honored regardless of declaration order; or: the resolver / `ir::verify`
  emits a compile-time diagnostic when a resource is a member of a returned
  collection declared after it ("declare the collection before the resource").

### Non-goals (must NOT change)

- The correct behavior when the collection is declared before the resource.
- The §15.6 caller-ownership-transfer semantics.

## Blast Radius

- `escape.rs:solve` (+ possibly lowering or a new verify rule) — fixed here.
- Sibling: bug-290 (inline-TRAP insertion invisible) is a distinct escape root
  cause. Confirm no other "float requires ordering" assumptions elsewhere.

## Fix Design

Preferred long-term: make lowering allocate the owned-list for a returned collection
before the resource is produced, so the float is order-independent. Cheaper interim:
add a compile-time diagnostic for the unsupportable order (turns a silent
double-close into a clear error). Recommend the diagnostic first (unblocks
correctness/safety), then the lowering improvement. Rejected alternative: leaving it
silent — a double-close is a memory-safety-adjacent runtime failure.

## Phases

### Phase 1 — failing test
- [ ] rt-behavior test of the repro; confirm double-close today. Add the contrast
      (before-order) case as a guard.
### Phase 2 — the fix
- [ ] Diagnostic (interim) and/or order-independent owned-list allocation.
### Phase 3 — validation
- [ ] Full suite green; both declaration orders behave correctly (or the bad order
      is cleanly rejected).

## Validation Plan

- Regression: the rt-behavior repro + before-order contrast.
- Runtime proof: no double-close.
- Doc sync: 23_escape-analysis.md (and 15_resource-management.md if a diagnostic is
  added).

## Summary

Escape analysis honors the resource→returned-collection float only in one
declaration order and silently miscompiles the other into a double-close. Fixing the
ordering assumption (or rejecting it) closes a runtime memory-safety-adjacent
failure; the real engineering choice is diagnostic-vs-lowering.

## Resolution — the diagnostic, as the report recommended

The report offered two fixes and recommended the diagnostic first. That is what
landed, and the choice was checked rather than assumed: the ordering constraint was
tested by deleting the `order >= res_order` skip and rebuilding. Lowering then fails
with `resource floats to 'xs', which has no owned-list while lowering bind f AS File`
— so the constraint is real, not stale conservatism. The owned-list is created at
the collection's *bind* site (`setup_owned_list`, from `builder_control`), so
honouring the other order means hoisting that allocation, which changes the scope
the drain obligation lives at. That is the report's "preferred long-term" fix and it
remains open; it is a lowering change, not an escape-analysis one.

What is closed is the memory-safety-adjacent part: the program no longer compiles.

- `ResOwner` gains a `FloatBlocked(collection)` variant. The unsupportable case
  previously collapsed into `Local`, which is exactly what made it silent — `Local`
  is also the correct answer for a resource that legitimately does not escape, so
  the two were indistinguishable downstream. Modelling it separately costs nothing
  and makes it rejectable.
- `solve` records it when phase 1 skipped a returned collection that genuinely holds
  the resource and phase 2 then found no outer collection either.
- `ir::verify` emits `TYPE_RESOURCE_RETURN_ORDER` (`2-203-0131`), naming both
  bindings and the order that fixes it. ir::verify is the sole implementer: the
  condition is knowable only from escape analysis' decision, which syntaxcheck does
  not compute.
- The variant is unreachable on the wire — verification rejects the program before
  it can be encoded — so `ir::binary` writes it as `Local` and the `.mfp` format
  stays v4-compatible rather than gaining a tag no reader could legitimately see.

### The spec already required this

§15.6 already said "the resources must be added to the collection at or after the
collection's own binding so the obligation rides the collection." The rule was
stated and simply never enforced, so this is the compiler catching up to its own
contract rather than a new restriction. §15.6 now cites the rule code, and
`diagnostics/01_rule-codes.md` lists it — the latter because
`every_rule_is_documented_in_the_spec` failed and caught the omission, which is the
guard working as intended.

### Verification

Two fixtures, deliberately paired: the syntax test pins the rejection (with the
diagnostic text in its golden), and the rt-behavior test pins that the *supported*
order still builds and runs, so the new rule cannot creep into rejecting valid
programs. Both use a returned handle rather than counting the list — the count alone
reports success while the handles are already closed, which is how this class of bug
hides.

Full `cargo test` green; acceptance 1004/1004.

### The first version of the rule was over-broad — two existing tests caught it

Full acceptance failed two pre-existing fixtures,
`syntax/resources/resource-collection-return-invalid` and
`syntax/resources/native-resource-in-list-invalid`. Both declare a **bare**
`List OF File` / `List OF Db` returned holding a resource, and both already assert
a `TYPE_RESOURCE_REQUIRES_RES` rejection for the missing `RES` marker. The new rule
piled a third error on top of them.

Those goldens were not regenerated, because the tests were right and the rule was
wrong. A bare `List OF File` cannot own a resource at *any* declaration order, so
"declare the collection before the resource" is advice that would not fix either
program — it is noise on an already-correct rejection, and following it would leave
the author no better off.

The rule is now gated on the collection's declared type actually carrying the `RES`
ownership axis. That required escape analysis to start recording declared types
(`decl_type`), which it had no reason to before. Both fixtures are green with their
original goldens intact.
