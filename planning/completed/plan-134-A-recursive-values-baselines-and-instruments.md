# plan-134-A: Recursive values — baselines, instruments, and the unmeasured quantities

Last updated: 2026-09-13
Overall Effort: huge (>3d) — eight lettered sub-plans, A → H
Effort: medium (1h–2h)
Depends on: nothing

plan-134 makes a value of a **recursive type** behave like every other value: each owner
holds an independent copy, and the value is freed when its owner goes away. Today such a
value is never copied at an owning store and never freed (bug-536 Shape C). So every
recursive value in a program is shared, `MUT` copies of a list of them alias their source
(bug-601's remaining row), and the untrusted-input decoders built on them grow without
bound. The single behavioural outcome when plan-134 is done: **a loop that binds, stores,
returns or discards a recursive value runs at constant peak memory, and mutating a copy
never changes its source.**

This letter changes no compiler behaviour. It lands the measurement instruments the later
letters gate on, and turns the one quantity that could not be measured before writing into a
number.

References (read first):

- `bugs/bug-536-scope-drop-leaks-recursive-types-return-constructor-string-temps.md` —
  "USER DECISION (2026-09-06)" (Shape C is a plan, not a bug fix) and "Shape C is blocked on
  recursive COPY-insertion".
- `bugs/bug-601-mut-copy-of-a-non-flat-list-aliases-its-source.md` — the recursive-type row.
- `mfb spec language memory-semantics` (`src/docs/spec/language/14_memory-semantics.md`):
  §14.1 (a copy is independent), §14.2 (a copy may become a move when the source is not used
  again), §14.5 (a recursive edge is an owned child, not a shared pointer; cycles are
  impossible; "implementations may use iterative drop internally to avoid stack overflow"),
  §14.6 (containers own their elements).
- `mfb spec memory arenas` (`src/docs/spec/memory/04_arenas.md` "Scope-Drop Frees") — the
  copy-insertion + scope-drop model this plan extends to recursive values.
- `.ai/collections.md` (the bug-601 GOTCHA) and `.ai/codegen-invariants.md` (the records
  section: "no owning copy at a bind and no drop anywhere").

## Prerequisites

| Must be true | Command | Status (2026-09-13) |
|---|---|---|
| plan-132 is complete (the pointer-`String` records are flat, so recursive types are the only non-copyable class left) | `ls planning/completed/plan-132-*` → one file | MET |
| plan-130-C is complete (the `--debug` arena counters this plan measures with) | `ls planning/completed/plan-130-C-*` → one file | MET |
| bug-538's fix is present (`collections::get` deep-copies a recursive element; the only existing recursive copy caller) | `grep -n "bug-538" src/codegen/memory/owned.rs` → a hit in `materialize_owned_element` | MET |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every
> command and update every status before you continue, and again before you decide to stop.
> If you stop, report the current status of *all* prerequisites.

## 1. Goal (this letter)

- `tools/recursive-value-bench/` exists with a README and one command that prints, for every
  program in §2.2, peak RSS and wall time at the documented sizes. Its output on main matches
  the baselines in §2.1 to within 10 % (RSS) — the later letters' before/after numbers come
  from this one tool.
- The length of a `__regex_Cont` chain during `regex::findAll` over a 100 000-character
  subject is measured and recorded in §2.1 (it bounds the depth the letter-B walker must
  handle on a real input).

### Non-goals (explicit constraints, for all of plan-134)

- **Value semantics do not change for any program that is correct today.** Every observable
  output stays the same; only memory use and (within the letter-D budget) speed move.
- **No layout, ABI or format change.** Record, union (`{tag, size, block}`), collection header
  and `.mfp` layouts stay as specified in `mfb spec memory heap-values` / `collections`.
  Recursive edges stay separate arena blocks (pointer fields/payloads).
- **Flat values are untouched.** Nothing in plan-134 changes the codegen of a program that
  declares or imports no recursive type; letters say which fixtures are expected to diff.
- **Resources keep moving, never copying.** A value with a resource anywhere inside it
  (`type_contains_resource`) keeps today's alias/move behaviour everywhere.
- **Existing borrows stay unfreed** (plan-86 E borrow-`get`, `MATCH` scrutinees, `UnionExtract`
  aliases, `by_ref` slots, `FOR EACH` elements, parameters) — letter G audits each.
- Forbidden shortcuts (from bug-536 §Non-goals): making `is_freeable_flat_value` true for a
  recursive type (a one-block free leaks the children and mis-sizes); adding any free before
  letters D and E have made every owner hold a distinct graph (a double free); raising a leak
  test's threshold.

## 2. Current State

### How a recursive value is handled today (read, cited)

- **The class.** `type_participates_in_cycle` and `type_reaches_cycle`
  (`src/codegen/collection/layout/builder_collection_layout.rs`) decide it; neither type is
  `type_is_memcpy_copyable`, so `is_freeable_flat_value`
  (`src/codegen/engine/value/builder_values.rs`) is false for both.
- **No copy at an owning store.** `lower_value_owned` copies only when
  `value_needs_owning_copy(value) && is_freeable_flat_value(type)`; otherwise it claims the
  pending temp and returns the same pointer. `lower_returned_value`
  (`src/codegen/engine/control/builder_exits.rs`) has the same gate. Record construction
  (`emit_build_inlined_record`), `WITH` (`lower_with_update`,
  `src/codegen/memory/value/builder_value_semantics.rs`), union wrap
  (`emit_wrap_record_in_union`) and the collection payload writer
  (`emit_copy_payload_to_collection`) store a recursive sub-pointer verbatim.
- **No free anywhere.** The `NirOp::Bind` arm registers `ActiveCleanup::OwnedValue` only when
  `owns_freeable_value` (requires `is_freeable_flat_value`); `pending_temp_is_freeable`,
  the `Assign`/`StoreGlobal` old-value frees and the closure-capture free
  (`src/codegen/cleanup/owned/builder_owned_cleanup.rs`) share the gate.
  `emit_owned_value_drop` frees one block; `_mfb_rt_drop_owned_collection`
  (`error/emission/park_error_helper.rs::lower_drop_owned_collection_helper`) frees the
  collection block only and never walks element pointers. `_mfb_arena_free` takes
  `(pointer, size)` — there is no allocator header, so every free recomputes its size.
- **The one deep copy.** `copy_value_to_current_arena`
  (`src/codegen/memory/arena/builder_arena_transfer.rs`) calls a per-type function
  (`thread_copy_symbol`, emitted for every member of `recursive_transfer_types` by
  `src/codegen/engine/builder/mod.rs`) whose body is `emit_thread_copy_real`; each pointer edge
  is its own arena allocation, and a recursive sub-edge is a **native call** back into the
  per-type function. Callers: thread send (`cleanup/thread/builder_thread_cleanup.rs`) and
  bug-538's `materialize_owned_element` (`src/codegen/memory/owned.rs`).
- **No last-use analysis.** Only `plan_returned_move` (`RETURN <owned local>`) elides a copy;
  `src/codegen/engine/analysis` has no liveness (`grep -rni "last.use\|liveness"
  src/codegen/engine/analysis` → empty). Precedent prescans live in
  `src/codegen/engine/function/function_lowering.rs` (`collect_borrow_get_locals`,
  `collect_reassigned_locals`, …).
- **The in-place arms assume one owner** and never check it (`admits_with`,
  `src/codegen/collection/assign/inplace_dest.rs`). `G24`
  (`src/codegen/collection/assign/builder_inplace_assign.rs`) declines in-place `removeAt`
  for a recursive element type in three arms.

### 2.1 Measured populations

All commands from the repo root on macOS AArch64, compiler at `c4279099d`. Probe programs
are in §2.2; this letter moves them into `tools/recursive-value-bench/`.

| What | Count / value | Command |
|---|---|---|
| Recursive type families in the builtins | 4: `json::Json` (`JsonArr.items`, `JsonObj.fields`), `__regex_Node` (`parts`, `opts`, `child`), `__regex_Cont` (`nxt`), `__regex_Choices` (`nxt`) | `grep -rn -A3 "add_union(RegistryUnion" src/codegen/builtins` + each union's `ty:` fields |
| Candidates that are NOT recursive | `canvas::DrawItem` (a `Group` names a sub-scene), `http::Stream` (two resource variants) | same, `canvas/mod.rs`, `http/mod.rs` |
| Recursive types in the example apps | `dom::Node` (`ElementNode.children AS List OF Node`) | `grep -n "EXPORT UNION Node" examples/browser/dom/src/lib.mfb` |
| `collections::append` over a recursive element list in builtin bodies | 5 (`json/helper_parse_array_items.rs`, `json/helper_revive.rs`, `regex/helper_parse_alt.rs`, `regex/helper_parse_concat.rs` ×2) | `grep -rnE "^\s*(acc\|items\|opts\|parts) = collections::append\(" <those four files> \| wc -l` |
| `lower_value_owned` callers | 5 | `grep -rn 'lower_value_owned(' --include='*.rs' src/codegen \| grep -v 'fn lower_value_owned' \| wc -l` |
| `ActiveCleanup::OwnedValue(` constructions | 6 | `grep -rn 'ActiveCleanup::OwnedValue(' --include='*.rs' src/codegen \| grep -v 'matches!' \| grep -v '=>' \| wc -l` |
| `is_freeable_flat_value` references (every gate the class skips) | 32 | `grep -rn 'is_freeable_flat_value' --include='*.rs' src/codegen \| grep -v 'fn is_freeable_flat_value' \| wc -l` |
| `emit_copy_payload_to_collection` references | 15 | `grep -rn 'emit_copy_payload_to_collection' --include='*.rs' src/codegen \| grep -v 'fn ' \| wc -l` |
| `emit_wrap_record_in_union` callers | 2 | `grep -rn 'emit_wrap_record_in_union(' --include='*.rs' src/codegen \| grep -v 'fn ' \| wc -l` |
| `G24` in the in-place assign arms | 5 lines, 3 gating functions (`try_inplace_remove_at_assign`, `…record_field_remove_at…`, `…state_remove_at…`) | `grep -n "G24" src/codegen/collection/assign/builder_inplace_assign.rs` |
| Shape C leak tests in `tests/` | 0 | `grep -rniE "shape c\b\|shape_c" tests --include='*.rs'` |
| Shape C union repro, peak RSS at 400k / 800k iterations | 52.7 MB / 104.3 MB | `c_union_rss` (§2.2) under `/usr/bin/time -l` |
| Shape C record repro, peak RSS at 400k / 800k | 105.1 MB / 209.1 MB | `c_record_rss` |
| `json::parse` of one 480 KB document, K = 1 / 2 / 4 times | 204.3 / 402.4 / 798.7 MB; 0.25 / 0.18 / 0.37 s | `json_repeat` |
| `regex::findAll` over a 100 000-char subject, K = 1 / 2 / 4 | 345.4 / 673.1 / 1328.6 MB; 0.25 / 0.19 / 0.39 s | `regex_repeat` |
| `_mfb_thread_copy_*` calls from `main` for a `Node` stored four ways (bind, list literal, `append`, `LET c = a`) | 0 (6 `arena_alloc`, 1 `arena_free`, 2 copy functions emitted) | `node_copies`, `mfb build --ncode`, count relocations `from _mfb_fn_main` |

**After** (filled in by the letters that move a row; plan-134-H records the rest). The table above
has no "after" column, so the later numbers are recorded here against the same instruments.

| Row | After | Letter, command |
|---|---|---|
| Shape C union repro, peak RSS at 400k / 800k | 1.08 MB / 1.08 MB (1 081 344 / 1 081 344 B) | plan-134-G, `tools/recursive-value-bench/run.sh target/release/mfb c_union_rss c_record_rss` |
| Shape C record repro, peak RSS at 400k / 800k | 1.03 MB / 1.03 MB (1 032 192 / 1 032 192 B) | plan-134-G, same run |
| bug-601 recursive row: `MUT ys = xs` over `List OF Tree`, 5 in-place appends | `ys=6 xs=112` at `cc1012bdc` (a read of freed storage, so the wrong number is not stable — `xs=240` when this row was first written; correct: `xs=1`) | `tree_alias` |
| Depth at which today's runtime deep copy crashes | 50 000 exits 0; **70 000, 100 000, 1 000 000 exit 139** | `deep_chain` (copy forced by `collections::get`) |
| Control: building and holding the same chain without a copy | 1 000 000 exits 0 | `deep_build_only` |
| Stack every thread runs on | 8 MiB (worker: `pthread_attr_setstacksize` / `CreateThread`, `runtime/thread/runtime_helpers.rs`); Windows main: 8 MiB reserve, 1 MiB commit (`.ai/arch-abi.md` "Win64 stack growth") | read |
| Nesting limits on decoder input | `json` 256 (`__JSON_DEPTH_LIMIT`), `regex` parse 200 (`__REGEX_PARSE_DEPTH_LIMIT`) | `grep -rn "DEPTH_LIMIT AS Integer" src/codegen/builtins` |
| `__regex_Cont` chain length during a match | **Bounded by the pattern, not the subject**: at most 2 + the Concat/Group/Repeat nesting on the path to the current node (`__REGEX_PARSE_DEPTH_LIMIT` 200 caps it); 3 for `[a-c]+[0-9]+`. Read: `__regex_run` (`regex/helper_run.rs`) pushes `ContSeq`/`ContCap`/`ContRep` only on entering a node and pops each when it completes; a repeat iteration pops its `ContRep` before pushing the next, so iterations do not deepen it. Measured: `arena.0.alloc_calls` 2 068 at 1 hit, 482 081 at 10 000 hits — (482 081 − 2 068) / 9 999 = 48 per hit at both sizes, so nothing per-match grows with the subject | `run.sh … regex_chain` rows `simple:1` / `simple:10000`, built `--debug` |
| `__regex_Choices` chain length during a match (the data-dependent recursive chain) | **Up to 500 000** (`__REGEX_PENDING_LIMIT`): one `__regex_Choice` per greedy non-simple repeat iteration. `(a)+` over 499 999 × `a` exits 0 (peak RSS 991 MB); 500 001 raises `backtracking limit exceeded` (exit 3). The step budget (2 000 000 per search, ~2 steps per iteration) is not what fires. So letter B's walker must handle a 500 000-deep `__regex_Choices` chain, 7× the depth today's native-recursive copy dies at | `run.sh … regex_chain` rows `group:499999` / `group:500001` |

### 2.2 Probe programs (moved into `tools/recursive-value-bench/` by Phase 1)

- `c_union_rss`: `SUB main` looping `LET v AS json::Json = json::JsonNull[NOTHING]` n times
  (n = first argument) — bug-536's own repro.
- `c_record_rss`: the same with `TYPE Node / kids AS List OF Node / tag AS Integer` and
  `LET nd AS Node = Node[kids := [], tag := i]`.
- `json_repeat`: builds `[` + 20 000 × `{"a":[1,2,3],"b":"xyz"},` + `0]` (480 003 bytes), then
  `LET v = json::parse(text)` K times.
- `regex_repeat`: builds 10 000 × `"abcab1234 "`, then `regex::findAll(subject,
  "[a-c]+[0-9]+")` K times (10 000 hits each).
- `node_copies`: `a` stored by `LET`, in a list literal `Node[kids := [a], …]`, by `append`,
  and by `LET c = a`.
- `tree_alias`: `TYPE Leaf / TYPE Branch (kids AS List OF Tree) / UNION Tree`; `LET xs` one
  element, `MUT ys = xs`, 5 × `ys = collections::append(ys, t)`; prints `ys=… xs=…`.
- `deep_chain`: `cur = Node[kids := [cur], tag := i]` n times, `append` to a list, then
  `collections::get` (forces the per-type deep copy). `deep_build_only` omits the `get`.

### Verified properties

- **The deep-copy crash is the copy's native recursion, not the chain.** `deep_build_only`
  holds a 1 000 000-deep chain; `deep_chain` dies at 70 000 only when the `get` copy runs
  (both measured). The copy functions recurse per edge (read: `emit_thread_copy_real` →
  `copy_record_fields_into_existing` → `copy_value_to_current_arena` → the per-type call).
- **Runtime values cannot form cycles — FALSE on main; letter E makes it true.** §14.5 says the
  compiler rejects value cycles, and under value semantics none can exist. The codegen breaks it
  (plan-134-D's in-place-arm audit, 2026-09-13): a constructor argument is lowered with
  `lower_value` (`builder_values.rs`, `NirValue::Constructor` → `emit_build_inlined_record` stores
  a non-inlined field's pointer verbatim), and the in-place list arms lower their item with
  `lower_value` too (`builder_inplace_assign.rs`: `try_inplace_append_assign`,
  `try_inplace_bulk_append_assign`, `try_inplace_prepend_assign`, `try_inplace_insert_assign`,
  `try_inplace_set_assign` for List and Map, `try_inplace_set_add_assign`) and write it with
  `emit_copy_payload_to_collection`'s byte copy. No gate asks whether the item reaches the
  destination (`InPlaceGate::admits_with` checks by-ref, live `FOR EACH` and layout only; G12
  catches only `args[1] == Local(name)`; the bug-496 operand snapshot covers globals/by-ref/STATE
  and only flat types). So `xs = collections::append(xs, Node[kids := xs, tag := 1])` stores a
  payload whose `kids` word is `xs`'s own block: with spare capacity a self-cycle, on a growing
  append a pointer to the freed pre-grow block. Repro (`/tmp/p134cycle`: two such appends, then
  `collections::get` of each element; correct output `len xs=2`, `first.kids=0`,
  `second.kids=1`): the pre-plan compiler and the plan-134-B walker both print `len xs=2` then
  exit 139 at the first `get`. The record-field and STATE in-place arms cannot fire for a
  recursive element (`record_collection_last_inlined`, gate G17, needs a memcpy-copyable field,
  and `flatness_walk` answers false on a type cycle); removal arms store nothing. The fix is
  plan-134-E's copy at construction stores plus owning-store lowering of these arms' item
  operands (added to plan-134-E Phase 1/2), pinned by this repro.
- **Worker and main stacks are both 8 MiB** on every target, so a depth bound measured on
  macOS main applies to workers (read, cited above). Windows main commits 1 MiB and grows by
  guard page, which `finalize_frame`'s probes support.

## 3. Design Overview (all letters)

Two halves that must be exact inverses, landed copy-first:

1. **Copy half (B–E).** Every owning store of a value whose type reaches a cycle holds an
   independent graph. B replaces the recursive per-type copy with a **non-recursive walker**
   (fixes today's crash, and makes copies safe on untrusted-depth input). C adds a
   **last-use analysis** so a store whose source is never read again moves instead of
   copying (§14.2), keeping the decoders' `acc = append(acc, item)` at zero copies. D routes
   the five `lower_value_owned`/`lower_returned_value` stores through the walker; E does the
   same for the construction stores that write sub-pointers verbatim. Copy-only changes add
   allocations and free nothing, so they cannot corrupt memory; bug-601's recursive row flips
   at D.
2. **Drop half (F–H).** F emits a **non-recursive per-type drop** walker, the inverse of B,
   with no callers; a symmetry harness proves copy-then-drop returns `live_bytes` to its
   starting value. G registers it at every place ownership ends (scope, temporaries,
   overwrite, move deactivation, closure env, `get` results). H frees collection elements
   (removed, overwritten, and on collection drop), revisits `G24`, measures the decoders, syncs
   the docs and runs the final gate.

**Where correctness risk concentrates:** G and H — a free of a block another owner still
reaches is arena corruption, not a leak. They come last, behind B–E's guarantee that every
owner's graph is distinct and F's symmetry harness.

**Where design uncertainty concentrates:** the speed cost of copying (answered by C+D's
benchmark gate, with the move analysis landing before the copies it avoids), and the depth of
real inputs (answered here and by B).

**Byte-identity is not this plan's correctness gate** except for letter C (an analysis with no
callers) and for programs with no recursive type. Expected diffs are named per letter:
`tests/byte-identity/json` and `tests/byte-identity/regex` (the only byte-identity dirs that
use a recursive type: `find tests/byte-identity -maxdepth 3 -type d | grep -iE
"json|regex|canvas|dom"` → those two), plus any fixture that declares a recursive user type.
A diff anywhere else is a bug to localize (objdump one fixture), never a verdict on the design.

### Rejected alternatives

- **Keep the recursive copy/drop and raise stack sizes.** Depth is data-dependent (a user tree,
  a regex continuation chain) and the stack is already 8 MiB everywhere; a bigger stack moves
  the crash, it does not remove it. §14.5 already licenses iterative drop.
- **Tail-call only the last pointer edge.** A node with a `List OF Node` has many edges; only
  a list-of-one chain would benefit.
- **Reference counting or copy-on-write for recursive values.** §14 says "no reference
  counting" and ownership is lexical; it would also add a header, which the Non-goals forbid.
- **Decline every in-place arm for the class (bug-601 option A).** Correct but makes
  `json::parse` array building quadratic, and fixes neither the leak nor the non-list stores.
- **Copy at stores without a move analysis.** Leaves the decoders paying one deep copy per
  appended element; C exists so D does not ship that regression.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the
> work; `- [~]` with one line on what remains; `- [x] ~~text~~ — moot: <evidence>` rather than
> deleting. Fill `Commit:` the moment a phase lands. An unticked box means NOT DONE.

### Phase 1 — the benchmark tool and the regex chain length

Lands measurement only; no compiler change.

- [x] Create `tools/recursive-value-bench/` with the seven programs of §2.2 as
      `programs/<name>/{project.json,src/main.mfb}`, a `run.sh <mfb> [program…]` that builds
      each and prints `name size maxrss_bytes real_s exit` per run (macOS `/usr/bin/time -l`,
      Linux `/usr/bin/time -v` or `ru_maxrss` via the program's `--debug` peak-RSS section),
      and a README stating what each program measures and which letter gates on it.
      (`tools/`, not `scripts/`: AGENTS.md — benchmarks and probes go in `tools/<name>/` with a
      README.) — 9 programs (`deep_build_only` is its own project; `regex_chain` added by the
      third task); rows also carry a `stdout` column, and `node_copies` adds an `-ncode`
      call-count row.
- [x] Run it on main and record the table in §2.1 if any row moved by more than 10 %. — no RSS
      row moved more than 0.1 %: `c_union_rss` 52 658 176 / 104 267 776, `c_record_rss`
      105 054 208 / 209 076 224, `json_repeat` 204 259 328 / 402 423 808 / 798 736 384,
      `regex_repeat` 345 391 104 / 673 103 872 / 1 328 578 560 bytes; `node_copies` 0 copy
      calls, 6 allocs, 1 free, 2 copy functions; `deep_chain` exit 0 / 139 / 139 / 139;
      `deep_build_only` exit 0. Only `tree_alias`'s garbage value moved (240 → 112; §2.1 row
      corrected).
- [x] Measure the `__regex_Cont` chain length: add a temporary counter program (in the tool's
      `programs/`, not in the compiler) that walks the continuation a `regex::findAll` builds,
      or read it from the `--debug` arena report (`alloc_calls` per match on a 1-hit subject
      vs a 10 000-hit subject). Record the number and command in §2.1. — `programs/regex_chain`;
      the `__regex_Cont` chain is pattern-bounded, and the data-dependent chain is
      `__regex_Choices`, up to 500 000 deep (§2.1, two rows).

Acceptance: `bash tools/recursive-value-bench/run.sh target/release/mfb` prints all nine
programs, and its union / record / json / regex rows match §2.1 within 10 %; §2.1 has no
UNMEASURED row.
  Check: that command → 9 programs, exit 0 for all but `deep_chain` at n ≥ 70 000 (exit 139)
  and `regex_chain group:500001` (exit 3, the pending limit) (est. 3 min).
  Result: met — 9 programs; RSS rows within 0.1 %; exits exactly as stated.
Commit: c9bcec1ae

## Validation Plan (this letter)

- Tests: none added — the tool is an instrument; letters B–H add the tests.
- Runtime proof: the tool's output table.
- Doc sync: `tools/recursive-value-bench/README.md`.
- Final gate: none here; the plan-wide gate runs once, in letter H.

## Open Decisions (plan-wide)

- **Speed budget for letter D** — recommended: `json_repeat` K=1 and `regex_repeat` K=1 wall
  time no more than 25 % above the §2.1 baseline (median of 5 runs) once C's moves are in;
  alternative: no budget, accept any cost for correctness (AGENTS/memory: correctness over
  performance). The budget exists to force C's analysis to cover the decoders, not to block
  correctness — a miss is a C/D bug to root-cause, with the owner consulted before shipping a
  regression.
- **Walker shape (letter B/F)** — recommended: one module-level walker per direction
  (`_mfb_rt_graph_copy`, `_mfb_rt_graph_drop`) that dispatches on a per-module type index over
  an arena-allocated work stack; alternative: keep one function per type and add an explicit
  stack to each. The single walker keeps one stack and one dispatch table to prove.
- **`G24` after letter H** — recommended: lift it for recursive element types once `get` owns
  its copy and removal frees the removed graph; alternative: keep declining (safe, slower).

## Corrections

- **2026-09-13, Prerequisites re-run** (all three commands): `planning/completed/plan-132-flatten-the-helper-built-pointer-string-records.md`,
  `planning/completed/plan-130-C-arena-counters-and-registry.md`, `grep -n "bug-538"
  src/codegen/memory/owned.rs` → line 31 inside `materialize_owned_element` — all MET.
- **Seven programs → nine.** §2.2 names `deep_build_only` as a variant of `deep_chain`; it
  needs its own project, and Phase 1's third task added `regex_chain`. The acceptance text
  now says nine.
- **`tree_alias` is `xs=112`, not `xs=240`** (measured at `cc1012bdc`). The value is a length
  read from freed storage, so it is not stable across builds; the defect (≠ 1) is what the row
  records.
- **The regex chain that grows with the input is `__regex_Choices`, not `__regex_Cont`.** The
  plan assumed the continuation chain was the data-dependent depth. Reading `__regex_run` and
  measuring (`regex_chain`) shows `__regex_Cont` is bounded by pattern nesting, while
  `__regex_Choices` reaches 500 000 (`__REGEX_PENDING_LIMIT`). The sizing conclusion for letter B
  is stronger, not weaker: its walker must copy/drop a 500 000-deep chain, far past the 70 000
  where today's recursive copy dies. `__regex_Choices` is a member of
  `recursive_transfer_types`, so it is in the walker's kind set already.

## Summary

The risk in plan-134 is the free: G and H. This letter only makes the numbers every later
letter is judged by reproducible, and closes the one unknown (regex chain depth) that sizes
letter B. It touches no compiler code.
