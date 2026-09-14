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
Commit: a570a90d3

### Phase 2 — copy-insertion

- [x] `builder_values.rs`: `needs_graph_copy`; the new branch in `lower_value_owned`;
      `owned.rs::materialize_owned_element` uses `needs_graph_copy`. — plus `store_is_last_use`
      (a `Local` or `Local.field` source whose store is a plan-134-C site); the branch copies with
      `copy_value_to_current_arena` unless the store is a last use or a fresh trapped-`Result`
      wrapper.
- [x] `builder_exits.rs`: the new branch in `lower_returned_value`. — after the flat branch;
      keeps the parameter-passthrough borrow (`current_returns_param_borrow` + bare local) so a
      borrow function is not copied twice.
- [x] Thread the op index / `MoveSites` to both. — `CodeBuilder::move_sites` (computed in
      `lower_function` beside the other prescans) and `current_op_key` (set/restored per op in
      `lower_ops_inner`); synthesized builders carry `None`. The build is warning-free (the
      transient `last_use.rs` dead-code warnings are gone: 0 `warning` lines in the test build).
- [x] The in-place-arm cycle audit, recorded in plan-134-A "Verified properties". — the
      property is FALSE on main: a constructor argument and the in-place list/map arms' item
      operand are lowered with `lower_value`, so `xs = collections::append(xs, Node[kids := xs,
      tag := 1])` stores `xs`'s own block (repro: pre-plan compiler and plan-134-B walker both exit
      139 at the first `get`). The record-field and STATE arms cannot fire for the class (G17);
      removal arms store nothing. D's stores do not reach these operands; plan-134-E now owns them
      (Phase 1 census + RED line added).
- [x] Tests: Phase 1 passes; add move-shape probes to the same file (json-style `acc =
      append(acc, item)` with `item` rebound per iteration prints the right tree; a
      last-use `LET b = a` then `a` never read — output unchanged). —
      `a_moved_recursive_value_prints_what_it_printed_before` (`tree=99[0[0,],1[10,],2[20,],]`,
      `moved=1[7,]`, the pre-plan compiler's output; committed with Phase 1's file).

Acceptance: every store in the test yields an independent value.
  Check: `cargo test --release --test rt_recursive_value_copies` → passed; `cargo test
  --release --test rt_error_value_copies --test rt_net_address_record_layout` → passed (the
  sibling copy pins) (est. 4 min).
  Result: met — `cargo test --release --no-fail-fast --test rt_recursive_value_copies --test
  rt_error_value_copies --test rt_net_address_record_layout` → 2 + 1 + 2 passed, 0 failed.
  `cargo test --release --bin mfb -- collect_last_use_moves` → 4 passed after the key API change.
  Runtime: `tree_alias` → `ys=6 xs=1` on macOS and on Linux box 2223 (exit 0).
Commit: 41f5694d6

### Phase 3 — the speed gate and goldens

- [~] `bash tools/recursive-value-bench/run.sh target/release/mfb json_repeat regex_repeat`
      five times; record the medians against §2.1. Over budget → count copies per parse with
      `--debug` (`arena.alloc_calls`) and find the store the analysis missed; fix it in C's
      analysis (with a table row), not by skipping the copy.
      — json within budget, regex not: after plan-134-C's MATCH-view extension, medians of 5
      `json_repeat` K=1 0.24 s (budget 0.31), `regex_repeat` K=1 0.43 s (budget 0.34), see
      Corrections. **Owner decision (2026-09-13): accept the regex cost for now, keep the
      correctness-first copies, re-measure after letters G/H free memory, and report the final
      numbers before merge** (added to plan-134-H Phase 3). Remaining: that re-measure.
