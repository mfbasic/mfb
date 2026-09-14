# plan-134-G: Free recursive values where their owners end

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-F

Register plan-134-F's drop at every place an owner of a recursive value ends: scope exit, the
end of a statement for an unbound temporary, overwrite by assignment (local and global), a
moved source's deactivated cleanup, a closure environment, and the owned copy `collections::get`
returns. Collection **element** frees are letter H. After this letter bug-536 Shape C's two
repros run flat.

Behavioural outcome: **`c_union_rss` and `c_record_rss` (plan-134-A §2.2) peak at the same RSS
at 400 000 and 800 000 iterations** (today 52.7 → 104.3 MB and 105.1 → 209.1 MB), and every
value test from letters B–F still prints the same output.

References: plan-134-A; plan-134-F; `mfb spec memory arenas` "Scope-Drop Frees";
`src/codegen/engine/builder/mod.rs::OwnedValueCleanup`;
`src/codegen/cleanup/owned/builder_owned_cleanup.rs::emit_owned_value_drop`;
`tests/runtime/rt_scope_drop_leaks.rs` (`assert_flat`, `peak_rss`).

Prerequisites: see plan-134-A; plan-134-F complete (`ls planning/completed/plan-134-F-*`). — MET
2026-09-13 (`planning/completed/plan-134-F-non-recursive-drop-functions.md`).

## 1. Goal

- New shape-C cases in `tests/runtime/rt_scope_drop_leaks.rs` pass via `assert_flat` (growth
  < 8 MiB between the two counts): union bind, record bind, reassignment, global overwrite,
  unbound temp (`len(json::stringify(json::parse(t)))`-style), closure capture, `get` result.
- The `--debug` churn loop of every B–F test shows `arena.double_free_skips` = 0.

### Non-goals

- Element frees inside collections (letter H): a `List OF Node`'s drop still frees only the
  list block until H, so list-heavy decoders are not yet flat.
- No change to flat-value cleanup codegen (byte-identical for modules without a recursive type).

## 2. Current State

- `NirOp::Bind` registers `ActiveCleanup::OwnedValue` when `owns_freeable_value` (requires
  `is_freeable_flat_value`); cleanups drain at scope end (`builder_control.rs`) and at exits
  (`builder_exits.rs`, including `emit_cleanup_branch_to_depth`).
- `OwnedValueCleanup` fields: `type_`, `stack_offset`, `closure_captures`, `capacity_slot`,
  `loop_alias_slot`, `result_wrapper`. `plan_returned_move` and `lower_returned_value` find
  ownership by `ActiveCleanup::OwnedValue` at a `stack_offset` — bug-571's comment warns against
  a separate variant.
- `pending_temp_is_freeable` requires `is_freeable_flat_value && !value_needs_owning_copy &&
  !value_is_runtime_managed`; `drop_pending_temps_to` / `clear_pending_temps_to`.
- `Assign` frees the old value via `emit_owned_value_drop` unless `by_ref`, not freeable-flat, a
  live `FOR EACH` iterable, or a record-field iterable base; `StoreGlobal` likewise.
