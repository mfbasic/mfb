# plan-134-I: The regex matcher by node index — no pattern-graph copies per backtracking step

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-134-H

Added during plan-134's final gate (a letter no earlier letter covered). The full release suite
on the merged tree failed `rt_regex_bounds::find_all_cost_is_bounded_for_the_whole_call` — the
DEC-02 denial-of-service bound — because the matcher now copies and frees pattern subtrees on
every backtracking step. This letter removes those copies by making the matcher refer to pattern
nodes by index. The test is right; the matcher's data shape is what value semantics cannot afford.

Behavioural outcome: **`findAll("(a|aa){1,16}b|a")` over 100 `a`s finishes inside the test's 8 s
deadline again** (pre-plan 1.08 s), with every regex answer unchanged.

References: `src/codegen/builtins/regex/helper_run.rs` (the matcher), `helper_try_at.rs`,
`helper_search_from.rs`, `helper_compile.rs`, `helper_simple_match_at.rs`,
`helper_is_simple_node.rs`, `helper_required_first_cp.rs`, `mod.rs` (the `__regex_*` records);
`tests/runtime/rt_regex_bounds.rs` (the pinned corpus, "same order, same answers");
plan-134-E Corrections (the cause, first recorded) and plan-134-H Phase 3 (the speed report).

Prerequisites: plan-134-H complete (`ls planning/completed/plan-134-H-*`).

## 1. Goal

- `cargo test --release --test rt_regex_bounds` → every case passes, including
  `find_all_cost_is_bounded_for_the_whole_call` inside its 8 s deadline.
- Every regex answer is unchanged: every `rt_regex_*` runtime suite, the regex rt-behavior
  fixtures (`test-accept.sh`), and the plan-134 value tests.
- `regex_repeat` K = 1 and the `rt_regex_bounds` findAll program are re-measured against the
  pre-plan build (`/tmp/p134-base`, `cc1012bdc`) and reported.

### Non-goals

- The parser keeps building the `__regex_Node` tree; only what the MATCHER holds changes.
- No change to the regex surface, error codes, budgets or exploration order.

## 2. Current State (measured 2026-09-13)

- `rt_regex_bounds`'s findAll program: pre-plan **1.08 s**, 2.06 GB RSS; after plan-134-H
  **30.57 s**, 2.5 MB RSS (`/usr/bin/time -l`, `/tmp/p134-rb` and `/tmp/p134-rb-base`).
- `--debug` arena counters for the same program: pre-plan `alloc_calls 7537843`,
  `alloc_bytes 611069712`; after H `alloc_calls 307429746`, `alloc_bytes 73713834656`,
  `free_calls 307429736` — 41× the allocations and 120× the bytes, all freed.
- Cause, read in `helper_run.rs`: every `__regex_Choice` push stores pattern subtrees by value —
  `alt` / `node` (a `__regex_Node`), `rep` / `repRec` (a `__regex_Repeat` holding its `child`
  subtree), `cont` (a `__regex_Cont` chain whose `ContSeq` links hold `parts AS List OF
  __regex_Node` and whose `ContRep` links hold the repeat again), and `root` placeholders (the
  whole pattern). Continuation steps build `__regex_ContSeq[seqNode.parts, …]` and
  `__regex_ContRep[repRec, …]`, and `node = collections::get(altNode.opts, i)` copies a subtree.
  Since plan-134-D/E each of these is a graph copy; since G/H each is freed on pop.
- The tree is read outside the parser only by `__regex_tryAt` (`prog.root` → `__regex_run`) and
  `__regex_searchFrom` (`__regex_requiredFirstCp(prog.root)`); `__regex_Cont*` exist only for
  the matcher (`grep -rn "prog\.root\|__regex_Cont" src/codegen/builtins/regex`).

## 3. Design

