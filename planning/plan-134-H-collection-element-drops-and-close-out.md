# plan-134-H: Free recursive collection elements, measure the decoders, close bug-536 Shape C

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-G

A collection of recursive values owns its elements (`mfb spec language memory-semantics` §14.6),
but its drop frees only the collection block, and removing or overwriting an element leaves the
element's graph behind. This letter frees element graphs on collection drop, removal, overwrite
and rebuild; revisits `G24`; measures the decoders; syncs the docs; and runs plan-134's only
full gate.

Behavioural outcome: **parsing the same JSON document K times costs the document's own tree once,
not K times** — `json_repeat` peak RSS at K = 1, 2, 4 differs by less than 8 MiB (today 204 /
402 / 799 MB) — and `regex_repeat` likewise (today 345 / 673 / 1329 MB).

References: plan-134-A (baselines); plan-134-F (drop walker); plan-134-G (registration);
`src/codegen/error/emission/park_error_helper.rs::lower_drop_owned_collection_helper`;
`src/codegen/collection/assign/builder_inplace_assign.rs` (`G24`); `.ai/collections.md`.

Prerequisites: see plan-134-A; plan-134-G complete (`ls planning/completed/plan-134-G-*`). — MET
2026-09-13 (`planning/completed/plan-134-G-drop-registration.md`).

## 1. Goal