- The closure drop frees a `Local` capture only when freeable-flat (`builder_owned_cleanup.rs`).
- **Non-owners that must stay unfreed** (read in plan-134-A's census): plan-86 E borrow-`get`
  (only freeable-flat non-`String` elements qualify, so a recursive element is never borrowed —
  VERIFY), `MATCH` scrutinee (`lower_value`, no cleanup), `UnionExtract` (`aliases_union_variant`),
  `by_ref` / `by_ref_capture_slot`, `FOR EACH` element (alias; `for_each_iterable_locals`),
  parameters (no `OwnedValue` at their slot).

### 2.1 The `is_freeable_flat_value` references, classified (Phase 1)

Census: `grep -rn 'is_freeable_flat_value' --include='*.rs' src/codegen | grep -v 'fn
is_freeable_flat_value' | wc -l` → **32** (2026-09-13, after letter F; matches plan-134-A). Every
row is one reference. **Ownership gate** = the reference decides whether this frame frees a
block, so it gains `|| owns_graph`. **Layout/copy** = it decides how a value is copied or
aliased, and stays flat-only (the recursive answer to the same question is already its own
branch: `needs_graph_copy`, plan-134-D/E). **Class** = the recursive class's own definition.

| # | file::symbol | kind | verdict |
|---|---|---|---|
| 1 | `engine/control/builder_control.rs::lower_ops_inner` `Bind` → `owns_freeable_value` (:594) | code | **ownership gate** — registers `ActiveCleanup::OwnedValue` |
| 2 | `engine/control/builder_control.rs::lower_ops_inner` `Assign` old-value free (:1195) | code | **ownership gate** — `emit_owned_value_drop` of the slot before overwrite |
| 3 | `engine/control/builder_control.rs::lower_ops_inner` `StoreGlobal` old-value free (:994) | code | **ownership gate** — both branches store; only the free differs |
| 4 | `engine/value/builder_values.rs::pending_temp_is_freeable` (:397) | code | **ownership gate** — statement-scope free of an unbound temp |
| 5 | `engine/value/builder_values.rs::runtime_result_is_caller_owned` (:1009) | code | **ownership gate** — frees a trapped runtime call's result (bug-566/576) |
| 6 | `cleanup/owned/builder_owned_cleanup.rs::emit_closure_drop` capture loop (:481) | code | **ownership gate** — captures are stored through `lower_value_owned`, so a recursive capture owns its graph |
| 7 | `collection/buffer/collection_buffer.rs::free_intermediate_collection` (:19) | code | **ownership gate** — frees a consumed intermediate collection block |
| 8 | `engine/control/builder_control.rs::lower_ops_inner` `is_borrow_get` (:587) | code | **layout/copy** — borrow eligibility; must stay flat-only (a recursive element is never borrowed) |
| 9 | `memory/owned.rs::materialize_owned_element` (:64) | code | **layout/copy** — `copy_flat_block`; the recursive copy is the `needs_graph_copy` branch above it |
| 10 | `engine/value/builder_values.rs::lower_value_owned` (:749) | code | **layout/copy** — `copy_flat_block`; the graph copy is the plan-134-D branch below it |
| 11 | `engine/control/builder_exits.rs::lower_returned_value` (:207) | code | **layout/copy** — `copy_flat_block` on return; plan-134-D's graph branch covers recursive |
| 12 | `engine/value/operand_snapshot.rs::snapshot_aliased_operand` (:157) | code | **layout/copy** — `copy_flat_block` of an in-place-mutated operand; a recursive operand takes no snapshot today and gains no free |
| 13 | `engine/value/builder_values.rs::needs_graph_copy` (:1184) | code | **class** — `!is_freeable_flat_value && reaches_cycle && !resource` |
| 14–19 | `collection/layout/builder_collection_layout.rs` :3110, :3249, :3572, :3600, :3739, :3786 | comment | none |
| 20–22 | `collection/layout/builder_collection_layout.rs` :3614, :3797, :3808 | test assertion message | none |
| 23 | `memory/owned.rs` :33 | comment | none |
| 24 | `builtins/canvas/gen_group.rs` :167 | comment | none |
| 25 | `engine/builder/mod.rs` :798 | comment | none |
| 26–28 | `registry/mod.rs` :6400, :6507, :6512 (`carries_a_block` is its own predicate and does not call it) | comment | none |
| 29 | `engine/builder/builder_emit_helpers.rs` :507 | comment | none |
| 30–32 | `engine/value/builder_values.rs` :995, :1001, :1598 | comment | none |

Totals: 7 ownership gates, 5 layout/copy, 1 class, 19 comment/test text = 32.

### 2.2 Non-owner verdicts (Phase 1)

- **plan-86 E borrow-`get`** — VERIFIED never a recursive element: `is_borrow_get` requires
  `is_freeable_flat_value(type_)` (`builder_control.rs::lower_ops_inner`, row 8), and
  `register_pending_temp` returns early under `borrow_get_result`. Row 8 stays flat-only.
- **`MATCH` scrutinee** — VERIFIED: lowered with `lower_value` and spilled to `match_value`, no
  cleanup (`builder_control.rs` `NirOp::Match`). A fresh call scrutinee becomes a pending temp
  only through `register_pending_temp`. The statement watermark is taken before the op
  (`let temp_watermark`, `lower_ops_inner`), and its `drop_pending_temps_to` runs after the op's
  closure returns (`TransferTemps::StatementScope`), i.e. after `END MATCH`. Each arm's
  statements drop only above their own watermark. The temp therefore already outlives every
  `UnionExtract` alias into it; nothing to move.
- **`UnionExtract`** — VERIFIED: `aliases_union_variant` excludes it from
  `owns_freeable_value`, and its cleanup branch is "Non-owning — no cleanup".
- **`by_ref` / `by_ref_capture_slot`** — VERIFIED: excluded from `owns_freeable_value` and
  non-owning at registration; the `Assign` free requires `!by_ref`.
- **`FOR EACH` element** — VERIFIED an alias for every non-`String` payload: bug-571's
  `owned_item_slots` is filled only by the `String` arms. The iterable local is excluded from
  the `Assign` free by `for_each_iterable_locals`, and a `name.field` iterable base by
  `for_each_iterable_record_fields`.
- **Parameters** — VERIFIED: no `OwnedValue` at their slot. `plan_returned_move` uses exactly
  that as its ownership test ("parameters and aliases have none").
- **Closure captures** — an OWNER, not a non-owner: the env stores each capture through
  `lower_value_owned` (`NirValue::Closure` arm), which for a recursive aliasing capture is
  plan-134-D's graph copy — or a move when it is the source's last use, which is exactly what
  the Phase 2 move-site deactivation must cover.

## 3. Design

- **One ownership predicate.** `CodeBuilder::owns_graph(type_)` = `needs_graph_copy(type_)`
  (plan-134-D). Everywhere a gate reads `is_freeable_flat_value` to decide *ownership*
  (`owns_freeable_value`, `pending_temp_is_freeable`, the `Assign`/`StoreGlobal` old-value free,
  the closure capture free), OR in `owns_graph` — each of the 32 references (plan-134-A §2.1)
  is classified in Phase 1 as "ownership gate" or "flat-layout question", and only ownership
  gates change. The classification is a table in §2, total (no reference unclassified).
- **Drop dispatch.** `emit_owned_value_drop`: when `owns_graph(type_)`, call
  `_mfb_rt_graph_drop(kind, slot)` and zero the slot (bug-440's null-guard pattern). No new
  `ActiveCleanup` variant (bug-571).
- **Moves deactivate.** A plan-134-C move site whose source local has an `OwnedValue` cleanup
  deactivates it for the rest of the scope, exactly as `plan_returned_move` does for a return
  (remove + restore on the moved path). Without this, a moved graph is freed twice.
- **`MATCH` on a fresh temporary.** A `MATCH` scrutinee that is a fresh call result becomes a
  pending temp freed at statement end, while the arms bind `UnionExtract` aliases into it. The
  temp must stay alive through the whole `MATCH` — the free is attached after the `END MATCH`,
  not at the scrutinee's statement boundary. Phase 1 verifies where today's flat-union MATCH
  frees it and mirrors it.
- **`get` results.** bug-538's owned copy is now registered like any fresh value (it flows
  through the `Bind` arm), so it is freed; no special case.

Risk: this is where a missed non-owner becomes a double free or use-after-free. Every
classification is a table row with the code read, and the churn loops run with the arena's
entropy fill (freed chunks scrubbed) so a use-after-free reads garbage rather than stale data.

## Phases

### Phase 1 — classification and RED

- [x] Classify all 32 `is_freeable_flat_value` references (plan-134-A §2.1 command) as
      ownership gate / layout question, with file::symbol; add the table to §2. — §2.1: census
      re-run → 32; 7 ownership gates, 5 layout/copy, 1 class definition, 19 comment/test text.
- [x] Verify each non-owner in §2 by reading its code path for a recursive type (borrow-`get`
      eligibility, `MATCH` temp lifetime, `UnionExtract`, `by_ref`, `FOR EACH`, params); record
      each verdict. — §2.2: all six verified non-owners. The `MATCH` fresh temp already lives
      past `END MATCH` (statement-scope drop after the op). Closure captures are owners.
- [x] Add the shape-C cases of §1 to `tests/runtime/rt_scope_drop_leaks.rs` using `assert_flat`;
      confirm they fail today. — Seven cases (`a_looped_recursive_*`, `a_looped_unbound_recursive_temp_*`),
      400 000 vs 800 000 iterations, run against the letter-F build (no plan-134-G code). Peak
      RSS growth per case:

      | case | growth | peak at 400k → 800k |
      |---|---|---|
      | union bind | 49 MB | 50 → 99 MB |
      | record bind | 99 MB | 100 → 199 MB |
      | reassignment | 99 MB | 100 → 199 MB |
      | global overwrite | 99 MB | 100 → 199 MB |
      | `get` result | 99 MB | 100 → 199 MB |
      | closure capture | 297 MB | 298 → 596 MB |
      | unbound temp | 3125 MB | 3126 → 6251 MB |

Acceptance: the table is total; the new leak cases fail on main.
  Check: `cargo test --release --test rt_scope_drop_leaks -- recursive` → the new cases failed
  (est. 4 min).
  Result: §2.1 total (32 rows); `0 passed; 7 failed` (each "peak RSS grew … — the loop leaks").
Commit: ad602369b

### Phase 2 — registration

- [x] `owns_graph`; change each ownership gate from the table. — `builder_values.rs::owns_graph`
      (`needs_graph_copy` with a walker kind). ORed into six gates:
      - `Bind` `owns_freeable_value`, which also excludes a borrowed `MATCH` view;
      - the `Assign` old-value free;
      - the `StoreGlobal` old-value free;
      - `pending_temp_is_freeable`;
      - `runtime_result_is_caller_owned`;
      - the `emit_closure_drop` capture free.

      Row 7 is deferred to H (Corrections).
- [x] `emit_owned_value_drop` dispatch to `_mfb_rt_graph_drop`. — `emit_graph_value_drop`
      (`graph_drop.rs`): null-guarded walker call, then the slot is zeroed.
- [x] Move-site deactivation in the store lowering (plan-134-C/D sites). — `release_moved_source`
      zeroes the moved local's slot or record field word, and `move_source_is_owned` gates every
      move. Also the shallow frees for byte-copying stores and the `WITH` kept-field copy
      (Corrections).
- [x] `MATCH` fresh-temp lifetime as verified. — No change needed (§2.2). A `MATCH` over a
      parameter needed the borrowed-view bind exclusion instead (Corrections). Exercised by the
      churn case's `weigh(t)` and `/tmp/p134-g-variants.sh` (5/5 exit 0; `--debug`
      `live_bytes 0`, `double_free_skips 0`).