- **Flatten once, at compile.** `__regex_compile` turns the parser's tree into parallel
  integer tables plus a leaf table, `start` (the root op's index) and `firstCp` (today's
  `__regex_requiredFirstCp(root)`, computed once). `__regex_Program` becomes
  `{kinds, opA, opB, opC, kids, leaves, start, firstCp, groups, names}` — no tree, so a program
  is a flat value.
- **An op is one index into `List OF Integer` tables** — reading it is a scalar load, never a
  record copy. `kinds`: 1 a simple leaf (`opA` = its index in `leaves`), 2 an anchor leaf (`opA`
  = leaf index), 3 Concat and 4 Alt (`opA` = first index in `kids`, `opB` = count), 5 a greedy
  and 6 a lazy Repeat (`opA` = child op, `opB` = lo, `opC` = hi), 7 Group (`opA` = child op, `opB`
  = slot). `kids` holds a Concat's parts and an Alt's options as op indices.
- **Leaves** (`leaves AS List OF __regex_Leaf`, a flat union of `__regex_Lit | __regex_Any |
  __regex_Class | __regex_Anchor`) reach the matcher as a parameter, so a leaf is read with a
  `get` used only as a `MATCH` scrutinee over an immutable local — plan-86 E's borrow, no copy.
- **The continuation is an index into an append-only frame table** (`List OF Integer`, five
  integers per frame, Corrections): sequence `(1, concatOp, nextIdx, 0, below)`, capture close
  `(2, slot, 0, 0, below)`, repeat `(3, repeatOp, count, startPos, below)`; -1 is "done". Frames
  are never rewritten, so a saved index resumes exactly that continuation.
- **A choice point is eight integers** `(kind, alt, rep, cont, pos, i, count, p)` in a `choices`
  table with its capture list in a `snaps` table (Corrections); a pop lowers `pending`, a push
  overwrites the slot above. `alt`/`rep` are op indices, `cont` a frame index.
- **Same exploration.** The loop keeps its structure — the same choice kinds, the same order —
  with `node` an op index and every `MATCH node` a dispatch on `op.kind`. The `__regex_Cont*`
  records, `__regex_isSimpleNode` and the node form of `__regex_simpleMatchAt` are replaced, not
  kept beside the new ones.