- `json_repeat` and `regex_repeat` flat across K (growth < 8 MiB, `assert_flat`'s threshold).
- A list/map of recursive values dropped, `removeAt`/`set`/`removeKey`ed, or rebuilt returns
  `arena.live_bytes` to its start.
- plan-134's full gate passes once.

### Non-goals

- Flat collections' drop helper stays byte-identical.
- No change to the collection header or payload layout.

## 2. Current State

- `_mfb_rt_drop_owned_collection` computes `header + cap*stride + dataCap (+ cap<<4 buckets)`
  and makes one `arena_free`; it never walks elements (plan-134-A census, agent-read).
- A collection type that participates in a cycle (`List OF Node` for `TYPE Node (kids AS List
  OF Node)`) is itself a member of `recursive_transfer_types`, so the F walker already has a
  kind for it; a collection whose element type only *reaches* a cycle (a `List OF json::Json`
  in user code) is not in that set — Phase 1 measures how the walker handles it.
- `G24` declines in-place `removeAt` for a recursive element type in 3 arms; the rebuild path
  it falls back to copies surviving elements into a new list (copies after plan-134-E) and drops
  the old list (element frees after this letter).
- In-place `set`/`insert`/`removeKey` overwrite or remove an element slot without freeing the
  old element graph (UNMEASURED per arm — Phase 1 task).

## 3. Design

- **Collection drop.** For an owned collection whose element (or value) type is in the class
  (`needs_graph_copy`), the drop goes through `_mfb_rt_graph_drop` with the collection's kind;
  the walker's collection shape pushes each element's edge before freeing the collection block
  (an inline record/union element: its cycle-type fields; a pointer payload: the element). If a
  reaching-but-not-participating collection type has no kind, add it to the walker's kind set
  (the set becomes "types that reach a cycle", generated from `type_reaches_cycle`).
- **Element removal/overwrite in place.** Each in-place arm that discards an element
  (`removeAt`, `set`, `removeKey`, `insert` over an existing key) drops the old element graph
  before the slot is reused, via the walker — the arm list comes from Phase 1's census.
- **`G24`.** Once `get` owns its copy (bug-538, freed since G) and removal frees the removed
  graph, compaction relocating payload bytes can no longer dangle a fetched value. Recommended:
  lift `G24` for recursive element types with a test that fetches, removes, and reads the
  fetched value after a growing append (bug-538's shape). Open Decision in plan-134-A.
- **Decoders.** No decoder code changes: the copies, moves and frees now apply to the `.mfb`
  bodies of `json` and `regex` like any user code.

Risk: the collection kinds added for reaching types change the walker's kind set (goldens), and
an in-place arm missed in the census leaks (not corrupts). The live_bytes assertions per arm catch
both.

## Phases

### Phase 1 — census and RED

- [x] List every in-place and rebuild arm that discards an element
      (`grep -rn "fn try_inplace_\|fn lower_.*remove\|fn lower_.*set" src/codegen/collection
      --include='*.rs'`), with whether it frees the old element today; record the count. —
      **33 functions, 17 discard an element, 0 free anything of it.** (Read-only sweep of every
      body; `G24` lines re-checked by `grep -n "G24\|type_participates_in_cycle" …`.)
      - Helpers:
        - `lower_map_set_in_place`: overwrites, or leaves dead slack.
        - `lower_map_remove_key_in_place`: shifts entries; the data is left as slack.
        - `lower_list_set_in_place`: overwrites or shifts.
        - `lower_list_remove_at`: rebuild; skips the hole and byte-copies survivors.
        - `lower_list_remove_at_in_place`: compacts over the hole.
      - In-place arms: `try_inplace_{remove_key,record_field_remove_key,record_field_remove_at,
        record_field_set_remove,record_field_set,state_remove_key,state_set,state_remove_at,
        state_set_remove,set,remove_at,set_remove}_assign`.
      - Only the three removeAt arms decline a recursive element (`G24`, `type_participates_in_cycle`
        at `builder_inplace_assign.rs:434`, `:1111`, `:2006`). `try_inplace_set_assign` and the
        map/set arms have no gate.
      - Rebuilding builtins (`func_set.rs::lower_set`, `func_remove_at.rs`, `func_remove_key.rs`,
        `func_remove.rs`) free nothing of the element. Their `free_intermediate_collection` /
        `emit_free_pre_grow_buffer` frees return early for any non-flat type (`if
        !self.is_freeable_flat_value(type_)`).
      - The other 16 (append, insert, prepend, splice, concat, set add, literal) discard nothing.
- [x] Measure whether a `List OF json::Json` declared in user code has a walker kind (build it
      with `--ncode`, look for the kind in `_mfb_rt_graph_drop`'s dispatch). — Yes for a type
      that takes part in a cycle, no for one that only reaches one. `/tmp/p134-h-kind`
      (`LET xs AS List OF json::Json`, `LET rs AS List OF Rep` for `TYPE Rep / child AS
      json::Json`), `mfb build -ncode` → the per-type shims (one per walker kind) are
      `List_OF_json_Json`, `Map_OF_String_TO_json_Json`, `json_JsonArr`, `json_JsonObj` and
      `json_Json`. No kind for `Rep` or `List OF Rep`. **Gap confirmed:** a reaching-but-not-
      participating type has no kind, so `owns_graph` is false for it and it still leaks. Phase
      2's "walker kinds generated from `type_reaches_cycle`" is needed.
- [x] `tests/runtime/rt_recursive_value_collection_drops.rs` (register in `Cargo.toml`): per
      arm, a `--debug` program that builds a list/map of recursive values, discards elements that
      way, and asserts `arena.live_bytes` back at its start; plus the json and regex K = 1/2/4
      `assert_flat` cases in `rt_scope_drop_leaks.rs`. Confirm they fail today. — Eight cases.
      Each discards and re-adds one element per iteration (constant size). Built with
      `--debug`, each asserts final `live_bytes` equal at 1 000 and 2 000 iterations,
      `double_free_skips` 0 and equal output. Against the plan-134-G build, **7 failed**:

      | case | `live_bytes` growth / 1 000 discards |
      |---|---|
      | list `set` in place | +144 000 |
      | map `set` over a key | +144 000 |
      | map `removeKey` | +279 216 |
      | record-field list `set` | +656 000 |
      | record-field `removeAt` | +816 000 |
      | record-field map `removeKey` | +1 056 000 |
      | `List OF Rep`, element only reaches a cycle | +272 000 |

      `a_list_remove_at_frees_the_removed_element` already **passes**: the local removeAt takes
      the `G24` rebuild. E's `own_collection_payload_edges` gives the survivors their own graphs,
      and G's `Assign` graph-drops the old list, removed element included. It stays as a pin.
      The json/regex `assert_flat` cases: json K = 1 → 4 grew 1077 MB (fails); regex is already
      flat (Corrections).
- [ ] Added by plan-134-G: the json form of G's unbound-temp case,
      `acc = acc + len(json::stringify(json::parse(t)))` for `t = [1,{"a":[2,3]}]`, as an
      `assert_flat` case in `rt_scope_drop_leaks.rs` (400 000 / 800 000). Measured after G:
      `--debug`, 1 000 iterations → `alloc_calls 200003`, `free_calls 166003`,
      `live_bytes 2912000`, doubling exactly at 2 000. The bound form
      `LET v AS json::Json = json::parse(t)` leaks the same `2912000`. So the leak is 34 blocks
      left inside `json::parse`'s helpers, not the statement-scope temp (a user-type unbound temp
      is exactly flat: 5 002 allocs, 5 002 frees). Find which helper arms leave them.

Acceptance: census complete; the tests fail on main.
  Check: `cargo test --release --test rt_recursive_value_collection_drops` → failed (est. 3 min).
Commit: —

### Phase 2 — element frees

- [ ] Collection drop through the walker for the class; walker kinds generated from
      `type_reaches_cycle` if Phase 1 showed a gap.
- [ ] Each discarding arm from the census drops the old element graph.
- [ ] `G24` per the Open Decision, with the fetch-remove-grow-read test.
- [ ] Tests: Phase 1 passes; every plan-134 runtime test passes.

Acceptance: element discards return live_bytes; the decoders are flat.
  Check: `cargo test --release --test rt_recursive_value_collection_drops --test
  rt_scope_drop_leaks` → passed (est. 12 min).
Commit: —

### Phase 3 — measurements, docs, close-out

- [ ] `tools/recursive-value-bench/run.sh target/release/mfb` → record the full "after" table in
      plan-134-A §2.1 (every row), including speed against the budget.
- [ ] Re-measure the plan-134-D speed budget the owner deferred here (2026-09-13): medians of 5
      `json_repeat` / `regex_repeat` K=1 against the pre-plan 0.25 s / 0.27 s (budget +25 %), with
      RSS, and report them to the owner before merging — at D they were 0.24 s / 0.43 s; at E
      0.25 s / 0.89 s (RSS 212 MB / 2.39 GB). Include plan-134-E's cause: `#regex_run` stores
      `root` and the current repeat as placeholder fields in every `__regex_Choice` /
      `__regex_ContRep`, which value semantics now copies — the remedy is a regex helper rewrite
      (plan-134-E Corrections), which the owner deferred.
- [ ] Add a `json::get` / `json::getOr` path-walk probe to `tools/recursive-value-bench/` and
      measure it before and after plan-134 (median of 5): since plan-134-D the path walk copies the
      subtree at each step (`current = value`, `nextValue = current`, `current = nextValue` in
      `#json_get`/`#json_getOr`, plan-134-D Corrections), which `json_repeat` never exercises.
      Include the numbers in the owner report.
- [ ] Doc sync: `mfb spec memory arenas` ("Scope-Drop Frees": recursive values are freed by a
      per-type non-recursive drop; construction stores copy them); `mfb spec memory heap-values`
      (non-flat pointer fields are owned children); `mfb spec architecture native` (the
      copy-insertion paragraph, `src/docs/spec/architecture/06_native.md`); `.ai/collections.md`
      (the bug-601 GOTCHA is history; `G24`'s new status); `.ai/codegen-invariants.md` (the
      recursive-types sentence). Render each with `mfb spec …`.
- [ ] Bugs: bug-536 Shape C fixed by plan-134 (doc updated and archived — shapes A/B/B-2 were
      already fixed); bug-601 archived if D closed its last row; `planning/bug-backlog.md`.
- [ ] plan-133-A (a peer plan) proposed an `#[ignore]`d soak case waiting on Shape C: tell its
      owner in the landing report; do not edit that plan.

Acceptance: every §2.1 "after" row recorded; docs render; bug docs archived.
  Check: `mfb spec memory arenas` renders the new paragraph (est. 1 min).
Commit: —

## Validation Plan (plan-134, run once here)

- Tests: every `rt_recursive_value_*` file, the shape-C cases in `rt_scope_drop_leaks.rs`, the
  unit tables in C, B and F.
- Coverage check: the new walkers are emitted only for modules with recursive types; confirm
  `tests/byte-identity/json` and `tests/byte-identity/regex` goldens contain
  `_mfb_rt_graph_copy` and `_mfb_rt_graph_drop` (grep the regenerated `.ncode`), so the gate
  actually covers them.
- Runtime proof: `tools/recursive-value-bench/run.sh` on macOS and box 2223; the value tests'
  programs via `.cmd` on box 2230.
- Final gate (once): `cargo test --release --no-fail-fast -- --skip artifact_gate_all` (est. 60
  min), `bash scripts/artifact-gate.sh target/release/mfb all` (est. 15 min), `bash
  scripts/test-accept.sh target/release/mfb /tmp/p134-accept` (est. 15 min) — run
  sequentially (the gate and test-accept share the tree lock).

## Open Decisions

- `G24` — see plan-134-A (recommended: lift for recursive element types).

## Corrections

- **Prerequisite re-run** (2026-09-13): `ls planning/completed/plan-134-G-*` → one file — MET.
- **`regex_repeat` is already flat after plan-134-G, so its RED case cannot fail.**
  `cargo test --release --test rt_scope_drop_leaks -- json_parse_of_one regex_find_all
  unbound_recursive_json` → `a_repeated_regex_find_all_runs_at_constant_rss ... ok`.
  The other two fail:
  - `c_recursive_json_repeat`: peak RSS grew 1077 MB between K = 1 and 4 (528 → 1605 MB);
  - `c_recursive_json_unbound_temp`: grew 5345 MB between 400k and 800k.

  The regex case stays as the regression pin for the goal. Phase 1's "confirm they fail" holds
  for json only.
- **`STATE` arms are outside the class.** A resource's `STATE` holding recursive values is
  resource-bearing: `type_contains_resource` walks the `STATE` clause, so `needs_graph_copy` and
  `owns_graph` are false for it. The three `try_inplace_state_*` discarding arms leak as before
  and get no per-arm case here.
- **A set cannot hold a recursive value, so the set arms need no case.** Measured, not
  assumed. `/tmp/p134-h-set` (`LET s AS Set OF Node = Set OF Node { Node[kids := [], tag := 1] }`)
  → `error[2-203-0061 TYPE_REQUIRES_COMPARABLE]: Set element type requires a comparable type,
  got Node`. The three set-remove discarding arms can therefore never see a recursive element.

## Summary

Finishes the drop half for collections and closes bug-536 Shape C with measured decoder numbers.
Its risk is a discarding arm missed by the census — a leak, caught per arm by live_bytes — not
corruption, because the walker and its registration were proven in F and G.