- [x] Tests: Phase 1 leak cases pass; every B–F runtime test passes; add a `--debug` churn case to
      `rt_recursive_value_copies.rs` asserting `double_free_skips = 0` and unchanged output over
      20 000 iterations of every store shape. —
      `every_recursive_store_shape_frees_once_under_churn`: fresh bind, copy, inline-payload
      moves, global overwrite, `WITH`, `get`, returned field, in-place appends, unbound temp,
      union wrap with `MATCH` over a parameter, and closure capture. It expects `acc=1340000`
      (67 per iteration, derived per line in the test's doc comment) and
      `double_free_skips 0`.

Acceptance: the shape-C repros are flat and no value or churn test changes.
  Check: `cargo test --release --test rt_scope_drop_leaks --test rt_recursive_value_copies
  --test rt_recursive_value_construction_copies --test rt_recursive_value_drop_symmetry
  --test rt_recursive_value_copy_depth --test rt_error_value_copies` → passed (est. 12 min:
  `rt_scope_drop_leaks` is 118 RSS cases and is the only suite that sees a leak or a freed-flat
  regression).
  Result (one binary):

  | suite | passed |
  |---|---|
  | `rt_scope_drop_leaks` | 125 / 125 (118 existing + 7 shape-C) |
  | `rt_recursive_value_copies` | 3 / 3 |
  | `rt_recursive_value_construction_copies` | 4 / 4 |
  | `rt_recursive_value_drop_symmetry` | 6 / 6 |
  | `rt_recursive_value_copy_depth` | 3 / 3 |
  | `rt_error_value_copies` | 1 / 1 |
Commit: 9dcb16fd4

### Phase 3 — measurements and goldens

- [x] `tools/recursive-value-bench/run.sh target/release/mfb c_union_rss c_record_rss` → flat;
      record in plan-134-A §2.1's "after" column. — `c_union_rss` 1 081 344 B at 400k and
      1 081 344 B at 800k; `c_record_rss` 1 032 192 B at both (exit 0; the baselines were
      52.7 → 104.3 MB and 105.1 → 209.1 MB). Recorded in plan-134-A's new "After" table below
      §2.1, which has no "after" column.
- [x] Artifact gate; expected diffs: modules with a recursive type (new drop calls at scope
      exits, temps, assignments). Regenerate. — `bash scripts/artifact-gate.sh
      target/release/mfb all` → `2013 golden(s) checked, 10 diff(s)`, all
      `json_codegen_cover_rt` / `regex_codegen_cover_rt` `.ncode` on the five targets.
      Localized with `mfb build -ncode` and `/tmp/p134-ncode-diff.py`:
      - **json**, against letter F's dump: `added: []`, `removed: []`, 12 functions changed —
        `main`, `#json_get`, `#json_getOr`, `#json_parse`, `#json_parseArray`,
        `#json_parseArrayItems`, `#json_parseNumber`, `#json_parseObject`,
        `#json_parseObjectItems`, `#json_parseRevive`, `#json_parseValue`, `#json_revive`.
        11 of them now call `_mfb_rt_graph_drop`; `#json_parse` changed through a temp
        claim/move only.
      - **regex**: 10 functions now call it — `#regex_run`, `#regex_parseAlt`,
        `#regex_parseAtom`, `#regex_parseClass`, `#regex_parseConcat`,
        `#regex_parseEscapeAtom`, `#regex_parseNamedGroup`, `#regex_parseParen`,
        `#regex_parseQuantSuffix`, `#regex_requiredFirstCp`.

      Every changed function owns or stores a recursive value. Regenerated:
      `scripts/regen-native-goldens.sh target/release/mfb tests/byte-identity/json
      tests/byte-identity/regex` → `10 golden(s) rewritten, 0 failure(s)`. Re-gated → json and
      regex `7 golden(s) checked, 0 diff(s)` each.
- [x] Run the copy and leak tests' programs cross-built on box 2223 (Linux) and, via a `.cmd`
      wrapper, on box 2230 (Windows has no harness: `.ai/remote_systems.md`). —
      `bash /tmp/p134-g-boxes.sh` extracts the programs from `rt_recursive_value_copies.rs`
      (copies, moves, store-shape churn) and `rt_scope_drop_leaks.rs` (the seven shape-C
      programs at 800 000 iterations). It cross-builds each for `linux-aarch64` and
      `windows-x86_64` and runs them on 2223 and 2230; a plain `.exe` needs no `.cmd` wrapper
      over ssh. **20 / 20 ok, `status=0`**:
      - copies, moves and churn print exactly the macOS assertions: `acc=1340000`, the seven
        copy lines, and `tree=99[…]|moved=1[7,]`;
      - every shape-C loop exits 0 with the same value line on both boxes.

      Correction: the first run lost the Linux outputs, because the Windows build replaced the
      project's `build/` directory before they were copied. The script now copies each output
      right after its own build.
- [x] Added (Validation Plan doc sync):
      - `src/docs/spec/memory/04_arenas.md` "Scope-Drop Frees": recursive values are freed by
        `_mfb_rt_graph_drop`; moves zero the source; byte-copying stores free only the copied
        block. `mfb spec memory arenas` renders both paragraphs.
      - `.ai/codegen-invariants.md`: the recursive-types sentence is history, keeping the rule.
      - bug-536's status gains the shape-C progress line.
      - plan-134-A gains an "After" table with this letter's two rows.

Acceptance: repros flat on macOS; the programs print the same output on Linux and Windows;
diffs confined.
  Check: bench (est. 2 min); gate (est. 15 min); box runs (est. 10 min).
  Result:
  - macOS bench flat: 1.08 / 1.08 MB and 1.03 / 1.03 MB.
  - Linux and Windows 20 / 20 identical.
  - Gate diffs confined to json/regex, explained per function, regenerated, re-gated clean.
Commit: 699b63b11

## Validation Plan

- Tests: the new `rt_scope_drop_leaks` cases; churn assertions.
- Runtime proof: bench flat; box 2223 and 2230 runs.
- Doc sync: `mfb spec memory arenas` "Scope-Drop Frees" (no longer "no per-type recursive drop
  glue"); `.ai/codegen-invariants.md` (the "no owning copy at a bind and no drop anywhere"
  passage for recursive types becomes history); bug-536 doc Phase 3 progress.
- Final gate: plan-134-H.

## Open Decisions

- None beyond plan-134-A's.

## Corrections

- **Prerequisite re-run** (2026-09-13): `ls planning/completed/plan-134-F-*` → one file — MET.
- **The `MATCH` fresh-temp task needs no lifetime change.** §3 says the temp "must stay alive
  through the whole `MATCH` … Phase 1 verifies where today's flat-union MATCH frees it". The
  read (§2.2) finds the statement-scope free already runs after the whole op. Phase 2's task
  is therefore a test that a fresh recursive scrutinee is read correctly in every arm under
  churn, not a code change.
- **§2.1 row 7 (`free_intermediate_collection`) is deferred to plan-134-H, not changed here.**
  Its callers (`list_mutate.rs` removeAt, `func_set.rs`, `func_insert.rs`, `gen_mutate.rs`,
  `func_sort*.rs`, `func_group_by.rs`, `func_partition.rs`) free the intermediate INSIDE the
  builtin's lowering. That is before `own_collection_payload_edges` (plan-134-E, hooked at
  `try_abi_inline_lower`) deep-copies the result's element edges. A graph drop there would free
  the element graphs the result still points at. It stays flat-only, so the intermediate leaks
  as today; H owns element frees. Ownership gates changed in G: 6.