Risk: a translation slip changes an answer. The pinned corpus (`rt_regex_bounds` "same order,
same answers") and the regex rt-behavior fixtures are the proof; the byte-identity goldens for
regex change by design and are regenerated after the corpus is green.

## Phases

### Phase 1 — RED and census

- [x] Record the failing case and the measurements of §2 in the plan (the test fails today on the
      plan-134-H build; no new test is needed — the DEC-02 bound is the RED). — §2. The RED is
      from the full suite on the merged tree (`cargo test --release --no-fail-fast --
      --skip artifact_gate_all`, log `/tmp/p134-final-tests.log`):
      `find_all_cost_is_bounded_for_the_whole_call ... FAILED` — "did not finish within 8s —
      findAll spent a fresh backtracking budget on every match". The same program timed and
      counted on the pre-plan build and the H build is recorded in §2.
- [x] Census every regex helper that reads a `__regex_Node` or a `__regex_Cont*` outside the parser
      (`grep -rln "__regex_Node\|__regex_Cont" src/codegen/builtins/regex`), each marked parser /
      matcher; the matcher set is what Phase 2 rewrites. — 13 files:
      - **parser, unchanged:** `helper_parse_alt`, `helper_parse_atom`, `helper_parse_class`,
        `helper_parse_concat`, `helper_parse_escape_atom`, `helper_parse_named_group`,
        `helper_parse_paren`, `helper_parse_quant_suffix` (they build the tree);
      - **matcher, rewritten:** `helper_run`, `helper_simple_match_at`, `helper_is_simple_node`;
      - **compile-time analysis, kept on the tree:** `helper_required_first_cp`, moved to run
        once in `__regex_compile`;
      - **records:** `mod.rs`.

      The tree is also read through `prog.root` by `helper_try_at` and `helper_search_from`
      (`grep -rn "prog\.root"`), which Phase 2 changes.

Acceptance: the census is total; the bound fails on the H build.
  Check: `cargo test --release --test rt_regex_bounds -- find_all_cost_is_bounded` → failed.
Commit: ad1a4d1d6

### Phase 2 — the flat program and the index matcher

- [x] `__regex_Op`, `__regex_Leaf`, the new `__regex_Program` and `__regex_Choice` records
      (`mod.rs`); `__regex_compile` flattens (a new `__regex_flatten` helper). — Two corrections
      (see Corrections), measured by the checks in the last task:
      - no `__regex_Op` record: an op is an index into the integer tables of §3;
      - no `__regex_Choice` record: choice points are integer tables.

      As built: `__regex_Leaf` (a union of Lit/Any/Class/Anchor),
      `__regex_Program{kinds, opA, opB, opC, kids, leaves, start, firstCp, groups, names}`, and
      `helper_flatten.rs` called from `__regex_compile`.
- [x] `__regex_run` by op index with the integer continuation; `__regex_tryAt`,
      `__regex_searchFrom` and the simple-leaf helper take the flat program. —
      `__regex_run(prog, leaves, start, caps0, ctx)`; `__regex_tryAt` passes `prog.leaves`;
      `__regex_searchFrom` reads `prog.firstCp`;
      `__regex_simpleMatchAt(leaves, at, pos, ctx)`.
- [x] Remove the matcher-only records and helpers the census names. — Removed from `mod.rs`:
      `__regex_ContDone/ContSeq/ContCap/ContRep`, the `__regex_Cont` union, and
      `__regex_NoChoice/Choice` with the `__regex_Choices` union. Deleted:
      `helper_is_simple_node.rs`, `helper_required_first_cp.rs`.
- [x] Tests: `rt_regex_bounds` passes; every `rt_regex_*` suite passes; the plan-134 value tests
      pass (the drop-symmetry case that names `#regex_run`'s `cont` and `node` locals and F's
      builtin-kinds unit test follow the removed types — record the evidence). —
      - `--test rt_regex_bounds --test rt_regex_span` → `5 passed`, `3 passed`,
        including `find_all_cost_is_bounded_for_the_whole_call` and the pinned corpus
        `matching_semantics_are_unchanged`.
      - `rt_native_regex_parser_depth` → 3 passed; `rt_recursive_get_alias`,
        `rt_recursive_map_transfer`, `rt_recursive_value_copy_depth`,
        `rt_imported_type_qualified_name` → ok; `rt_scope_drop_leaks` (b536/b562 skipped) →
        `131 passed`.
      - `rt_recursive_value_collection_drops` 9, `rt_recursive_value_copies` 3,
        `rt_recursive_value_drop_symmetry` 6 passed.
      - Drop-symmetry evidence: the old case failed with `drop_regex_stack: the hook on
        `stack` copied nothing (4167 alloc_calls plain, 4167 hooked)`. It now hooks
        `node`/`work`/`parts`, the regex locals the NIR still types as recursive
        (`mfb build -nir`: `node AS #regex_Node` ×18, `work` and `parts AS List OF
        #regex_Node`).
      - Unit tests: `cargo test --release --bin mfb -- graph_drop_edges_match
        collect_last_use_moves` → `8 passed` (both corrected tests, see Corrections).

Acceptance: the DEC-02 bound holds and no regex answer changes.
  Check: `cargo test --release --no-fail-fast --test rt_regex_bounds --test rt_regex_span`
  plus every other `rt_regex_*` target (`ls tests/runtime/rt_regex_*`) and the plan-134 value
  suites → passed (est. 10 min).
Commit: 5d052ac7b

### Phase 3 — goldens, measurements, docs

- [x] Artifact gate; expected diffs: `byte-identity/regex` (and any fixture importing regex).
      Localize per function, regenerate, re-gate. — `bash scripts/artifact-gate.sh
      target/release/mfb all` → `2019 golden(s) checked, 155 diff(s)` in 39 fixtures. That is
      wider than predicted, because the gate also covers the final-gate fixes since the H
      goldens.

      Localized per function (`/tmp/p134-i-localize/run.sh`), dumping with the letter-H compiler
      and this build:
      - `bits`, `control-flow-if`, `control-flow-match` and `list-ops-codegen-rt` change only
        `_mfb_fn_main`; `audio` changes 10 functions and `crypto-ec-valid` 70; no function was
        added or removed.
      - Instruction diffs of `bits`/`control-flow-if` `main` and `crypto` `hmac` show one change
        (`/tmp/p134-i-localize/fndiff.py`): a call's error branch now parks x0–x3, drops the
        statement's owned temp (`_mfb_rt_drop_owned_string` /
        `_mfb_rt_drop_owned_collection`, the call the success path makes next) and reloads.
        The frame grows 32 B for the parked result. This is `d2acb15e6`.
      - The six regex fixtures also change their `.ir`, because this letter changed the
        records.

      Regenerated: `scripts/regen-native-goldens.sh target/release/mfb <39 dirs>` → `159
      golden(s) rewritten, 0 failure(s)`; `scripts/sync-goldens.sh` over the six regex fixtures
      → `synced 18 golden file(s) across 6 test(s)`. Re-gated `all` → `2019 golden(s) checked,
      0 diff(s)`.
