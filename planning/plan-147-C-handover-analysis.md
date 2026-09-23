# plan-147-C: The hand-over analysis (no callers)

Last updated: 2026-09-21
Effort: large (3h–1d)
Depends on: plan-147-B

Before any call changes, this letter adds the two analyses letter D will act on, and
proves them against hand-derived tables. It emits no code: output is byte-identical
on every target. This mirrors plan-134-C, which landed `collect_last_use_moves` with
no callers before letters D/E used it.

- **Caller side, `collect_handover_args`**: the `(op, argument)` pairs where a
  local's value may be handed to a callee instead of lent.
- **Callee side, `consumable_params`**: for each user function, the parameters an
  owned variant would use up (update in place, return, or bind) rather than just
  read. That decides whether a variant is worth emitting.

References: plan-147-A §2.3 (the conditions below are the rows' "What keeps it"
column, made exact); `planning/completed/plan-134-C-last-use-move-analysis.md`;
`src/codegen/engine/analysis/last_use.rs`.

## Prerequisites

See plan-147-A. plan-147-B must be complete: `ls planning/plan-147-B-* 2>/dev/null` →
no matches.

## 1. Goal

- `collect_handover_args(function, model, callees) -> HandOverArgs` and
  `consumable_params(function, model) -> ParamSet` exist in
  `src/codegen/engine/analysis/handover.rs`.
- A unit table covers every shape in §3.3 and passes. It includes every refusal
  row that pins a plan-147-A §2.3 condition.
- Emitted code is byte-identical on every target. Nothing reads the new sets yet.
- The corpus census row plan-147-A §2.2 left UNMEASURED is filled.

### Non-goals

- Any codegen change, including consumers (letter D).
- Transitive hand-over through fresh temporaries and through a parameter passed on
  to another owned variant (letter E). Here a parameter is consumable only by a
  **direct** consuming use (§3.2).
- Every plan-147-A non-goal.

## 2. Current State

- `collect_last_use_moves` (`last_use.rs:912`) computes, per op, which places are
  live after it. It includes `trap_live`, everything any `TRAP` handler reads
  (`:214`, `:230`). It yields sites only for `Bind`/`Assign`/`StoreGlobal`/`StateAssign`/
  `Eval`/`Return`/`Fail`/`ExitProgram`, where the place is read exactly once in the op
  and is not live after it (`:864-882`).
- `excluded_roots` (`:647-746`) excludes parameters, `LocalRef` (address-taken),
  closure captures, `FOR`/`FOR EACH` variables, the `TRAP` error name, `STATE`
  resources, resource-bearing types, locals bound from `Capture`, non-view
  `UnionExtract` binds, borrow-`get` locals, and any name never bound. `Place`
  covers locals only, so globals can never be sites.
- Inline `TRAP`/`RECOVER` is desugared before NIR into `bind $trap_res`,
  `ResultIsOk` and an `IF` (`optimizer/opt1/recovery.rs:34-37`). A handler that reads
  `x`, including `RECOVER x`, is an ordinary read after the op, so `x` is live after it.
  The function-level `TRAP` is `NirOp::Trap{name, body}`, at most one, placed last.
