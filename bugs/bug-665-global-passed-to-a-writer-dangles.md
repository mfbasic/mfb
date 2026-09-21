# bug-665: a global passed to a function that reassigns it leaves the parameter dangling

Last updated: 2026-09-20
Effort: large (3h–1d)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: tests/runtime/rt_global_argument_reassigned_by_callee.rs (to be added, Phase 1)

A module-level collection `g` passed as an argument to a FUNC/SUB that
reassigns `g` crashes the program (`Error: 7-701-0001 Allocation failed.`) or
reads garbage. The callee's parameter holds a pointer to `g`'s block (arguments
are borrowed, not copied); the callee's `g = …` lowers through `NirOp::StoreGlobal`,
which frees `g`'s old block; the parameter now points at freed arena memory.

**The single correct behavior a fix produces:** the parameter keeps the value
`g` had at the call, whatever the callee does to `g`. The program below prints
`param: alpha gamma len=3`, then `3`, then `g len=5`.

References:

- `mfb spec language memory-semantics` §14 (values are copied on bind; a
  parameter is a value).
- Found by the plan-142 research for in-place global updates
  (`planning/plan-142-A-*.md`), which lists this bug as a prerequisite.
- Sibling: bug-666 (the same free, reached from a `FOR EACH` over the global).

## Failing Reproduction

```basic
IMPORT collections
IMPORT io

MUT g AS List OF String = ["alpha", "beta", "gamma"]

SUB clobber(xs AS List OF String)
  g = collections::append(g, "delta")
  g = collections::append(g, "epsilon")
  MUT pad AS List OF String = ["zzzzzzzzzzzzzzzzzzzzzzzzz", "yyyyyyyyyyyyyyyyyyyyyyyy", "xxxxxxxxxxxxxxxxxxxxxxx"]
  io::print("param: " & collections::get(xs, 0) & " " & collections::get(xs, 2) & " len=" & toString(len(xs)))
  io::print(toString(len(pad)))
END SUB

SUB main()
  clobber(g)
  io::print("g len=" & toString(len(g)))
END SUB
```

`mfb build . && ./build/probe.out` with the release compiler built from
`b6a10efbc` (macOS aarch64):

- Observed: `Error: 7-701-0001` / `Allocation failed.` — nothing printed.
- Expected: `param: alpha gamma len=3`, `3`, `g len=5`.

Contrast (works today): the same program with `LET c AS List OF String = g`
then `clobber(c)` prints `param: alpha gamma len=3`, `3`, `g len=5`. The only
difference is that the argument is a local copy, not the global itself.

| Environment | Result |
| --- | --- |
| macos-aarch64, release, `b6a10efbc` | fails ✗ |
| other targets | UNVERIFIED (the lowering below is target-generic, so expected to fail) |

## Root Cause

- Reading a global yields its block pointer, not a copy:
  `NirValue::Global` lowering, `src/codegen/engine/value/builder_values.rs:1764-1792`.
- Call arguments are lowered with plain `lower_value` and passed as-is:
  `emit_prepared_call_args_hooked`,
  `src/codegen/engine/builder/builder_emit_helpers.rs:102-156` (`:113`). The
  callee prologue stores the incoming pointer into the parameter slot
  (`src/codegen/engine/function/function_lowering.rs:1032-1062`) without copying.
- The callee's `g = …` is `NirOp::StoreGlobal`
  (`src/codegen/engine/control/builder_control.rs:1060`), which frees the global's
  previous block (`:1097-1119`, bug-47) — the block the parameter still points at.
- The existing guard for this free, bug-496's operand snapshot
  (`src/codegen/engine/value/operand_snapshot.rs:4-18`), protects only an operand
  from a *later sibling operand of the same expression*; it returns early for calls
  with fewer than 2 args (`:62-68`) and never snapshots the last operand, so a
  lone `f(g)` is not covered.

## Goal

- The reproduction prints the expected three lines, on every target.
- Any call argument whose value is (or is read out of) a module-level global
  `g`, passed to a callee that can transitively reach a `StoreGlobal` of `g`, is
  not freed while the callee's parameter can still read it.

### Non-goals (must NOT change)

- Arguments stay borrowed in the common case: a call that cannot reach a write of
  the global must not start copying (no regression for `len(g)`, `f(g)` where `f`
  only reads).