- [x] Re-measure against the pre-plan build: `rt_regex_bounds`' findAll program and `regex_repeat`
      K = 1 (medians of 5, RSS); record them in plan-134-H's speed report and plan-134-A's
      "After" table. — `bash /tmp/p134-i-measure/run.sh`, idle machine, the two compilers back
      to back, same output from both:
      - findAll: pre-plan 0.77 s / 2.06 GB → 0.55 s / 163 MB;
      - `regex_repeat` K=1: pre-plan 0.11 s / 345 MB → 0.04 s / 18 MB;
      - `--debug` `alloc_calls` 7 537 843 → 644.

      `rows.sh` (single runs):
      - `regex_repeat` K = 1 / 2 / 4 → 0.03 / 0.07 / 0.12 s, 18.3 MB flat;
      - `regex_chain group:499999` → exit 0, 0.84 s, 671 MB (H: 11.4 s, 2.22 GB);
      - `group:500001` → exit 3 at the pending limit.

      Recorded in plan-134-H's speed report and in plan-134-A's "After" rows.
- [x] Doc comments of the rewritten helpers describe the flat program and the integer
      continuation. — In `5d052ac7b`:
      - the module doc and body header of `helper_run.rs` cover the frame, choice and
        snapshot tables and their bounds;
      - `helper_flatten.rs` gives the op encoding;
      - `helper_simple_match_at.rs` notes the borrowed leaf.

      The same commit updates `src/docs/spec/stdlib/01_regex.md` (op and frame tables),
      `.ai/codegen-invariants.md` and the `recursive-value-bench` README and program comments.

Acceptance: goldens regenerated and re-gated clean; measurements recorded.
  Check: the gate (est. 15 min); the measurement script (est. 3 min).
Commit: 47feff86c (goldens), 1ce7ef468 (measurements)

## Validation Plan

- Tests: `rt_regex_bounds` (the RED), every `rt_regex_*`, the regex rt-behavior fixtures in
  `test-accept.sh`, the plan-134 value suites.
- Final gate: plan-134's (run once after this letter, per plan-134-H's Validation Plan).

## Open Decisions

