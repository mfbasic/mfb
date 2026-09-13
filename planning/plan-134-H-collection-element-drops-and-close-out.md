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

- [x] Collection drop through the walker for the class; walker kinds generated from
      `type_reaches_cycle` if Phase 1 showed a gap. — Gap shown, so: `graph_drop_kind_names` →
      `TypeModel::graph_drop_kinds`, seeded from the model and the module's spelled types
      (Corrections). `graph_drop_kind` reads it, so every G ownership gate now covers
      reaching-only types (`Holder`, `Map OF String TO Node`, `List OF Rep`).
- [x] Each discarding arm from the census drops the old element graph. — Element drops, via
      `emit_drop_list_element` / `emit_drop_entry_value` and the new
      `_mfb_rt_graph_drop_edges`, at the four helpers every discarding arm reaches:
      - `lower_list_set_in_place`;
      - `lower_list_remove_at_in_place`;
      - `lower_map_set_in_place`;
      - `lower_map_remove_key_in_place`.

      Collection blocks:
      - Intermediate and pre-grow collection blocks of recursive values are freed shallowly
        (`free_intermediate_collection`).
      - The three `STATE` arms are outside the class, and the set arms are unreachable
        (Corrections).
- [x] `G24` per the Open Decision, with the fetch-remove-grow-read test. — Lifted, as
      recommended, at all three gates (`builder_inplace_assign.rs`):
      - `get` returns an owned copy since plan-134-D;
      - the compaction now frees the removed element.

      Pinned by `a_fetched_recursive_element_survives_an_in_place_remove_and_a_growing_append`
      → `fetched=2 len=52 first=4`, `double_free_skips 0`.
- [x] Tests: Phase 1 passes; every plan-134 runtime test passes. — Plus the fix the json leak
      exposed: a graph moved out by `RETURN` reports `already_standalone` (Corrections;
      `--debug` bound `json::parse` → 55 003 allocs / 55 003 frees, `live_bytes 0`).

Acceptance: element discards return live_bytes; the decoders are flat.
  Check: `cargo test --release --test rt_recursive_value_collection_drops --test
  rt_scope_drop_leaks` → passed (est. 12 min).
  Result (one binary):
  - `rt_recursive_value_collection_drops` 9 / 9;
  - `rt_scope_drop_leaks` 128 / 128, including `json_repeat` K 1 → 4 and the json unbound
    temp;
  - the B–G value suites 17 / 17 (`rt_recursive_value_copies` 3, `…construction_copies` 4,
    `…drop_symmetry` 6, `…copy_depth` 3, `rt_error_value_copies` 1).
Commit: —

### Phase 3 — measurements, docs, close-out

- [x] `tools/recursive-value-bench/run.sh target/release/mfb` → record the full "after" table in
      plan-134-A §2.1 (every row), including speed against the budget. — `bash
      tools/recursive-value-bench/run.sh target/release/mfb` → every program exit 0 except
      `regex_chain group:500001`, which raises by design (exit 3). Recorded in plan-134-A's
      "After" table:
      - peak RSS is flat across K for `json_repeat` (173 MB), `regex_repeat` (18 MB) and
        `json_get` (114 MB);
      - the shape-C repros are unchanged at 1.08 / 1.03 MB.

      Speed against the budget: the next task's medians-of-5 comparison. This run's times
      are single runs.
- [x] Re-measure the plan-134-D speed budget the owner deferred here (2026-09-13): medians of 5
      `json_repeat` / `regex_repeat` K=1 against the pre-plan 0.25 s / 0.27 s (budget +25 %), with
      RSS, and report them to the owner before merging — at D they were 0.24 s / 0.43 s; at E
      0.25 s / 0.89 s (RSS 212 MB / 2.39 GB). Include plan-134-E's cause: `#regex_run` stores
      `root` and the current repeat as placeholder fields in every `__regex_Choice` /
      `__regex_ContRep`, which value semantics now copies — the remedy is a regex helper rewrite
      (plan-134-E Corrections), which the owner deferred. —
      `bash /tmp/p134-h-speed.sh` compared, on this machine in one run, the pre-plan compiler
      built from `cc1012bdc` (`git archive` → `/tmp/p134-base`) with the plan-134-H worktree
      build. Medians of 5; the first run of each includes first-launch cost:

      | probe | before (pre-plan) | after (plan-134-H) |
      |---|---|---|
      | `json_repeat` K=1 | **0.10 s**, 204 MB | **0.18 s**, 173 MB |
      | `regex_repeat` K=1 | **0.11 s**, 345 MB | **0.57 s**, 18 MB |

      **Budget:**
      - json: 0.18 s is within the plan-134-A absolute budget (0.25 s × 1.25 = 0.31 s), but
        +80 % over today's same-machine baseline.
      - regex: 0.57 s is over both — 0.34 s absolute, and 5.2× the same-machine baseline.

      **Cause unchanged since E:** every `__regex_Choice` / `__regex_ContRep` push stores the
      pattern root and the current repeat as fields, and value semantics copies those graphs.
      H's frees add the matching drops (RSS 345 MB → 18 MB). The remedy is still the deferred
      regex helper rewrite. Output identical (`chars=100000 hits=10000`, `bytes=480003`).