- `StoreGlobal`'s old-block free stays (removing it reintroduces the bug-47 leak).
  "Fix" by leaking the old block is forbidden.
- No change to the language: passing a global stays legal.

## Blast Radius

Found by reading, not yet by running each (Phase 1 runs them):

- User FUNC/SUB call with a global argument, callee writes it — **fixed by this bug**.
- A builtin with a callback: `collections::forEach(g, LAMBDA(v AS String) -> g = …)`,
  or `filter`/`transform`/`reduce`/`sortBy`/`mapValues`/`groupBy`/`partition`/`any`/`all`/`findIndex`
  with a callback that writes `g` while the builtin walks `g`'s block —
  **fixed by this bug** (same free, same borrowed argument).
- A method-style call through a `FUNC` value (`LET f = clobber` then `f(g)`) —
  **fixed by this bug**; the reachability walk must fail closed on opaque calls.
- `FOR EACH v IN g` with a write to `g` in the body — **bug-666**, not here.
- A `get`-borrow binding rooted in a global — UNVERIFIED whether the borrow-get
  gate (`function_lowering.rs:411-416`, keyed on locals) can ever admit one;
  Phase 1 answers it.

## Fix Design

At a call site, before lowering each argument: if the argument reads a
module-level global `g` (a bare `Global`, or a member/element read rooted in one)
and the callee — or any callback argument of a builtin — can reach a
`NirOp::StoreGlobal` of `g`, lower that argument owned (`lower_value_owned`,
a deep copy into a statement-scope temporary the parameter then borrows).

"Can reach" reuses the G25 walker (`src/codegen/collection/assign/inplace_dest.rs:894-1010`,
`call_target_reaches_state_assign`) with the leaf swapped from `NirOp::StateAssign`
to `NirOp::StoreGlobal { name == g }`: memoized DFS through `self.functions`,
failing closed for symbols it cannot see and for any call passed a `FUNC` value.

Rejected:
- **Callee copies every parameter** — a copy on every call of every function; the
  common case pays for the rare one.
- **Defer `StoreGlobal`'s free while any frame borrows `g`** — needs runtime
  borrow tracking, and a deferred free is a leak by another name.
- **Reject the program** — passing a global to a function that updates it is
  ordinary code.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_global_argument_reassigned_by_callee.rs` (plus its
      `[[test]]` stanza in `Cargo.toml`, checked by `tests/guards/test_targets_registered.rs`):
      the reproduction above (expects the three lines), the `forEach` callback
      variant, and the `FUNC`-value variant. Confirm each fails today.
- [ ] Settle the `get`-borrow row of the blast radius (read
      `function_lowering.rs:411-416`; if a global container can be admitted, add
      a case to the test).

Acceptance: `cargo test --test rt_global_argument_reassigned_by_callee` → every
case fails with the crash/garbage recorded above (est. 3 min).
Commit: —

### Phase 2 — the fix

- [ ] Generalize the G25 walker in `inplace_dest.rs` to take the leaf predicate
      (`StateAssign` for G25, `StoreGlobal { name }` here); keep G25's behavior
      identical.
- [ ] In `emit_prepared_call_args_hooked` (and the builtin-call argument path
      used by `abi_inline` lowerings with callbacks), lower an argument owned when
      it reads a global the call can reach a write of.

Acceptance: `cargo test --test rt_global_argument_reassigned_by_callee` → all
pass; `cargo test --test codegen_inplace_append_call_result` (G25 cases) still
passes (est. 5 min).
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate any `.ncode` goldens the new copies shift; each diff must be a
      call whose argument is a global written by the callee.
- [ ] Full suite: `cargo test` and `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

Acceptance: both green; every golden delta is a call site of that shape.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_global_argument_reassigned_by_callee.rs`.
- Runtime proof: the reproduction prints the expected lines.
- Doc sync: `mfb spec` memory-semantics section on parameters, if it states that
  arguments are never copied.
- Full suite: `cargo test`; `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Copy at the call site (recommended — only the rare writer-callee pays) vs. at
  the callee's first write of the global (needs to know which parameter aliases it).

## Summary

The risk is the reachability walk: a false "cannot reach" leaves the crash, a
false "can reach" only costs a copy, so it must fail closed.