- Direct user calls: `NirValue::Call` whose target resolves to a user `NirFunction`
  in the merged module (packages included; `src/target/shared/nir/lower.rs`
  `merge_packages`). Builtins, `LINK` functions, and calls through function values are
  other targets. `store_reach.rs` already distinguishes them (it treats indirect and
  `LINK` calls as reaching everything, `:143-166`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Unit-table rows plan-134-C needed for the same kind of analysis | **6** `#[test]` functions (measured 2026-09-23) | `rg -c '#\[test\]' src/codegen/engine/analysis/last_use.rs` → `6`. Each is a multi-shape table rather than one shape per test, so §3.3's 16 rows fit comfortably in a similar handful. |
| Corpus call sites approved by `collect_handover_args` | UNMEASURED — this letter's Phase 3 measures it | Phase 3 |

## 3. Design

### 3.1 Caller: an argument that may be handed over

`collect_handover_args` returns `(op_key, call_path, arg_index)` for every direct
user call in an op whose argument `arg_index` is `NirValue::Local(x)` and:

- **H1 owned local.** `x` is not in `excluded_roots`: not a parameter, not by-ref
  or captured, not address-taken, not a `FOR`/`FOR EACH` variable, not a live
  `FOR EACH` iterable at the op. Letter D widens this for a parameter that the
  current function itself owns.
- **H2 type.** `x`'s type is a `List`/`Map`/`Set` or `String`, and
  `type_contains_resource` is false (row S8).
- **H3 last use.**
  - `x` is read exactly once in the whole op, and that read is this argument (row
    S6: `f(x, x)` is two reads).
  - `x` is not live after the op. For `Assign{target: x}` the op's own store kills
    `x`, so "live after" is computed without that store's target. The old value is
    dead, and the new one is what the rest of the function reads.
  - `x ∉ trap_live` (row S3: no function-level handler reads it).
  - Inline `TRAP` handlers are covered by the desugar (§2): a handler read makes `x`
    live after the op.
- **H4 not a global.** Structural: `Place` has no globals (row S7).
- **H5 direct user call.** The target is a user `NirFunction` whose body is in the
  module. It is not a builtin, not `LINK`, not a call through a function value (row
  S13), and not an `ISOLATED` thread entry (row S10).
- **H6 the callee would use it.** `arg_index ∈ consumable_params(callee)` (§3.2). If
  not, handing over only moves a free from caller to callee, so it is refused.

### 3.2 Callee: a parameter an owned variant consumes

`consumable_params(F)` returns the parameters `p` of `F` such that:

- **P1** `p`'s type is as in H2;
- **P2** `p` would pass H1 if it were a local: not captured, not address-taken, not
  a `FOR EACH` variable;
- **P3** at least one path ends `p`'s life with a **consuming use** at its last use:
  - `RETURN OP(p, …)` where `OP` is a self-update-shaped builtin
    (`is_self_update_call`), which is B's S11;
  - `RETURN p`, which is `plan_returned_move`'s move.

  (`MUT y = p` / `LET y = p` and passing `p` on to another owned variant are E.)

P3 decides only whether a variant is *worth* emitting. Correctness does not depend
on it: an owned variant frees any owned parameter it does not consume (letter D).

### 3.3 The unit table

One row per shape, as a NIR function built with the same test helpers
`last_use.rs`'s tests use. Expected `HandOverArgs` or `ParamSet` is written by hand:

| Shape | Expected |
|---|---|
| `acc = helper(acc, i)` in a loop, helper consumes `xs` | handed over |
| `LET y = helper(x, 1)` then print `x` | refused: H3 live after (S2) |
| inline `TRAP` handler reads `x` | refused: H3 via desugar (S3) |
| `x = helper(x) TRAP(e) RECOVER x END TRAP` | refused: H3 (S3) |
| `x = helper(x) TRAP(e) RECOVER other END TRAP` | handed over: the old value is never read |
| function-level `TRAP` reads `len(x)` | refused: `trap_live` (S3) |
| `x = pair(x, x)` | refused: H3 two reads (S6) |
| global argument | refused: structural (S7) |
| `List OF RES` argument | refused: H2 (S8) |
| argument captured by a lambda | refused: H1 (S9) |
| call through `LET f = helper` | refused: H5 (S13) |
| helper that only reads `xs` (`len(xs)`) | refused: H6 |
| `x` is a `FOR EACH` iterable live at the op | refused: H1 |
| `consumable_params`: `RETURN collections::set(xs, i, v)` | `{xs}` |
| `consumable_params`: `IF n = 0 THEN RETURN xs` / `RETURN len(xs)` | `{xs}`: one path consumes |
| `consumable_params`: `RETURN len(xs)` only | `{}` |

**Correctness risk:** the `Assign` store-kill in H3. Getting it wrong either way is
unsafe: treat the target as live and nothing is ever handed over; drop the kill for a
target the op *also* reads elsewhere and a live value is handed over. The rule is
exact: kill only the op's own store target, and only after H3's "read exactly
once" has already passed.

**Rejected:** computing hand-over inside `collect_last_use_moves` as more
`MoveSites`. Its sites are owning stores and its consumers are stores. A call
argument is a different consumer with its own callee-dependent condition (H6). A
separate module keeps plan-134's behaviour byte-identical by construction.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with one line on what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — The analyses

- [x] Fill the first measured-populations row: `rg -c '#[test]' src/codegen/engine/analysis/last_use.rs` → **6**.
- [x] `src/codegen/engine/analysis/handover.rs`: `HandOverArgs`, `ParamSet`,
      `collect_handover_args`, `consumable_params` (§3.1–§3.2). It reuses `last_use.rs`'s
      liveness through a new `pub(crate) fn live_out_of(function, model) -> LiveOut`
      that returns the per-op live-out map, `trap_live` and `excluded_roots` in one
      value, plus `pub(crate)` on `Places`, `Place::root`, `reads_of`, `kill`,
      `place_live` and `read_count`.
- [x] Register the module in `src/codegen/engine/analysis/mod.rs`. Nothing calls it
      outside tests — the module carries a documented `#![allow(dead_code)]` saying so,
      which **letter D removes** when it wires the consumers up.

Acceptance: it builds, and nothing but tests uses it.
  Check: `cargo build --release && rg -n 'collect_handover_args|consumable_params' src --glob '!**/handover.rs'`
  → **builds clean (no warnings), and `rg` returns nothing** (2026-09-23). The plan
  said "only `analysis/mod.rs`"; `mod.rs` declares the module but never names either
  function, so **no** match is the correct result. See Corrections.
Commit: (this commit)

### Phase 2 — The unit table

- [x] Every §3.3 row as a `#[test]` in `handover.rs`: 4 tests carrying all 16 rows —
      `collect_handover_args_follows_the_hand_derived_table` (12 caller rows, including
      a positive twin for the function-level `TRAP`), `a_global_argument_is_never_handed_over`
      (S7), `a_resource_bearing_argument_is_never_handed_over` (S8), and
      `consumable_params_follows_the_hand_derived_table` (the callee rows).
- [x] Prove each refusal row can fail. Each condition was removed, the suite run, and
      the file restored byte-for-byte (`diff -q` against a saved copy). The four
      failure lines, verbatim:

      1. **S2/S3 — delete H3's "dead after the op"** (`|| place_live(&after, &place)`):
         ```
         read again after the call (S2): 1 argument(s) approved, want 0
         an inline TRAP handler reads it (S3): 1 argument(s) approved, want 0
         a function-level TRAP READS it (S3, trap_live): 1 argument(s) approved, want 0
         ```
      2. **S6 — delete H3's "read exactly once"** (`read_count(&all_reads, &place) != 1 ||`):
         ```
         the same local reaches two parameters (S6): 1 argument(s) approved, want 0
         ```
      3. **S3 — delete the `trap_live` extension** (`after.extend(live.trap_live…)`):
         ```
         a function-level TRAP READS it (S3, trap_live): 1 argument(s) approved, want 0
         ```
      4. **S13/H5 — let the indirect target resolve.** H5's guard is
         `callees.get(target)`, and an indirect call's NIR target is the LOCAL's name
         (`{ "kind": "call", "target": "f", … }`, dumped with `mfb build -ir`), so the
         lookup misses. The proof is therefore test-side, which is stronger than
         deleting a compiler condition: adding `"f" -> consume` to the probe's callee
         map makes H5 accept, and the row goes red:
         ```
         called through a function value (S13, H5): 1 argument(s) approved, want 0
         ```

Acceptance: all rows pass, and the four RED proofs are recorded.
  Check: `cargo test --bin mfb handover` → **`ok. 4 passed; 0 failed`** (2026-09-23).
Commit: f1ce00da6

### Phase 3 — Corpus census and neutrality

- [x] One-off probe: a temporary `MFB_HANDOVER_CENSUS=1` hook at the end of
      `target/shared/lower.rs:lower_project`, which ran both analyses over the real
      merged `NirModule` of each project and then **was removed again** (the tree is
      back to its committed state; `grep -c MFB_HANDOVER_CENSUS src/target/shared/lower.rs`
      → `0`). It had to go through the real pipeline: `nir_for_src` takes a single
      source string and cannot resolve a project's `IMPORT`s. Results, recorded in
      plan-147-A §2.2:

      | Tree | Projects | NIR functions | Approved arguments | Consumable parameters |
      |---|---|---|---|---|
      | `examples/*` | 12 | 1,932 | **35** | **40** |
      | `benchmark/mfb` | 1 | 1,346 | **504** | **509** |
      | `packages/*` | — | — | counted inside consumers | counted inside consumers |

      `examples/audio` and `examples/yaml-json` do not build and are excluded.
      **`packages/*` gets no row of its own**: a package build emits a `.mfp` and never
      reaches `lower_project`, so package bodies are censused where they are actually
      compiled — folded into each importing consumer by `merge_packages`. That is why
      `examples/network-server` alone reports 420 functions. The benchmark's 504
      approved arguments line up with plan-147-A §2.2's 485 `RETURN
      collections::<op>(param, …)` sites in that tree, which is the shape letters D and E
      exist to serve.
- [x] Byte-identity: nothing reads the sets, and the gate confirms it —
      `artifact-gate.sh target/release/mfb collections` → **0 diffs**, with the census
      hook removed and the compiler rebuilt.

Acceptance: census recorded, and codegen unchanged.
  Check: `bash scripts/artifact-gate.sh target/release/mfb collections` →
  **`1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`** (2026-09-23). The analysis
  has no caller, so one package's gate is enough to catch an accidental one.
Commit: (this commit)

## Validation Plan

- Tests: the §3.3 table, with RED proofs for four refusal rows.
- Coverage check: Phase 1's `rg` shows the module has no non-test caller.
- Runtime proof: none (no behaviour change).
- Doc sync: none until F.
- Final gate: plan-147-F.

## Open Decisions

- Should H6 require P3 on the callee, or hand over whenever H1–H5 hold? **Recommended:
  require P3.** Otherwise every `len(x)`-style helper gets a variant whose only effect
  is to move a free.

## Corrections

- **A call is identified by its address, not by a `call_path`.** §3.1 describes a
  hand-over triple as `(op_key, call_path, arg_index)`. It is `(op_key, call_key,
  arg_index)`, where `call_key(value)` is the address of the `NirValue::Call` node —
  the same device `op_key` already uses, and for the reason `last_use.rs`'s module doc
  gives for it: an address "cannot drift the way a counted index could". A path of
  child indices into a value tree would have to be rebuilt in step with every
  desugar that rewrites the tree.

- **Phase 1's check expects no match, not `analysis/mod.rs`.** `mod.rs` declares
  `pub(crate) mod handover;` and nothing else, so it never names `collect_handover_args`
  or `consumable_params`. The acceptance is "`rg` returns nothing", which is the
  stronger reading of the same intent: no production caller anywhere.

- **The reuse is one function, not three `pub(super)` exports.** Phase 1 asked for
  `ops_in`, `trap_live` and `excluded_roots` to be exposed. `Liveness` is built from
  `ViewShape`, `Canon` and the exhaustive-`MATCH` set, all computed inside
  `collect_last_use_moves`, so exposing the three pieces would have meant exporting
  that whole construction. Instead `last_use.rs` gained one
  `pub(crate) fn live_out_of(function, model) -> LiveOut` that performs the
  construction and returns exactly what a second analysis needs: the per-op live-out
  map, `trap_live`, and the exclusion set.

  It differs from `collect_last_use_moves` in one deliberate way, documented at the
  function: **every `MATCH` view is treated as borrowed**, so a read through a view is
  charged to the view's source and keeps that source live. `collect_last_use_moves`
  narrows to a fixed point to discover which views may own; hand-over does not need
  that extra reach, and borrowing only ever ADDS liveness — so the simplification can
  only refuse a hand-over, never license a wrong one. That is the fail-closed
  direction.

- **The analysis has to recognise the TRAPPED call form, `NirValue::CallResult`.**
  §3.1 says "every direct user call in an op", and the first implementation matched
  only `NirValue::Call`. The §3.3 row "`RECOVER` names something else" then came back
  refused when it should be approved, and dumping the NIR showed why: the inline-`TRAP`
  desugar rewrites `x = f(x) TRAP(e) …` into
  `Bind $trap_res0 = { "kind": "callResult", "target": "consume", … }`. Matching only
  `Call` means **never looking at a call under a handler at all** — precisely the
  shape §2.3's S3 rows exist to constrain, so the gap would have hidden itself: every
  such call would have been silently refused, which looks like correct conservatism.
  `for_each_call` now matches `Call | CallResult`, and `call_key` names either.

- **`NirVisitor` cannot collect borrows, so two walks are hand-written.** The trait's
  methods take `&NirValue`/`&NirOp` with no lifetime parameter of their own, so a
  visitor cannot return `Vec<&NirValue>`. `for_each_call` therefore does its work
  *during* the traversal (a callback, still going through the shared `walk_value`, so
  it cannot drift), and `ops_of` is an explicit recursion whose `match` has no
  wildcard — a new `NirOp` variant is a build error there, exactly as in `last_use.rs`.

## Summary

The analysis is where plan-147-A's semantics become code. The one subtle rule
is the `Assign` store-kill in H3. Everything else is refusal, and refusing is
always correct.