- [x] Add a `json::get` / `json::getOr` path-walk probe to `tools/recursive-value-bench/` and
      measure it before and after plan-134 (median of 5): since plan-134-D the path walk copies the
      subtree at each step (`current = value`, `nextValue = current`, `current = nextValue` in
      `#json_get`/`#json_getOr`, plan-134-D Corrections), which `json_repeat` never exercises.
      Include the numbers in the owner report. — `programs/json_get`: K
      `json::get(doc, ["a", "b", "c", "leaf"])` walks past a 20 000-item sibling array. Sizes
      1 / 10 / 100 (in `run.sh` and the README); first tried at 1 000, then cut to 100 because
      it took 15 s at K = 100.

      | K | before (pre-plan) | after (plan-134-H) |
      |---|---|---|
      | 1 | 0.07 s, 135 MB | 0.13 s, 114 MB |
      | 10 | 0.32 s, 857 MB | 0.95 s, 114 MB |
      | 100 | 3.78 s, **8.08 GB** | 5.63 s, **114 MB** |

      Before, every step's alias leaked, and RSS grew linearly to 8 GB at K = 100. After, the
      per-step subtree copies are freed and RSS is flat, but each step costs a copy: ~3× at
      K = 10, ~1.5× at K = 100 (where the old leak's page faults dominated). Output identical
      (`found=K`).
- [x] Doc sync: `mfb spec memory arenas` ("Scope-Drop Frees": recursive values are freed by a
      per-type non-recursive drop; construction stores copy them); `mfb spec memory heap-values`
      (non-flat pointer fields are owned children); `mfb spec architecture native` (the
      copy-insertion paragraph, `src/docs/spec/architecture/06_native.md`); `.ai/collections.md`
      (the bug-601 GOTCHA is history; `G24`'s new status); `.ai/codegen-invariants.md` (the
      recursive-types sentence). Render each with `mfb spec …`.
      - **arenas:** the drop walker and move paragraphs landed in plan-134-G's Phase 3.
        Renders "frees it through the module's one drop walker, _mfb_rt_graph_drop …".
      - **heap-values:** a non-flat pointer field to a recursive graph is an "owned child",
        deep-copied with and freed with its parent. Renders at line 69.
      - **architecture native:** the "one exception to pointer-free" sentence — copy,
        move-at-last-read and drop, including in-place element discards. Renders at line 421.
      - **`.ai/collections.md`:** the bug-601 GOTCHA's "construction stores still alias" is
        history (E, G, H named). `G24` is marked lifted, with the aliasing rule it encoded
        kept. The reaching-type lesson now also names the drop kinds.
      - **`.ai/codegen-invariants.md`:** in-place element discards are freed since H, and
        intermediates are freed shallowly. The prose was updated in G's Phase 3 and here.

      Rendered with the rebuilt binary: `target/release/mfb spec memory heap-values`,
      `… memory arenas`, `… architecture native`.
- [x] Bugs: bug-536 Shape C fixed by plan-134 (doc updated and archived — shapes A/B/B-2 were
      already fixed); bug-601 archived if D closed its last row; `planning/bug-backlog.md`.
      - **bug-536:** status set to "CLOSED 2026-09-13 — shape C fixed by plan-134 (letters
        A–H)", with the measurements. Archived by `git mv` to
        `bugs/completed/bug-536-scope-drop-leaks-recursive-types-return-constructor-string-temps.md`.
      - **bug-601:** already archived — `ls bugs/completed/ | grep bug-601` →
        `bug-601-mut-copy-of-a-non-flat-list-aliases-its-source.md`; nothing open in `bugs/`.
      - **`planning/bug-backlog.md`:**
        - "construction stores still alias until plan-134-E" is history;
        - "bug-536 has no actionable work" now reads CLOSED;
        - the shape-C bullet now reads FIXED by plan-134, with the numbers.
- [x] plan-133-A (a peer plan) proposed an `#[ignore]`d soak case waiting on Shape C: tell its
      owner in the landing report; do not edit that plan. — Note for the landing report, not an
      edit to plan-133-A (`planning/plan-133-A-browser-memory-diagnosis-and-soak-test.md`,
      untouched):
      - plan-133-A §1 plans `tests/runtime/rt_debug_soak.rs`, whose recursive-workload soak case
        "fails on today's compiler for the documented reason" (bug-536 Shape C).
      - Shape C is fixed by plan-134, so that case should now pass and can be re-enabled.
      - Its owner should re-run it after plan-134 merges.
      - `tests/runtime/rt_debug_soak.rs` does not exist in this branch (`ls`); whether it has
        landed on main is checked at the merge.

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
- **The walker kinds are seeded from the module, not just the type model.** A `List OF Rep` that
  only a local declares is never a component of a model record or union, so kinds derived from
  `TypeModel` alone miss it. `graph_drop_kind_names` (`builder_collection_layout.rs`) seeds the
  walk from the model's records and unions, globals, parameter and return types,
  `LET`/`FOR`/`FOR EACH` types, and constructor, collection-literal and union-wrap types. The
  list is stored as `TypeModel::graph_drop_kinds`, filled in `from_module` and
  `from_module_and_packages`. It puts the copy walker's kinds first, so their indices are shared.
