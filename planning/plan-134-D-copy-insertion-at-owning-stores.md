# plan-134-D: Copy-insertion for recursive values at bind, assign, global, return and capture

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-C

Give a recursive value an independent copy at the five owning stores that go through
`lower_value_owned` / `lower_returned_value`, moving instead of copying where plan-134-C's
analysis proves the source is not read again. This is the half of bug-536 Shape C that makes
ownership true; it adds allocations and frees nothing, so it cannot corrupt memory. It fixes
bug-601's remaining row: a `MUT` copy of a `List OF Tree` stops sharing its source.

Behavioural outcome: **`MUT ys = xs` over a list of a recursive type, followed by in-place
appends to `ys`, leaves `xs` unchanged** (`tree_alias` prints `ys=6 xs=1`; today `xs=240`).

References: plan-134-A (§2, §3, Open Decisions: the speed budget); `bugs/bug-601-…md`;
`src/codegen/memory/owned.rs::materialize_owned_element` (bug-538 — the precedent this
mirrors: `!is_freeable_flat_value && type_reaches_cycle && !type_contains_resource` →
`copy_value_to_current_arena`).

Prerequisites: see plan-134-A; plan-134-C complete (`ls planning/completed/plan-134-C-*`) — MET
2026-09-13 (`planning/completed/plan-134-C-last-use-move-analysis.md`).

## 1. Goal

- `tree_alias` prints `ys=6 xs=1`.
- `node_copies`' `LET c AS Node = a` emits a copy (one `_mfb_thread_copy_*`/walker call from
  `main`, today 0); stores whose source is a last-use local emit none.
- `json_repeat` K=1 and `regex_repeat` K=1 wall time within the plan-134-A budget (recommended
  ≤ +25 % over §2.1, median of 5).

### Non-goals

- No frees (letters F–H). Memory use may rise by the copies until G lands; the leak tests stay
  where they are.
- Construction stores (record fields, `WITH`, union wrap, collection payloads) are letter E.
- Flat values: byte-identical.

## 2. Current State

- `lower_value_owned` (`src/codegen/engine/value/builder_values.rs`) — 5 callers (plan-134-A
  §2.1): the `Bind` arm, the `Assign` arm, `NirOp::StoreGlobal`, closure capture
  (`lower_value_owned(capture)`), and one more; each copies only when `value_needs_owning_copy
  && is_freeable_flat_value`.
- `lower_returned_value` (`src/codegen/engine/control/builder_exits.rs`) — copies an aliasing
  source only when `is_freeable_flat_value`; `plan_returned_move` needs an `OwnedValue` cleanup
  and so never moves a recursive local today.
- `value_needs_owning_copy` is true for `Local`, `Global`, `Capture`, `MemberAccess`,
  `UnionExtract`, `ResultValue`/`ResultError`, static strings, rodata-string calls and
  param-borrow calls.
- In-place arms (`inplace_dest.rs::admits_with`) never check ownership; they become sound for
  the class once every `MUT` owns its graph.

## 3. Design

- **One predicate.** Add `CodeBuilder::needs_graph_copy(type_) -> bool` =
  `!is_freeable_flat_value(type_) && type_reaches_cycle(model, type_) &&
  !type_contains_resource(model, type_)`, and make `materialize_owned_element` use it too (one
  definition of the class).
- **`lower_value_owned`.** After the flat-copy branch: if `value_needs_owning_copy(value) &&
  needs_graph_copy(type)` and the store is not in the function's `MoveSites` (plan-134-C), copy
  with `copy_value_to_current_arena`. A move site returns the pointer unchanged (today's
  behaviour, now proven safe).
- **`lower_returned_value`.** Same branch for an aliasing source that is not
  `plan_returned_move`'s moved local and not a move site.
- **Move sites need the lowering to know the op index.** Thread the current op index (the
  lowering loop already has it) into the two functions' callers, and look up `MoveSites`
  computed at function entry.
- **Verify no in-place arm can form a cycle** (plan-134-A "Verified properties" left it
  UNVERIFIED): read every in-place arm in `collection/assign/` and `collection/list/` and
  record, per arm, that it writes only values lowered through an owning store.

Risk: a move site that is not really last-use shares a graph — invisible now, a double free in
G. plan-134-C's tests carry that; D adds runtime probes of each move shape.

## Phases

### Phase 1 — RED

