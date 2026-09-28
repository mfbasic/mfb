# plan-149-B: Global, STATE and by-ref-capture owners

Last updated: 2026-09-22
Effort: medium (1h–2h)
Depends on: plan-149-A (Prerequisites are in plan-149-A)

B widens the place-copy-forwarding row (`src/optimizer/opt1/placefwd.rs`, from A) to three more owner classes:
globals, a resource's `STATE`, and by-ref-capture locals (plan-145-H's `Ref` / `RefField` sites). What these three
share is that something other than a visible `Assign` can write them, so each window must also be **call-safe**.

**The single outcome:** at `-O1`, `LET cur = g.xs; g = WITH g { xs := OP(cur, …) }` and
`LET cur = f.state.xs; f.state.xs = OP(cur, …)` take the plan-145 in-place path, with output identical to `-O0`. A
window containing a call that could write the owner keeps the copy.

## Prerequisites

See plan-149-A. In addition: plan-149-A's phases are all ticked, with their `Commit:` lines filled
(`grep -c '^- \[ \]' planning/plan-149-A-*.md` → 0).

## 1. Goal

- Global, `STATE` and by-ref-capture owners are forwarded under W1–W3 plus the call gate (W4). The unit, codegen and
  run-time tests show each forwarded shape and each declined one.

### Non-goals

These are the same as plan-149-A's. In particular, no codegen change and no `-O0` behaviour.

## 2. Current State

- **The NIR shapes** are in plan-149-A §2:
  - global: `MemberAccess{Global{g}, …}` written by `StoreGlobal{g}`;
  - `STATE`: `MemberAccess{MemberAccess{Local(f), "state"}, …}` written by `StateAssign{resource: f}`, where `f` is in
    `function.resource_owners`;
  - by-ref capture: a local bound from `Capture{by_ref: true}` (`nir/mod.rs:291`), whose reads and writes go through
    the parent's slot.
- **User calls vs builtins:** a user call's target is a name in `module.functions` (plan-149-A §2). A-A1 records the
  indirect-call and lambda-argument shapes. **B's gate uses what A1 recorded.** If A1 found a shape this section does
  not cover, fold it into W4 below before coding.
- **The global census precedent:** `src/optimizer/opt1/plans/globals.rs:census`, which counts every global's reads and
  writes. It is not reused for the window, which needs *position* rather than counts, but it shows the initializer-store
  exception (bug-552) that the window scan does not face, because a function never contains another function's
  initializer.

## 3. Design

**W4 — the call gate** (global, `STATE` and by-ref-capture owners). Ops `i+1..=L`, *including op L's value*, contain
none of the following:

- a `Call` or `CallResult` whose target is in `module.functions` (a user function can write a global, or reach a
  by-ref slot through another closure);
- any call with a function-valued argument (`FunctionRef`, `Closure`, or a `Local` or `Capture` whose type is `Func`):
  the callback can write;
- an indirect call, in the NIR shape A1 recorded.

Builtin package calls (`collections.*`, `len`, and the like) are allowed: runtime helpers do not write user globals,
slots, or `STATE` payloads.

**Why op L's value is included.** Consider `g = WITH g { xs := set(cur, j, helper()) }`. Originally `cur` was taken
before `helper()` ran. After the forward, `g.xs` is read when the argument is evaluated. W4 declines rather than depend
on argument order and on how the in-place arm orders its reads.

**W5 — the resource gate** (`STATE` only). No bare `Local(f)` value (not under a `MemberAccess`) appears in the window.
Passing the resource to anything, including `fs::close(f)`, declines. `StateAssign{f}` is the owner write, as W1/W2
treat it.

**The owner write per class, for W1/W2:**

| Class | Owner write |
|---|---|
| global | `StoreGlobal{g}` |
| `STATE` | `StateAssign{f}` |
| by-ref capture | `Assign{o}` |

A global owner needs no census gate beyond W4, because it cannot be `LocalRef`'d, captured, or rebound.

**Risk:** W4 is the proof that nothing *invisible* writes the owner. It is deliberately coarse: any user call declines.
The benchmark rows contain only builtin calls (`collections::get` / `set` / `remove` / `removeAt` / `removeKey`, and
`toString` in the Dynamic rows), so the coarse gate still reaches all 16.

## Phases

> **NOTE — keep the checkboxes current as you go** (see plan-149-A). **An unticked box means NOT DONE.**

### Phase B1 — Global and by-ref-capture owners