- **An inlined element needs a drop that frees its edges, not its bytes.** A record or union
  element lives inside the collection's block. `_mfb_rt_graph_drop_edges` is the same walker with
  the root entry's block free skipped (a `graph_drop_root` flag cleared at the first pop). A
  pointer payload still goes through `_mfb_rt_graph_drop`. Used by `emit_drop_list_element` /
  `emit_drop_entry_value` at four discard sites:
  - `lower_list_set_in_place`, after the bounds check;
  - `lower_list_remove_at_in_place`, entry-table branch;
  - `lower_map_set_in_place`, `found_handle`;
  - `lower_map_remove_key_in_place`, before the entry shift.

  After this, four of the seven RED arms passed: list `set`, map `set`, the reaching-type list,
  and the pinned list `removeAt`. The other four still grew by 82 000–256 000 B per 1 000
  iterations.
- **Intermediate and pre-grow collection blocks of recursive values are freed shallowly —
  G's deferred §2.1 row 7.** The remaining growth was blocks, not graphs.
  `free_intermediate_collection` (and `emit_free_pre_grow_buffer`, which delegates to it)
  returned early for any non-flat type. None of its blocks owns its elements' children, so
  none may be graph-dropped:
  - a grown-over buffer's payloads moved to the new one;
  - `lower_list_remove_at`'s and `lower_map_remove_key`'s products byte-copy survivors the
    source still owns;
  - a singleton byte-copies an item owned by its temp or local.

  They are now freed by `emit_shallow_block_free` when `owns_graph`.
- **A graph moved out by `RETURN` was re-materialized, leaking its top block — one per
  `json::parse`.** After the element and intermediate frees, 127 of 128 leak cases passed; the
  json unbound temp still grew 79 MB (400k → 800k). `--debug`, 1 000 iterations:
  - `stringify` of a pre-parsed document → `live_bytes 32`, flat;
  - bound `LET v = json::parse(t)` → `56003` allocs / `55003` frees, `live_bytes 32000`;
  - the unbound form → `live_bytes 32000`.

  So exactly one 32-byte block per parse. `-nir` shows `#json_parse` ends `return parsed.value`
  (`parsed AS #json_Node`). plan-134-G's `lower_returned_value` moved that field out
  (`release_moved_source`) but reported the result `already_standalone = false`.
  `emit_return_exit_inner` then re-materialized the data union's top block
  (`materialize_inline_value_in_arena`): the caller got a copy and the moved original leaked. A
  moved graph is its own block, so the move branch now reports `true`.
- **Goldens (not a listed task; the final gate needs them clean).** `bash
  scripts/artifact-gate.sh target/release/mfb all` → `2013 golden(s) checked, 10 diff(s)`: all
  `json_codegen_cover_rt` / `regex_codegen_cover_rt` `.ncode` on the five targets.
  Per-function diff against letter G's dumps (`/tmp/p134-ncode-diff.py`):
  - json 163 → 164 functions, `added: ['_mfb_rt_graph_drop_edges']`, changed
    `_mfb_rt_graph_drop` (more kinds) and `#json_parse`, `#json_parseArray`,
    `#json_parseArrayItems`, `#json_parseNumber`, `#json_parseObject`,
    `#json_parseObjectItems`, `#json_parseValue`, `#json_revive`;
  - regex 190 → 191 functions, the same `added`, changed `_mfb_rt_graph_drop` and 16 regex
    helpers — `#regex_compile`, `count`, `find`, `findAll`, `findAllMatches`, `findMatch`,
    `match`, `parseAlt`, `parseAtom`, `parseClass`, `parseConcat`, `parseEscapeAtom`,
    `parseNamedGroup`, `parseParen`, `parseQuantSuffix`, `replace`, `split`.

  Every changed function stores or discards recursive values. Regenerated →
  `10 golden(s) rewritten, 0 failure(s)`; re-gated json and regex → `7 golden(s) checked,
  0 diff(s)` each.

  Validation Plan coverage check:
  `grep -o '"symbol": "_mfb_rt_graph_[a-z_]*"'` over both regenerated dumps finds
  `_mfb_rt_graph_copy`, `_mfb_rt_graph_drop`, `_mfb_rt_graph_drop_edges` and
  `_mfb_rt_graph_stack_grow` once each, so the gate covers all four walkers.
- **A set cannot hold a recursive value, so the set arms need no case.** Measured, not
  assumed. `/tmp/p134-h-set` (`LET s AS Set OF Node = Set OF Node { Node[kids := [], tag := 1] }`)
  → `error[2-203-0061 TYPE_REQUIRES_COMPARABLE]: Set element type requires a comparable type,
  got Node`. The three set-remove discarding arms can therefore never see a recursive element.

## Summary

Finishes the drop half for collections and closes bug-536 Shape C with measured decoder numbers.
Its risk is a discarding arm missed by the census — a leak, caught per arm by live_bytes — not
corruption, because the walker and its registration were proven in F and G.