- [x] `tests/runtime/rt_recursive_value_copies.rs` (register in `Cargo.toml`), mirroring
      `tests/runtime/rt_error_value_copies.rs`: one program over `TYPE Node (kids AS List OF
      Node)` and a recursive `UNION Tree`, one line per store — `LET c = a` then a `MUT` copy
      rebuilt; `MUT ys = xs` + `removeAt` (declined by G24, rebuild path) and + in-place
      `append`; a global assigned from a local then the local's list appended; `RETURN` of a
      field (`RETURN h.kids`) mutated by the caller; a closure capturing a list then the list
      rebuilt. Each line reads the source after the copy changed. Confirm it fails today
      (at least the `append` line: `xs=240`). — `a_recursive_value_copy_is_independent_of_its_source`
      (one line per store: field → `MUT`, `MUT` + `removeAt`, `MUT` + `append`, global from a local,
      `RETURN h.kids`, an escaping closure's capture); fails today with `append xs=160` (the
      garbage value differs from the plan's 240 — a read of freed memory).
- [x] Add `tree_alias` as a case asserting `ys=6 xs=1`. — the `tree` line; fails today with
      `tree ys=6 xs=112`.

Acceptance: the new test fails on main for the documented reason.
  Check: `cargo test --release --test rt_recursive_value_copies` → failed, showing `xs=240`
  (est. 2 min).
  Result: met — `cargo test --release --no-fail-fast --test rt_recursive_value_copies` →
  `0 passed; 1 failed`; `left: "field a=96 ak=2\nremoveAt xs=2 ys=1\nappend xs=160 zs=3\n
  global g=96 local=2\nreturn h=96 got=2\nclosure f=96 cap=2\ntree ys=6 xs=112"` — identical
  to the pre-plan compiler's output (Corrections), so the failure is the documented aliasing and
  not a harness or build error.
Commit: —

### Phase 2 — copy-insertion

- [ ] `builder_values.rs`: `needs_graph_copy`; the new branch in `lower_value_owned`;
      `owned.rs::materialize_owned_element` uses `needs_graph_copy`.
- [ ] `builder_exits.rs`: the new branch in `lower_returned_value`.
- [ ] Thread the op index / `MoveSites` to both.
- [ ] The in-place-arm cycle audit, recorded in plan-134-A "Verified properties".
- [ ] Tests: Phase 1 passes; add move-shape probes to the same file (json-style `acc =
      append(acc, item)` with `item` rebound per iteration prints the right tree; a
      last-use `LET b = a` then `a` never read — output unchanged).

Acceptance: every store in the test yields an independent value.
  Check: `cargo test --release --test rt_recursive_value_copies` → passed; `cargo test
  --release --test rt_error_value_copies --test rt_net_address_record_layout` → passed (the
  sibling copy pins) (est. 4 min).
Commit: —

### Phase 3 — the speed gate and goldens

- [ ] `bash tools/recursive-value-bench/run.sh target/release/mfb json_repeat regex_repeat`
      five times; record the medians against §2.1. Over budget → count copies per parse with
      `--debug` (`arena.alloc_calls`) and find the store the analysis missed; fix it in C's
      analysis (with a table row), not by skipping the copy.
- [ ] `node_copies` with `--ncode`: record the copy-call count from `main` (expected 1 for
      `LET c AS Node = a` if `a` is read afterwards, else 0 — record which and why).
- [ ] `bash scripts/artifact-gate.sh target/release/mfb all`; expected diffs: json, regex and
      recursive-user-type fixtures only; regenerate with `scripts/regen-native-goldens.sh`.

Acceptance: decoders within budget; gate diffs confined and explained.
  Check: the bench medians (est. 2 min); the gate (est. 15 min — the only check covering every
  target's emitted stores).
Commit: —

## Validation Plan

- Tests: `rt_recursive_value_copies.rs`; the tree_alias case.
- Runtime proof: `tree_alias` prints `ys=6 xs=1` on macOS and box 2223.
- Doc sync: `.ai/collections.md` bug-601 GOTCHA (copy-insertion now exists for recursive
  types); `bugs/bug-601-…md` (recursive row fixed by plan-134-D — the bug closes).
- Final gate: plan-134-H.

## Open Decisions

- Speed budget — see plan-134-A.

## Corrections

- **The speed budget's baseline is a median of 5, measured the same way as the after-numbers.**
  §2.1's `json_repeat`/`regex_repeat` K=1 times are single runs that include the first launch of
  a freshly built binary. Re-measured on the pre-plan compiler (the main checkout's
  `target/release/mfb`, which reproduces the committed json/regex goldens byte-for-byte), five
  passes of `bash tools/recursive-value-bench/run.sh <mfb> json_repeat regex_repeat` (each pass
  rebuilds, so every run carries the same launch cost): `json_repeat` K=1 0.27 / 0.25 / 0.24 /
  0.26 / 0.23 s → **median 0.25 s**; `regex_repeat` K=1 0.27 / 0.26 / 0.28 / 0.28 / 0.26 s →
  **median 0.27 s**. Budget (+25 %): json ≤ 0.31 s, regex ≤ 0.34 s.
- **The RED program's pre-plan output** (pre-plan compiler, `/tmp` build of the same source the
  test embeds): `field a=96 ak=2`, `removeAt xs=2 ys=1`, `append xs=160 zs=3`, `global g=96
  local=2`, `return h=96 got=2`, `closure f=96 cap=2`, `tree ys=6 xs=112`. Five of seven lines
  are wrong; `removeAt` is right only because `G24` declines its in-place arm.
- **How a store asks plan-134-C's analysis.** `MoveSites` is keyed by the op's address
  (plan-134-C Corrections). `lower_ops_inner` records the address of the op it is lowering
  (saved and restored around nested bodies), and `lower_value_owned` / `lower_returned_value`
  ask `is_last_use` about that op. `MoveSites` is computed at the NIR-function entry
  (`lower_function`, which also lowers lambdas — they are `NirFunction`s); the synthesized
  builders (runtime helpers, per-type copy shims) carry none, so every store there copies.
- **A field site skips the copy and does nothing else in this letter.** plan-134-C reports
  `(op, x.f)` sites (`LET item = parsed.value`). Nulling `x.f` in the source record is what makes
  a field move safe once frees exist, but before letter E a construction store (`Wrap[n := x]`)
  can still alias `x`'s block, and nulling the field would be visible through that alias. So D
  returns the loaded pointer uncopied — exactly today's behaviour for that store — and the
  null-out belongs to plan-134-G's move deactivation, after E has removed construction aliasing.
  A D move therefore never creates an alias that did not exist before D.

## Summary

The copy half's main store sites. Safe by construction (adds copies only) and it closes
bug-601. The risk moves forward: every "move" it takes must be right, because G frees based on
the same ownership picture.