- **"Moves deactivate" is not enough: a store that byte-copies a recursive value shares its
  children.** §3 treats every store as keeping the value's pointer. Three store shapes copy the
  top block's BYTES instead:
  - a record or union collection payload (`payload_edge_shape` → `InlineRecord`/`InlineUnion`:
    list/map/set literals, every in-place item store);
  - a union wrap (`emit_wrap_record_in_union` byte-copies the variant record at +16);
  - `WITH`'s kept fields (`lower_with_update` copies each untouched field's word, so a kept
    recursive pointer field is shared by the old and the rebuilt record).

  Freeing the source's graph after any of these frees the new owner's children. Remedies:
  - **Store shapes.** `lower_value_stored` becomes `StoreShape`-aware (`graph_drop.rs`):
    `Pointer` for a constructor argument, a `WITH` update and a `STATE` replacement;
    `Inline` for a union wrap; `Payload` classified by `payload_edge_shape`.
  - **Fresh values.** A pointer store claims the pending temp. An inline store makes it
    `shallow` (`PendingTemp::shallow` → `emit_shallow_block_free` frees only the top block).
  - **Last read of an owning place.** `release_moved_source` zeroes the local's slot, or the
    record's pointer-field word, which every drop skips. An inline store also registers a
    shallow free of the copied block.
  - **Other aliasing sources.** Copied as in plan-134-E; an inline store shallow-frees the
    copy's top block.
  - **`WITH`.** A kept `owns_graph` pointer field is copied into the rebuilt record.