- None: the only alternative (raising the test's deadline) weakens a security bound and is not
  taken.

## Corrections

- **§3 refined before Phase 2: integer op tables, not a flat op record.** The first draft
  held ops in `List OF __regex_Op` (a flat record carrying `kind`, a leaf union, a `kids` list and
  the scalars). A `collections::get` of a flat record element is still a `copy_flat_block` — an
  allocation and a free on every node visit, the per-step cost this letter exists to remove, and
  a `kids` list inlined in each record makes each copy variable-length. Parallel `List OF
  Integer` tables read an op with scalar loads, and the leaf table is borrowed by plan-86 E, so a
  node visit allocates nothing.

- **§3 refined in Phase 2: the continuation and the choice stack are integer tables too, not a
  per-choice `List OF Integer` and a linked chain.** §3 kept a linked `__regex_Choice` chain whose
  records copied the continuation list on every push. A pop (`stack = c.nxt`) out of a
  recursive chain is a move only if plan-134-C proves it one, and every push copies the frame
  list. As built instead:
  - a continuation is an index into an append-only table of five-integer frames (tag, three
    operands, the frame below);
  - a choice point is eight integers in `choices`, with its capture list in `snaps`;
  - a pop lowers `pending` and a push overwrites the slot above.

  A choice therefore saves a frame index. Frames are never rewritten, so the saved continuation
  is exact. Evidence that nothing per step allocates: `--debug` of `rt_regex_bounds`' findAll
  program gives `alloc_calls 644`, against `7537843` pre-plan and `307429746` after
  plan-134-H.
- **Leaves: `Lit`/`Any`/`Class` share op kind 1, anchors kind 2, both through `__regex_Leaf`.**
  The records keep their `__regex_Node` order in the new union, because
  `TypeModel::union_variant_tags` is keyed by the variant record (`builder/mod.rs`), so each
  record's tag is the same in both unions. The leaf is read by `__regex_simpleMatchAt(leaves,
  at, pos, ctx)` and by the anchor arm, through a `get` that only a `MATCH` reads over a
  parameter — plan-86 E's borrow.
- **`__regex_requiredFirstCp` is gone, not kept on the tree.** The census row said "kept on the
  tree, moved to compile". Its three rules (a non-folding `Lit`; `Concat` and `Group` pass to
  their first child; otherwise -1) are a loop over the flat tables at the end of
  `__regex_flatten`, stored as `__regex_Program.firstCp`. Keeping a recursive helper for one
  compile-time walk would have kept a second reader of the tree.
- **The move-analysis unit test moved off the matcher.**
  `last_use::tests::collect_last_use_moves_covers_the_regex_matcher_views` asserted three things
  in `#regex_run` and `#regex_isSimpleNode`: that `stack = c.nxt` and `cont = seqCont.nxt` are
  moves out of an owning MATCH view, and that a MATCH on a parameter borrows. Four answers:
  1. It was written by plan-134-D as the speed gate for the matcher's hot stores.
  2. It protects plan-134-C's field moves and borrowed views.
  3. Nothing else depends on it.
  4. This letter deletes both functions and both shapes (`grep -rn "__regex_Cont\|isSimpleNode"
     src/codegen/builtins/regex` → none).

  The same three shapes now sit in a self-contained `CHAIN_BACKTRACKER` program, so the
  behaviour stays pinned: `collect_last_use_moves_covers_the_chain_backtracker_views ... ok`.
- **F's builtin-kinds unit test names the recursive regex types that still exist.**
  `graph_drop_edges_match_the_copy_edges_for_the_builtin_types` expected `#regex_Cont` and
  `#regex_Choices`. Measured after this letter, the model's recursive kinds are
  `{"#regex_Alt", "#regex_Concat", "#regex_Group", "#regex_Node", "#regex_Repeat", "List OF
  #regex_Node"}`. The expectation is now `#regex_Node`, `#regex_Concat`, `#regex_Repeat`.
- **`tools/recursive-value-bench` `regex_chain` re-checked.**
  - `group 499999` → exit 0; `group 500001` → exit 3, `raised=regex: pattern too complex …`
    (the pending limit, as before).
  - `simple` `--debug` `alloc_calls`: 2108 / 27122 / 252144 at r = 1 / 1000 / 10 000, which is
    25.0 per hit on both intervals.

  Its comments and the README no longer describe the chains.

## Summary

Value semantics made every copy real; the matcher was written when copies of its pattern
references were free. Referring to nodes by index restores the pre-plan cost without giving back
any of plan-134's guarantees.
