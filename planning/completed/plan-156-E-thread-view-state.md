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
| plan-156-D complete | `ls planning/plan-156-D-*` → no match | MET (2026-09-25: "no matches found"; archived) |

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

- [x] `tests/rt-behavior/threads/bug693-worker-state/`: bug-693's
      reproduction, expecting 5.
      (Today: "error: native code WITH target 'Unknown' is not a record while
      lowering state assign worker", no executable.)
- [x] `tests/rt-behavior/threads/thread-view-state-independent/`: parent and
      worker STATE, each side printing only its own values.
      (Parent `STATE P {n}`, worker `STATE W {n, label}` — different types.
      Expected `parent starts at 0`, `parent 7`, `worker 10`, `parent still 7`.
      Today: `MFB_PARSE_UNEXPECTED_TOKEN` at the worker's `STATE` without
      `RES`, reached before the binding's `TYPE_BINDING_MISMATCH` the plan
      predicted — Correction 1.)

Acceptance: both fail (an internal error and `TYPE_BINDING_MISMATCH`).
  Check: `bash scripts/test-accept.sh target/release/mfb $(mktemp -d) 'rt-behavior/threads/bug693-*' 'rt-behavior/threads/thread-view-state-*'`
  → 2 failures (est. 1 min).
  Result: "acceptance tests failed: 4 mismatch(es) (2 test(s) ran)" — both
  `build.log`s, plus bug-693's `.ast`/`.ir`, which exist once it parses and are
  recorded in Phase 2.
Commit: 75af2df32

### Phase 2 — parse, type, store

- [x] `src/types.rs` and `src/ast/expr.rs`: the §3 parse rule; fix the two
      stale doc comments.
      (`ParameterType::parse`'s thread arm lifts an output's own top-level
      `Stateful` onto the handle, `Stateful{ThreadHandle, S}`, and `with_state`
      builds the same, so `t.with_state(s) == parse(t STATE s)` still holds.
      `ast/expr.rs` needed nothing: `parse_type_name` stops before `STATE`, which
      the binding / parameter parsers read — Correction 2. Both doc comments
      fixed. Every structural thread match that could meet a stateful view now
      strips it: `builtin_resource_close_function`,
      `is_builtin_sendable_resource_type`, `resource_record_freed_at_drop`, the
      codegen handle-type helper and `thread.start`'s return type, the native
      plan's storage class, and the registry's `unify`, where a thread-handle
      pattern matches a stateful view by its handle — which is also why
      `thread::start` never compares the two views' STATE.)
- [x] `src/ast/items.rs:parse_params`: STATE on resource-typed parameters.
      (A parameter whose type spells `Thread OF …`/`ThreadWorker OF …` reads a
      `STATE` without `RES` — the thread types being the resource types the
      parser can tell apart by spelling.)
- [x] Agreement per §3. The view-record STATE lifecycle is in
      `thread::start`, the trampoline and the drop.
      (Agreement: the parent's is an ordinary resource's; the worker's is its
      parameter's; `unify` strips a view's STATE, so `thread::start` compares
      neither — pinned by `thread_start_does_not_compare_parent_and_worker_state`.
      Lifecycle — Correction 3: not in `thread::start` or the trampoline, which
      cannot know the STATE type. The parent's `RES … STATE P` binding runs the
      existing lazy `emit_resource_state_init`, and its drop frees it through the
      cleanup's `state_type`; a parameter declaring a thread view's STATE runs
      the same lazy init at function entry, which is where the worker's view
      gets its default, and the worker's STATE goes with its arena. `--debug` of
      `thread-view-state-independent`: `arena.0.live_bytes 0`,
      `thread.table_live 0`, `double_free_skips 0`.)
- [x] Grammar: `19_grammar.md` `threadBody` gains the STATE clause.

Acceptance: both Phase 1 fixtures pass; existing STATE tests pass.
  Check: the Phase 1 `test-accept.sh` command → 0 failures (est. 1 min);
  `cargo test --bin mfb state` → pass (est. 2 min).
  Result: `bug693-worker-state` → `5`; `thread-view-state-independent` →
  `parent starts at 0` / `parent 7` / `worker 10` / `parent still 7`; with the
  thread and resource sets → "acceptance tests passed" after recording the two
  fixtures' `.ast`/`.ir` (220 ran); `cargo test --release --bin mfb -- verify
  state` → 562 passed; full `cargo test --bin mfb` → 4327 passed; IR sweep over
  960 fixtures → 0 differ; `artifact-gate all` → 2152 checked, 0 diffs.
Commit: 30f7668a6

## Validation Plan

- Tests: the two Phase 1 fixtures, plus a verifier unit test that
  `thread::start` does not compare the parent's and the worker's STATE.
- Doc sync: `19_grammar.md`. The §15 and §16 prose is written in F.
- Final gate: at the end of F.

## Corrections

1. **The corpus count, and the second failure's reason.** The Summary's
   `rg -n 'Thread(Worker)? OF .*TO .* STATE' tests examples src/docs` finds 5
   files, all false positives: a resource-plane `STATE` (`RES X STATE S TO O`)
   or a second `STATE` later on the same line. Anchored to a `STATE` directly
   after the output type (`TO <Out> STATE`), the count outside the two new
   fixtures is 0, so nothing relies on the old binding to `out`. The
   independent-state fixture fails at parse, on the worker's `STATE` without
   `RES`, rather than on the binding's type.

2. **`src/ast/expr.rs` needed no change.** `parse_type_name` already stops before
   a `STATE` keyword; the binding and parameter parsers read the clause
   themselves and HIR attaches it with `with_state`. So the §3 parse rule lives in
   `ParameterType::parse` and `with_state` (`src/types.rs`), which is where the
   clause becomes the handle's.

3. **The STATE lifecycle is the binding's and the parameter's, not
   `thread::start`'s or the trampoline's.** Neither runtime helper knows the
   STATE type (one start helper and one trampoline serve every thread), so each
   would need per-type code. The existing lazy `emit_resource_state_init` does
   the same job where the type IS known: the parent's `RES … STATE P` binding
   runs it (as for any stateful resource), and a parameter declaring a thread
   view's STATE runs it at function entry — the worker entry's view arrives with
   none. The parent's drop frees its STATE through the cleanup's `state_type`;
   the worker's STATE is in the worker's arena, which the worker unmaps as it
   finishes (plan-156-B).

## Summary

A parse rule and a storage slot. The risk is the grammar change: `STATE` after
a thread type used to bind to `out`. No test or corpus file relies on that,
since it crashed (bug-693); confirm with
`rg -n 'Thread(Worker)? OF .*TO .* STATE' tests examples src/docs` before
Phase 2 and record the count in Corrections.