- [x] In `placefwd.rs`, accept a `MemberAccess` chain rooted at `Global{g}`, or at a local bound from
      `Capture{by_ref: true}`. Add W4, with the owner writes from the §3 table.
      Done: `Owner::Global` / `Owner::ByRef`, `CallGate` (W4), `ModuleFacts::may_hold_function`. A bare `Global{g}`
      source is accepted too (plan-149-A §3 lists `Global{o}` as a place). The function-valued-argument rule is
      implemented as "no function value anywhere in the window" (Corrections).
- [x] Unit tests. These **forward**:
      - the global `set` shape;
      - the global `removeAt` shape;
      - a by-ref capture's field `set` shape.

      These **keep the copy**:
      - a user call in the window;
      - a user call inside op L's value;
      - a `collections::` call with a lambda argument in the window;
      - an indirect call in the window;
      - a `StoreGlobal{g}` between the bind and a read.

      Done, plus two not listed: a by-ref capture with a user call in the window declines, and an owned local still
      forwards across a user call (W4 is only for owners a call can reach).

Acceptance: `cargo test --bin mfb optimizer::opt1::placefwd` → all pass, including the new cases (est. 5 min).
Verified 2026-09-27: 29 passed.
Commit: —

### Phase B2 — `STATE` owners

- [ ] Accept a chain rooted at `MemberAccess{Local(f), "state"}` where `f` is in `function.resource_owners`. The owner
      write is `StateAssign{f}`. Apply W4 and W5.
- [ ] Unit tests. These **forward**:
      - the `list (State-Fixed) set` shape (`get(cur)`, then `f.state.xs = set(cur, …)`);
      - the `removeAt` shape;
      - the `removeKey` shape;
      - the `remove` shape.

      These **keep the copy**:
      - `fs::close(f)` in the window;
      - `f` passed to a user `SUB` in the window;
      - a `StateAssign{f}` between the bind and a read.

Acceptance: `cargo test --bin mfb optimizer::opt1::placefwd` → all pass (est. 5 min).
Commit: —

### Phase B3 — Codegen and run-time proof for B's classes

- [ ] Extend `tests/codegen/codegen_place_forward.rs` to cover the global `set`, the `STATE` `set` and the `STATE`
      `removeAt` shapes. At `-O1` it asserts the plan-145 store slots: `global_field_inplace` / `state_field_inplace`
      (or the collection arm's slot that plan-145 records for that site) are present, and `with_target` is 0. At `-O0`
      the rebuild is present.
- [ ] Extend `tests/runtime/rt_place_forward.rs` to cover every B forwarding shape, plus the W4 and W5 negatives as
      programs (a user `SUB` that writes the global, called in the window, where the old value must be observed). It
      asserts stdout is identical at `-O0` and `-O1`.

Acceptance: `cargo test --test codegen_place_forward --test rt_place_forward` → all pass (est. 5 min).
Commit: —

## Corrections

- **W4's "function-valued argument" rule is widened to "any function value in the window" (B1).** An argument's type
  is not in NIR: a `MemberAccess` or a builtin's result (`collections::get(fs, 0)` on a `List OF FUNC …`) can be a
  function with no `Func`-typed binding in sight. The gate therefore declines when the window holds a `FunctionRef`,
  a `Closure`, or any `Local` / `Capture` / `Global` whose type can hold a function (`ModuleFacts::may_hold_function`,
  which walks record and union members through `module.types`; an unresolvable user nominal answers yes). A function
  value can only enter the window through one of those, so this is sound where a per-argument test is not. The
  benchmark rows hold none, so the coverage the plan counts on is unchanged.
- **The indirect-call test (B1).** Per plan-149-A §2, an indirect call is `Call{target: <binding name>}` and an
  unqualified target can also be a builtin (`toString`). The gate declines a call whose target is a user function,
  a global, or any name bound in the function (a `Bind`, a parameter, or a `FOR` / `FOR EACH` / `TRAP` binder — the
  census now records the binder types too, so a `FOR EACH fn IN fs … fn(x)` call is caught).
- **Unit-test modules declare their record types (B1).** The gate resolves members through `module.types`, which a
  real module always fills; the tests' `module_of` declares `R` and `Inner` so a case declines for its own hazard,
  not because `R` is unresolvable.

## Summary

W4 carries B's risk: it must catch every way a call can write the owner. It declines on any user call, any callback,
and any indirect call, and B3's run-time negatives check that the old value is still observed when such a call writes.
