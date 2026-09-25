# plan-156-E: per-view STATE on `Thread` and `ThreadWorker`

Last updated: 2026-09-24
Effort: medium (1h–2h)
Depends on: plan-156-D

Each view is its own resource record (C), so each carries its own STATE.
The parent declares it on the binding:

```basic
RES t AS Thread OF M TO O STATE P = thread::start(...)
```

The worker declares it in its entry function's signature:

```basic
worker AS ThreadWorker OF M TO O STATE W
```

The two are independent. Neither is shared across threads, and they need not
agree, unlike a resource plane's STATE, where both sides must name the same
type. This fixes bug-693: today a thread type's trailing `STATE` binds to the
**output** type and crashes the compiler on a state write.

**Checkable outcome:**
- bug-693's reproduction builds and prints 5.
- A parent writes and reads `t.state.n` while its worker writes and reads its
  own `worker.state.n`, and each side sees only its own values.

References: spec §15 (STATE), §16; bug-693; plan-156-A (settled semantics).

## Prerequisites

See plan-156-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-156-D complete | `ls planning/plan-156-D-*` → no match | NOT MET |

## 1. Goal

- A thread type's trailing `STATE X` attaches to the handle (the view).
- Each view's STATE is default-initialised with its record, read and written
  in place, and freed with the view.

### Non-goals

- STATE on the resource-plane element (`Thread OF RES fs::File STATE Cursor TO O`)
  keeps its current meaning and agreement rule.
- STATE on `O` as a data type is not a thing: data types carry no STATE.

## 2. Current State

- `src/types.rs:ParameterType::with_state` on a `ThreadHandle` puts STATE on
  `out`. `Stateful{base,state}` is built only for a bare-token base
  (`split_state_clause`). A probe of `Thread OF M TO O STATE P` fails with
  `TYPE_BINDING_MISMATCH` (2026-09-24).
- Stale doc comments: the `with_state` doc ("STATE is not a variant") and the
  `is_defaultable` doc ("`parse` has no STATE arm").
- The parser reads a parameter's STATE only after `RES`
  (`src/ast/items.rs:parse_params` → `parse_optional_state`). So
  `worker AS ThreadWorker … STATE W` without `RES` is
  `MFB_PARSE_UNEXPECTED_TOKEN`.
- Thread type parsing: `src/ast/expr.rs:parse_thread_type_name`,
  `parse_resource_plane_type`. Grammar: `src/docs/spec/language/19_grammar.md`
  `threadBody`, which has no STATE on a thread type.
- STATE agreement checks: `calls.rs:check_argument_state_agreement`,
  `check_return_state_declaration`, `check_binding_state_agreement`
  (`ops.rs` Bind arm), `check_thread_transfer_state`,
  `link.rs:check_link_state_agreement`.

## 3. Design

- **Parse.** In a `Thread`/`ThreadWorker` type, a `STATE X` after the `TO O`
  part produces `Stateful{ base: ThreadHandle{…}, state: X }`. The STATE is
  the view's, not `out`'s.
- **Parameters.** A `ThreadWorker` parameter may carry `STATE` without the
  `RES` keyword. A resource parameter needs no `RES` (verified 2026-09-24), so
  gating STATE on `RES` is the anomaly. Extend `parse_optional_state` to
  resource-typed parameters.
- **Agreement.** The parent's STATE agrees across its bindings, parameters and
  returns like any resource. The worker's is declared once, in its entry
  signature, and `thread::start` does **not** compare the two.
- **Storage.** Each view record's `RESOURCE_OFFSET_STATE` slot, default-built
  at view creation (`thread::start` for the parent, the trampoline for the
  worker), freed at view drop or close.
- **Diagnostics.** A state assignment whose STATE type is unresolved reports a
  rule-coded diagnostic instead of the internal error in bug-693.

## Phases

> Keep checkboxes current (see plan-156-A's note). **An unticked box means NOT
> DONE.**

### Phase 1 — failing tests

- [ ] `tests/rt-behavior/threads/bug693-worker-state/`: bug-693's
      reproduction, expecting 5.
- [ ] `tests/rt-behavior/threads/thread-view-state-independent/`: parent and
      worker STATE, each side printing only its own values.

Acceptance: both fail (an internal error and `TYPE_BINDING_MISMATCH`).
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/threads/bug693-*' 'rt-behavior/threads/thread-view-state-*'`
  → 2 failures (est. 1 min).
Commit: —

### Phase 2 — parse, type, store

- [ ] `src/types.rs` and `src/ast/expr.rs`: the §3 parse rule; fix the two
      stale doc comments.
- [ ] `src/ast/items.rs:parse_params`: STATE on resource-typed parameters.
- [ ] Agreement per §3. The view-record STATE lifecycle is in
      `thread::start`, the trampoline and the drop.
- [ ] Grammar: `19_grammar.md` `threadBody` gains the STATE clause.

Acceptance: both Phase 1 fixtures pass; existing STATE tests pass.
  Check: the Phase 1 `test-accept.sh` command → 0 failures (est. 1 min);
  `cargo test --bin mfb state` → pass (est. 2 min).
Commit: —

## Validation Plan

- Tests: the two Phase 1 fixtures, plus a verifier unit test that
  `thread::start` does not compare the parent's and the worker's STATE.
- Doc sync: `19_grammar.md`. The §15 and §16 prose is written in F.
- Final gate: at the end of F.

## Corrections

## Summary

A parse rule and a storage slot. The risk is the grammar change: `STATE` after
a thread type used to bind to `out`. No test or corpus file relies on that,
since it crashed (bug-693); confirm with
`rg -n 'Thread(Worker)? OF .*TO .* STATE' tests examples src/docs` before
Phase 2 and record the count in Corrections.