- **A move must come from an owner.** `store_is_last_use` now also requires
  `move_source_is_owned`: the place's root local has a live `OwnedValue` at its slot, and a field
  place names a record pointer field. A `MATCH` view (an alias into its union) or a by-ref local
  owns nothing, so handing its pointer to an owner would free an interior pointer. Such a store
  copies instead, a codegen change from plan-134-D/E for those sites.
- **The unbound-temp case is a user type; its json form moves to plan-134-H.** §1 names the case
  "`len(json::stringify(json::parse(t)))`-style". After Phase 2 that program no longer crashes
  but still grows 643 MB (400k → 800k). A `--debug` split shows the growth is not the temp:
  - unbound `len(json::stringify(json::parse(t)))`, 1 000 iterations → `live_bytes 2912000`;
  - bound `LET v AS json::Json = json::parse(t)` → the same `2912000` (82 003 allocs, 48 003
    frees);
  - user-type unbound temp `total(mk(i))` → `live_bytes 0` (5 002 allocs, 5 002 frees).

  The 34 blocks per iteration are left inside json's list-building helpers, which §1's
  non-goals assign to letter H ("list-heavy decoders are not yet flat"). The G case now uses the
  user-type form, same test name. The json form is a new plan-134-H Phase 1 task with these
  numbers.
- **A borrowed `MATCH` view's bind is a non-owner §2.2 missed.** `MATCH t` over a parameter
  lowers to `bind $match0 local t` before the `match` op (`-nir` of a `weigh(t AS Tree)`
  probe). plan-134-C marks that bind a borrowed view, so `lower_value_owned` skips the graph
  copy and `$match0` holds the caller's graph. The widened `owns_freeable_value` registered a
  graph drop on it, and `weigh` freed the caller's tree. The caller's own drop then walked
  garbage: SIGSEGV at exit after correct output, in 4 of 5 variants (`/tmp/p134-g-variants.sh`;
  `_mfb_fn_weigh` relocated to `_mfb_rt_graph_drop`). Fix: `borrowed_graph_view`
  (`owns_graph && store_is_borrowed_view`) is excluded from `owns_freeable_value`.
- **`lower_returned_value`'s recursive param-borrow shortcut relied on "a recursive local owns no
  `OwnedValue` cleanup".** That is false after this letter, so it now uses the flat branch's
  `owns_block` guard.

## Summary

The highest-risk letter: it turns ownership into frees. It is safe only because B–E made every
owner distinct and F proved the drop exact; the total classification table and the churn loops
are its own guard.