- [x] `node_copies` with `--ncode`: record the copy-call count from `main` (expected 1 for
      `LET c AS Node = a` if `a` is read afterwards, else 0 — record which and why). —
      `run.sh <D mfb> node_copies` → `main_copy_calls=1` (was 0): `LET c AS Node = a` copies,
      because `a` is read afterwards (`toString(a.tag)` in the print). The list literal in `b`
      and the `append` are construction stores (letter E), so they still emit none.
- [x] `bash scripts/artifact-gate.sh target/release/mfb all`; expected diffs: json, regex and
      recursive-user-type fixtures only; regenerate with `scripts/regen-native-goldens.sh`.
      — `artifact-gate [all]` (final D build, `/tmp/p134-mfb-dfinal`): `2013 golden(s)
      checked, 10 diff(s)`, exactly `byte-identity/json` and `byte-identity/regex` on all five
      targets. Localized per function against the pre-D dumps (Corrections); regenerated with
      `scripts/regen-native-goldens.sh` (`10 golden(s) rewritten, 0 failure(s)`); re-gated →
      json `7 golden(s) checked, 0 diff(s)`, regex `7 golden(s) checked, 0 diff(s)`.

Acceptance: decoders within budget; gate diffs confined and explained.
  (Budget: partially met — json within, regex over; the owner accepted it pending plan-134-H's
  re-measure, 2026-09-13.)
  Result: gate diffs confined and explained (json/regex only, localized per function); the
  budget as recorded above.
  Check: the bench medians (est. 2 min); the gate (est. 15 min — the only check covering every
  target's emitted stores).
Commit: 41f5694d6

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
- **Speed gate, first measurement: `regex_repeat` is far over budget, `json_repeat` within it.**
  Five passes of `run.sh /tmp/p134-mfb-d json_repeat regex_repeat` (D's release build):
  `json_repeat` K=1 0.54 / 0.26 / 0.25 / 0.28 / 0.25 s → median 0.26 s (budget 0.31), RSS
  212.5 MB (was 204.3); `regex_repeat` K=1 1.36 / 1.93 / 1.21 / 1.90 / 1.80 s → **median
  1.80 s (budget 0.34)**, RSS **3.62 GB** (was 345 MB — every copy leaks until letter G).
  Localized with `-ncode` relocations to the per-type copy shims, before (`/tmp/p134-mfb-c`) vs
  after D: `#regex_run` went from 3 to 10 `__regex_Node` copy calls and gained 5 `__regex_Cont`,
  2 `__regex_Repeat`, 2 `__regex_Choices`; `#regex_isSimpleNode`, `#regex_simpleMatchAt` and
  `#regex_requiredFirstCp` each gained a `__regex_Node` copy. The NIR shows the cause
  (`-nir`, `#regex_isSimpleNode`): the MATCH desugar binds its scrutinee to a temporary —
  `Bind $match5 = Local(node)` — and the cases alias it (`Bind litNode =
  UnionExtract(Local $match5)`). That bind is an ordinary owning store, so D deep-copies a
  recursive scrutinee whenever plan-134-C does not call it a move: always for a parameter (the
  per-step helpers), and in `#regex_run` whenever the source stays live. And C excludes the case
  aliases, so `stack = c.nxt` copies the rest of the `__regex_Choices` chain on every backtrack
  pop (quadratic in the pending count) and `cont = seqCont.nxt` copies the continuation.
  Per the plan the fix is in C's analysis, not a skipped copy: a read-only MATCH view borrows,
  an owning view's aliases are places of the view, and an exhaustive MATCH has no fall-through.
- **The MATCH-view extension (plan-134-C Corrections) and the bug its first run found.** D's
  consumer: `lower_value_owned` also skips the walker copy when the op is a borrowed view
  (`store_is_borrowed_view` → `MoveSites::is_borrow`). The first run of
  `collect_last_use_moves_covers_the_regex_matcher_views` failed: `#regex_run`'s `stack = c.nxt`
  was not a site although `$match11 = stack` was correctly an owning move. A temporary
  `#[ignore]` probe printed the shape — `$match12 <- Field("c", "alt")`, a borrowed view bound
  from a FIELD of the alias `c` — and the cause: a read through a borrowed view was charged to
  the source's root local as a whole read, so `MATCH c.alt` read all of `$match11` and kept
  `$match11.nxt` live after the pop. Fixed by charging a view bound from a field to that field
  (`Canon` maps a name to a `Place`); the SHAPES probe also renamed a field from the keyword
  `next` (`MFB_PARSE_INVALID_IDENTIFIER`). `cargo test --release --bin mfb --
  collect_last_use_moves` → `6 passed; 0 failed`.
- **Speed gate after the MATCH-view fix: json within budget, regex still over.** Five passes of
  `run.sh /tmp/p134-mfb-d2 json_repeat regex_repeat`: `json_repeat` K=1 0.27 / 0.24 / 0.25 /
  0.24 / 0.24 s → median 0.24 s (budget 0.31); `regex_repeat` K=1 0.46 / 0.43 / 0.43 / 0.42 /
  0.44 s → **median 0.43 s (budget 0.34)**, RSS 919 MB (was 3.62 GB before the fix, 345 MB
  pre-plan). `regex_chain simple:10000 --debug`: `arena.0.alloc_calls` 712 092 (pre-plan
  482 081). `-ncode` census of `#regex_run`: 8 `__regex_Node` copy calls (3 of them the
  pre-existing `collections::get` copies), 1 `__regex_Repeat`, 1 `__regex_Cont`. The store listing
  (`-nir`, binds/assigns from a local or field) names them: `MUT node = root` (a parameter — once
  per match attempt), `repRec = repNode` (an alias of the `MATCH node` view, which borrows because
  `node` stays live across loop iterations), `cont = c.cont` (`c.cont` is read again on the kind-1
  path), `node = c.rep.child` / `grpNode.child` / `repRec.child` (nested or live sources).
  **Refuted hypothesis:** the walker's 64-entry first work stack (1 552 bytes, entropy-filled on
  alloc and scrubbed on free per copy) was not the cost — an initial capacity of 4 measured
  0.58 / 0.45 / 0.40 / 0.40 / 0.54 s (median 0.45 s), no change; reverted. Removing the remaining
  copies needs a *borrowed local* (a non-collection recursive local bound from a borrowed source
  that never copies), which letter G would then have to free only when the slot owns its value —
  an ownership state the design does not have. That is this plan's Open Decision ("speed budget
  for letter D"), put to the owner.
- **Golden localization (Phase 3).** Per-function `-ncode` diff of the two byte-identity fixtures,
  pre-D (plan-134-B/C build — C's gate proved the two identical) vs the final D build: **regex**
  189 → 189 functions, nothing added or removed, only `#regex_run` changed, and its copy-shim
  calls went from 3 `__regex_Node` to 8 `__regex_Node` + 1 `__regex_Cont` + 1 `__regex_Repeat`
  — the stores listed above. **json** 162 → 162, changed only `json::get` (2 → 4 `json::Json`
  copy calls), `json::getOr` (2 → 7), `#json_parseArrayItems` (+1 `List OF Json`) and
  `#json_parseObjectItems` (+1 `Map OF String TO Json`). The parse helpers' copy is their
  accumulator bound from a parameter (`MUT acc AS List OF Json = items`,
  `MUT acc AS Map OF String TO Json = fields`, one small copy per array/object). The `get`/`getOr`
  copies are the path walk (`current = value` from a parameter, `nextValue = current`,
  `currentValue = current`, `current = nextValue`): correct, but a `json::get` now copies the
  subtree at each path step. `json_repeat` does not exercise `get`, so plan-134-H's speed
  re-measure should add a `json::get` probe before the owner report.

## Summary

The copy half's main store sites. Safe by construction (adds copies only) and it closes
bug-601. The risk moves forward: every "move" it takes must be right, because G frees based on
the same ownership picture.
