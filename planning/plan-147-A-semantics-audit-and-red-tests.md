# plan-147-A: Owned-argument semantics audit and the RED tests

Last updated: 2026-09-21
Overall Effort: huge (>3d)
Effort: medium (1h–2h)
Depends on: nothing (the Prerequisites below gate the whole of plan-147)

## plan-147 as a whole

**Goal of the whole plan:** a collection or `String` that the caller no longer needs
is handed to the callee instead of lent, and the callee updates it in place. For
example, `acc = helper(acc, i)` where `helper` is `RETURN collections::set(xs, i, v)`
should run at the cost of the inline `acc = collections::set(acc, i, v)`, with no
per-call copy of `acc`. The same rule gives the recursive case,
`RETURN fill(collections::append(xs, n), n - 1)`, and the plain-local case,
`RETURN collections::append(out, x)` with `out` a local at its last use. Neither is
in place today.

This is the "owned argument" option from the 2026-09-21 discussion. It sits behind
`.ai/collections.md` §"An accumulator must not be threaded through a helper". It is a
**codegen optimization only**. No program can observe it except by timing or
allocation counts. Letter A establishes that from the spec before any code is
written, and pins it with tests that must stay green through every later letter.

| Letter | Delivers | Effort |
|---|---|---|
| **A** | Semantics audit (this file §2.3), RED allocation tests, GREEN semantics fixtures | medium |
| **B** | `RETURN OP(x, …)` is in place when `x` is an owned local at its last use (new site S11) | large |
| **C** | Hand-over analysis: which call arguments may be handed over, which parameters an owned variant consumes. No callers. | large |
| **D** | Owned function variants and the caller hand-over (the calling-convention change) | large |
| **E** | Owned-parameter site S12 in the guard matrix; transitive hand-over (fresh temps, recursion, a parameter passed on) and `MUT y = p` | large |
| **F** | Record accumulators (field form of S11, via plan-145's seam); spec/doc amendments; the full gate | large |

Letter order is implementation order. Each letter depends on the one before it.

References:

- `mfb spec language memory-semantics` §14 intro, §14.1, §14.2, §14.3, §14.6, §14.7
  (`src/docs/spec/language/14_memory-semantics.md`), the normative rules this plan
  must not change.
- `mfb spec language functions` §6 "Parameter passing"
  (`src/docs/spec/language/06_functions.md:54`).
- `mfb spec language error-model` §8.2–§8.5a (`src/docs/spec/language/08_error-model.md`).
- `.ai/collections.md` §"In-place mutation: one table, four sites" and §"An accumulator
  must not be threaded through a helper".
- `planning/completed/plan-142-*` (the self-update seam and guard this plan extends),
  `planning/completed/plan-134-C-last-use-move-analysis.md` (the analysis this plan
  extends).

## Prerequisites

These are a precondition on the whole of plan-147, not a dependency to negotiate.

| Must be true | Command | Status |
|---|---|---|
| plan-145 (field self-updates) complete. It extends `SELF_UPDATE_TABLE`, `ENABLED_SITES`, the matrix test and the runtime harness that letters B and E add a site to. It also provides the field arms E needs for record accumulators. | `ls planning/plan-145-* 2>/dev/null` → no matches | **MET** (2026-09-22: `ls planning/plan-145-*` -> no matches; all nine letters archived to `planning/completed/plan-145-A..I`, landed on main through `096acb8bd plan-145-I: lock the field guard; docs`) |
| plan-146 (`String` self-updates) complete. It adds the `String` arms that a new site must fire for, and it edits the same harness. | `ls planning/plan-146-* 2>/dev/null` → no matches | **MET** (2026-09-22: `ls planning/plan-146-*` -> no matches; all eight letters A-H archived to `planning/completed/`, landed on main through `13f2e9fcc plan-146: archive A-H to planning/completed`) |
| The seam files are unchanged since this plan was written. If either command lists a commit, re-read §2 before starting and correct it in Corrections. | `git log --oneline 2f55eb184.. -- src/codegen/collection/assign/self_update.rs src/codegen/engine/analysis/last_use.rs` | **CHANGED** (2026-09-22: 19 commits -- nine from plan-145 landing the field seam, ten from plan-146 landing the `String` seam, newest `d648d0231`). §2.1 and §2.2 re-read and corrected below; see Corrections. |

Everything below is written against the world where these hold. This plan does
not absorb, work around, or hand-roll anything from plan-145 or plan-146.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- `planning/plan-147-A-semantics-audit-and-red-tests.md` §2.3 holds the semantics
  audit. For every spec rule the change touches, it records why the observable
  behaviour is unchanged or which condition keeps it unchanged. It also names the
  one normative sentence that needs amending (§14.6), with the amendment text.
- `tests/runtime/rt_owned_argument.rs` exists. Its five allocation-scaling cases are
  **RED** at HEAD, and each failure message names the case.
- `tests/rt-behavior/memory/owned-argument-semantics-rt/` exists. It covers every
  condition in §2.3 whose violation would be observable, and it is **GREEN** at HEAD.
  Every later letter keeps it green.

### Non-goals (explicit constraints — all of plan-147)

- **No source-language change.** No new keyword, annotation, or diagnostic. §6 says
  "MFBASIC source does not add ownership-annotation parameter keywords such as `MOVE`".
- **No observable change.** The only observable differences allowed are time and
  allocation counts (§14.1: "This is an optimization only; it must not change
  diagnostics or observable behavior except performance"). That includes:
  - every value a program prints;
  - `Error.code`, `Error.message` and `Error.source` (file, line, char);
  - which `TRAP` handler runs;
  - drop order of resources.
- **No ABI change for anything outside the compiled program:**
  - a function value (`LET f = helper`) still names the one existing symbol;
  - `LINK`/C-visible symbols, thread entry points, and `.mfp` package contents are
    unchanged.
  Owned variants are extra internal symbols.
- **No change to resources, threads, or globals as arguments.** Arguments of those
  kinds are never handed over. A global argument keeps its §14.3 snapshot rule.
- **No change to the diagnostics** `TYPE_USE_AFTER_MOVE` / `ir::verify` move tracking
  (source-level, untouched, as in plan-134-C's non-goals).

## 2. Current State

### 2.1 How a collection argument is passed today (read 2026-09-21)

- **Caller.** A `Local` argument lowers to a load of the caller's block pointer. The
  callee gets the caller's own block, not a copy (`builder_values.rs:lower_value_inner`,
  `NirValue::Local` arm; call emission is `builder_emit_helpers.rs:emit_call`). A copy
  is made only by the operand snapshot:
  - a global the call can store to (`operand_snapshot.rs:want_arguments_the_call_can_free`,
    `store_reach.rs:call_reaches_store`);
  - a local a sibling closure captures by reference (plan-142-G).

  Only resources and threads ever move into a call
  (`builder_resource_cleanup.rs:deactivate_moved_resource_arguments`,
  `emit_call`'s `deactivate_consumed_cleanups`).
- **Callee.** A parameter gets no `ActiveCleanup::OwnedValue` (`function_lowering.rs:lower_function`,
  the parameter loop). It is never freed on any exit. `plan_returned_move`
  (`builder_exits.rs`) says so outright: "parameters and aliases have none".
  Parameters are immutable in IR verification (`src/ir/verify/mod.rs:407`,
  `muts.insert(param.name, false)`), so no `NirOp::Assign` can target one.
- **Return.** `RETURN collections::set(xs, i, v)` with `xs` a parameter lowers
  through `lower_returned_value` → plain `lower_value`. The copying `set`
  (`func_set.rs:lower_set`) does `copy_collection_tight(xs)` and then sets into the
  copy. `RETURN xs` deep-copies the parameter unless `function_returns_param_borrow`
  (`function_lowering.rs:120`) makes the whole function a borrow passthrough.
- **Caller store.** `acc = helper(acc, i)` goes through `NirOp::Assign`
  (`builder_control.rs`). The self-update arms decline because the callee is not a
  builtin (G3, `inplace_dest.rs:resolve_self_update`). The call's fresh result is
  claimed, and the old `acc` is dropped only **after** the call returns
  successfully. So on failure the old value is intact, as §14.2 requires.
- **Self-update sites.** Dispatch happens only in `NirOp::Assign` and
  `NirOp::StoreGlobal`. `RETURN OP(x, …)` is never a site: `NirOp::Return` →
  `emit_return_exit` → `lower_returned_value`, and `ops_hold_self_update`
  (`self_update.rs`) matches only the two assignment ops.
  `ENABLED_SITES = [Local, ForEach, Lambda, Global]` (`self_update.rs:2496` after
  plan-145/146; the value is unchanged, the line moved). plan-145 added a
  **test-only** `FIELD_SITES` next to it (`self_update.rs:2500`) that the matrix
  iterates with `ENABLED_SITES.iter().chain(FIELD_SITES)` (`:3102`); it is not a
  production site list. `ops_hold_self_update` (`self_update.rs:656`) now also
  matches plan-145's field form (`with_holds_field_self_update`, and `gR = WITH gR
  { f := g(gR.f, …) }` at `:676`), but still **only** under `NirOp::Assign` and
  `NirOp::StoreGlobal` — so `RETURN OP(x, …)` is still never a site, as this plan
  assumes.
- **Last-use analysis.**
  - `collect_last_use_moves` (`src/codegen/engine/analysis/last_use.rs:912`) yields
    sites only for simple statements, and only for recursive-graph types at the
    consumers.
  - It excludes parameters ("any name the function never binds"), `by_ref`/captured,
    address-taken, `FOR`/`FOR EACH` variables, `STATE`, resource-bearing types and
    globals (`excluded_roots`, `:647`).
  - Everything any `TRAP` handler reads is in `trap_live` and counts as live after
    every op (`:214`, `:230`).
- **Variants.** Monomorphization clones per type argument, not per call site
  (`src/monomorph/lower.rs:instantiate_function`). There is no mechanism that emits
  two variants of one user function. The per-function extras are the builtin wrapper,
  ABI helper and thread-copy function (`function_lowering.rs:1629`).
- **Packages.** Callee bodies from other packages are visible. `merge_packages`
  (`src/target/shared/nir/lower.rs`) merges every `.mfp`'s IR into one module, which
  is how `call_reaches_store` already reads package bodies.

### 2.2 Measured populations

| What | Count | Command |
|---|---|---|
| `FUNC`s whose `RETURN collections::<mutating op>(p, …)` has a **parameter** first argument (the D/E shape) | examples 4, packages 5, tests **12**, benchmark 485, builtins **2** (re-measured 2026-09-22) | `R='RETURN\s+collections::(append\|set\|insert\|prepend\|removeAt\|removeKey\|add\|remove)\(\s*\w+\s*[,)]'; rg -P -g '*.mfb' "$R" <tree>`, then an awk pass comparing the first argument with the enclosing `FUNC`'s parameter names (census agent, 2026-09-21) |
| …with a **local** first argument (the B shape) | examples 14, packages 5, tests **0**, benchmark 0, builtins **4** (re-measured 2026-09-22) | same pass |
| `NAME = f(NAME, …)` call sites with a non-builtin callee | examples 55, packages 205, tests 92, benchmark 5, builtins 301 | `rg -P -g '*.mfb' '^\s*(\w+)\s*=\s*[\w:]+\(\s*\1\s*[,)]' <tree> \| rg -v -P '=\s*(collections\|strings\|math\|bits)::' \| wc -l` (`-g '*.rs'` for `src/codegen/builtins`) |
| …of those, threading a collection or `String` (not an `Integer` or a record) | UNMEASURED: the type needs the compiler | Letter C's corpus census task measures it. It does not set the split: the work scales with shapes, not with call sites. |
| Self-recursive functions threading a collection accumulator | 1 pure (`src/codegen/builtins/canvas/helper_render.rs:162` `__canvas_appendDraw`), plus 2 record accumulators (`packages/json_schema/src/index.mfb:195` `walkSchema`, `examples/browser/dom/src/lib.mfb:300` `gatherSpecs`) | census agent's awk scan for self-calls in `x = f(…x…)` / `RETURN f(…append…)` (heuristic; misses mutual recursion) |

**Baseline cost** (release `mfb` built after `77a9255b1`; `/tmp/owned` program
reproduced as the RED test below; one run on a busy machine, 2026-09-21):

| Shape, N = 20,000 | Inline | Through a helper |
|---|---|---|
| `append` to `List OF Integer` | 1 ms | 19,224 ms |
| `set` into `Map OF Integer TO Integer` | 51 ms | 372,079 ms |
| `s & "x"` | 0 ms | 841 ms |
| recursive `fill` (N = 5,000) | — | 1,252 ms |

### 2.3 Semantics audit

Each row is a rule plan-147 could break, how the design keeps it, and the fixture
that fails if it doesn't. "Handed over" means the caller's local `x` is passed to an
owned variant that frees or consumes it.

| # | Rule (source) | Could it break? | What keeps it | Pinned by |
|---|---|---|---|---|
| S1 | Arguments are owned values; a copy may become a move when the caller no longer needs the value (§6 "Parameter passing", §14.1 last para, §14.3 first sentence) | No: this is the plan's licence | Hand-over only at `x`'s last use (C: H3) | — |
| S2 | "A call cannot observe or mutate a caller-owned value after the argument has been passed" (§14.3) | No | After hand-over there is no caller-owned value: `x` is dead on every path (C: H3), and its cleanup is deactivated like a moved resource's (D) | fixture `after-call-read` (a read of `x` after the call must keep its value: the analysis refuses) |
| S3 | "If evaluating the right-hand side fails, the old value remains live" (§14.2) | **Yes**, if the callee changes the block in place and then fails, and something reads `x` afterwards | Hand-over requires `x` not in `trap_live` (no function-level handler reads it). An inline `TRAP` is desugared before NIR, so a `RECOVER x` or a handler read makes `x` live after the op, and hand-over is refused (C: H3). If nothing reads `x` on the failure path, the old value is unobservable. | fixtures `trap-reads-old`, `recover-old`, `fn-trap-reads-old` |
| S4 | Destructive update may not change ownership behaviour (§14 intro) | No | The block has exactly one owner at every point: the caller's `x` until the call, then the callee's parameter, then the return slot. Moved-from bindings are not dropped (§14.7). | runtime harness `before = x` check (E) |
| S5 | "A `MUT` collection buffer may be destructively updated only while it is owned by that single live `MUT` binding" (§14.6) | **The text, not the behaviour.** An owned parameter, and B's `LET x` at its last use, are not `MUT` bindings. | Amend §14.6 in letter F (text below). The behaviour is covered by S4: one owner, not read again. | `mfb spec` render check (F) |
| S6 | No two live bindings share a collection buffer (§14.6); containers never alias (§14.6) | No | Copy-insertion guarantees an owned local has its own block. The same `x` passed twice in one call is two reads, so hand-over is refused (C: H3). | fixture `same-arg-twice` |
| S7 | A global argument keeps the value the global had at the call (§14.3) | Would, if a global were handed over | Globals are never handed over (C: H5) | fixture `global-arg` |
| S8 | Resources: pointer semantics, close-once, drop order (§15, §14.7) | Would, if a resource-bearing value were handed over | Excluded by type (`type_contains_resource`, C: H2) | fixture `res-in-list-arg` (a `List OF RES`) |
| S9 | Closures capture `LET`s by value; `forEach` by-ref `MUT` capture (§14.4, plan-142-G) | Would, if a captured or by-ref local were handed over | Excluded (C: H1, the `excluded_roots` set) | fixture `captured-arg` |
| S10 | Threads: values crossing a thread boundary (§16) | No | `thread::start`/`transfer` are builtins, not user calls; `ISOLATED` entry points are not called directly (C: H7) | — |
| S11 | `Error.source` is stamped at the origin (file, line, char) and never rewritten (§8.5a) | Would, if a variant carried different spans | A variant is a NIR clone of the same function with the same spans (D). The fixture compares `err.source` from the owned path with the lent path. | fixture `error-source-same` |
| S12 | Diagnostics unchanged (§14.1) | No | No new rule, no verifier change (non-goals) | full suite, F |
| S13 | Function identity: a function value, `LINK`, exported symbols | Would, if the base symbol changed | The base function is emitted unchanged. Variants are extra, internal, and only called by direct calls the analysis approved (D). | fixture `function-value-call` |
| S14 | Tooling that counts functions (`mfb test --coverage`) | UNVERIFIED | Task A.3 below: find what coverage keys on; D must map a variant to its source function | task A.3 |

**§14.6 amendment (text for letter F):** "A collection buffer may be destructively
updated only while exactly one binding owns it and that binding's current value is
not read again after the update — a single live `MUT` binding, or any owned binding
(including a parameter the caller handed over) at its last use. No program can
observe the difference from a copy."

## 3. Design Overview (letter A)

A writes no compiler code. It fixes the semantics in writing and puts two sets of
tests in place:

- The **RED allocation tests** turn green one by one in B, D and E.
- The **GREEN semantics fixtures** must never go red.

Method is plan-142's (Correction A4). Run the statement `N` and `2N` times under
`mfb build --debug`, sum `arena.<k>.alloc_calls`, and require
`count(2N) − count(N) < N/8`. A copying path allocates at least once per call, so it
fails.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work; `- [~]` for partial with one line on what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — The semantics fixtures (GREEN)

Pins every observable rule in §2.3 before any code changes.

- [x] Add `tests/rt-behavior/memory/owned-argument-semantics-rt/`, with the same
      layout as `tests/rt-behavior/arithmetic/float-call-boundary-finite-rt/`:
      `project.json`, `src/main.mfb`, and `golden/` holding `build.log`,
      `owned_argument_semantics_rt.{ast,ir,run}`. Generate the goldens with the
      harness, then read the `.run` by hand against the expected values. One `SUB`
      per §2.3 fixture name, each printing a line. The nine cases:
      - `after-call-read` (S2): `LET y = helper(x, 1)` then print `x`;
      - `trap-reads-old` (S3): inline `TRAP` handler prints `len(x)` after a helper
        that appends, then `FAIL`s;
      - `recover-old` (S3): `x = helper(x) TRAP(e) RECOVER x END TRAP`, print `x`;
      - `fn-trap-reads-old` (S3): function-level `TRAP` prints `len(x)`;
      - `same-arg-twice` (S6): `x = pair(x, x)`;
      - `global-arg` (S7): the helper reassigns the global it was passed;
      - `captured-arg` (S9): the helper argument is captured by a lambda called after;
      - `error-source-same` (S11): the same failing helper called both at a hand-over
        site and at a lent site; print `err.source.line`/`char`;
      - `res-in-list-arg` (S8): a `List OF RES` threaded through a helper; the
        resource is closed once, at its owner's exit.
- [x] Also add `function-value-call` (S13): the same helper called through
      `LET f = helper` and directly, with equal output. Prints
      `function-value-call equal=TRUE n=3 x=2` — equal results either way, and `x`
      intact after both calls.

Acceptance: the fixture passes at HEAD, and its `.run` golden shows the values §2.3
expects: the old value where a handler or a later read sees it, the global's value
at the call, and equal `err.source` lines.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/owned-accept 'owned-argument-semantics*'`
  → **`acceptance tests passed (1 test(s) ran)`**, 0 diffs (2026-09-22, at HEAD with
  no compiler change). The twelve printed lines, each verified by hand against §2.3:

  ```
  after-call-read x=2 y=3                              S2  x intact after the call
  trap-reads-old x=2 code=7                            S3  handler sees the OLD x (helper had grown it to 3, then failed)
  trap-reads-old y=2                                   S3  RECOVER x yields the old value
  recover-old x=3                                      S3  x unchanged across the failed RHS
  fn-trap-reads-old x=4 code=7                         S3  function-level handler sees the old x
  fn-trap-reads-old ret=4
  same-arg-twice x=3                                   S6  pair([1,2],[1,2]) = append([1,2], 2)
  global-arg n=2 g=5                                   S7  the argument is the global's value AT THE CALL, not the 5 the callee stored
  captured-arg f=2 y=3                                 S9  the lambda's captured copy is unaffected
  error-source-same same=TRUE bLen=2 r=0               S11 identical origin from the hand-over and the lent site; b intact
  res-in-list-arg n=2                                  S8  two handles, closed once at the owner's exit (exit 0)
  function-value-call equal=TRUE n=3 x=2               S13 same result through `LET f = addOne` and directly
  ```
Commit: (this commit)

### Phase 2 — The RED allocation tests

Five shapes, RED today, each turned green by a named later letter.

- [ ] Add `tests/runtime/rt_owned_argument.rs`, modelled on
      `tests/runtime/rt_global_self_update.rs` (N/2N `alloc_calls` under
      `mfb build --debug`). Each case is marked with the letter expected to turn it
      green:
      1. `local-return` (B): `RETURN collections::append(out, x)`, `out` a local;
      2. `helper-append` (D): `acc = addOne(acc, i)`;
      3. `helper-map-set` (D): `m = put(m, i)`;
      4. `helper-concat` (D): `s = grow(s, "x")`;
      5. `recursive-fill` (E): `RETURN fill(collections::append(xs, n), n - 1)`.
- [ ] Mark the five cases `#[ignore = "plan-147-<letter>"]` so the suite stays green,
      and add one non-ignored test asserting that the ignored set is exactly those
      five. That test is what each later letter edits when it un-ignores a case.

Acceptance: each case fails when run explicitly, naming itself.
  Check: `cargo test --test rt_owned_argument -- --ignored` → 5 failed, each message
  naming its case (est. 4 min: 10 debug builds).
Commit: —

### Phase 3 — Close the one UNVERIFIED semantics row

- [ ] S14: read how `mfb test --coverage` attributes execution
      (`rg -n 'coverage' src/cli src/target/shared -g '*.rs' | head`). Record in §2.3
      whether it keys on the function symbol, the NIR function, or source spans, and
      what letter D must do so a variant's execution counts for its source function.
      If it keys on spans, write "no action".

Acceptance: row S14's "What keeps it" names the mechanism and the D task, or "no
action" with the code citation.
  Check: `rg -n 'S14' planning/plan-147-A-semantics-audit-and-red-tests.md` shows no
  `UNVERIFIED` (est. 1 min).
Commit: —

## Validation Plan

- Tests: the GREEN fixture (Phase 1) and the RED harness (Phase 2).
- Coverage check: Phase 2's guard test fails if a case is dropped or un-ignored
  without the letter's change.
- Runtime proof: none in A (no behaviour change).
- Doc sync: this file only. The §14.6 amendment lands in F with the behaviour, per
  `.ai/spec-content.md`'s as-is-at-HEAD rule.
- Final gate: plan-147-F only.

## Open Decisions

- Should a global argument ever be handed over? **Recommended: no, not in plan-147.**
  The alternative needs `call_reaches_store`-style reasoning about the callee
  reading the global mid-call. S7 already has a working snapshot rule, and the
  census shows no global-threading helper worth it.

## Corrections

- **2026-09-22, gate run (`/follow-plan 147`): stopped at Prerequisites; no plan work
  started.** Re-ran all three commands and updated all three Status cells:
  - The plan-145 row moved NOT MET -> **MET**. `ls planning/plan-145-*` -> no matches;
    the nine letters are in `planning/completed/`, landed through `096acb8bd`.
  - The plan-146 row is still **NOT MET**. `ls planning/plan-146-*` lists A-H, and
    `grep -c '^- \[ \]'` over worktree `P-146` counts 60 unticked boxes (A done, B
    partial with 7 remaining, C-H untouched). plan-146 is a precondition of plan-147,
    never scope for it, so nothing from it was absorbed or hand-rolled here.
  - The seam-files row moved `(re-run)` -> **CHANGED**. Nine plan-145 commits have
    touched `src/codegen/collection/assign/self_update.rs` and
    `src/codegen/engine/analysis/last_use.rs` since `2f55eb184`. Per that row's own
    instruction, **sections 2.1 and 2.2 of this file are stale and must be re-read and
    corrected before letter A starts** -- in particular section 2.1's
    `ENABLED_SITES = [Local, ForEach, Lambda, Global]` line (plan-145 extends both
    `ENABLED_SITES` and `SELF_UPDATE_TABLE` with field sites) and its `excluded_roots`
    description. plan-146 will edit the same two files again, so that re-read is best
    done once plan-146 has landed, not now.

- **2026-09-22, gate re-run (`/follow-plan 147`): all three rows now pass; the plan
  starts.** Re-ran all three commands in worktree `P-147` (merged up to main
  `13f2e9fcc`):
  - plan-145 row: **MET**, unchanged (`ls planning/plan-145-*` -> no matches).
  - plan-146 row: NOT MET -> **MET**. `ls planning/plan-146-*` -> no matches; the
    eight letters are archived under `planning/completed/`, landed on main through
    `13f2e9fcc plan-146: archive A-H to planning/completed`. The 60 unticked boxes
    the previous gate run counted are all resolved.
  - Seam-files row: **CHANGED**, now 19 commits since `2f55eb184` (the nine plan-145
    ones plus ten from plan-146, newest `d648d0231`). Per that row's instruction §2.1
    and §2.2 were re-read and corrected; see the two entries below.

- **§2.1 corrected (seam files moved under plan-145/146).** Two claims were stale:
  - `ENABLED_SITES` is at `self_update.rs:2496`, not `:1303`. Its **value is
    unchanged** (`[Local, ForEach, Lambda, Global]`), so nothing this plan rests on
    moved. plan-145 added a test-only `FIELD_SITES` list beside it.
  - `ops_hold_self_update` (`self_update.rs:656`) gained plan-145's field arms
    (`with_holds_field_self_update`; the `gR = WITH gR { f := g(gR.f, …) }` arm at
    `:676`). It still matches **only** `NirOp::Assign` and `NirOp::StoreGlobal`, so
    §2.1's load-bearing claim — `RETURN OP(x, …)` is never a self-update site — holds
    at HEAD and letter B's premise is intact.
  - Everything else in §2.1 verified unchanged at its stated line:
    `last_use.rs:912` `collect_last_use_moves`, `:647` `excluded_roots`, `:214`/`:230`
    `trap_live`, and `inplace_dest.rs`'s G3 guard (`:287`, `:296`).

- **§2.2 populations re-measured (2026-09-22), two cells corrected.** Re-ran the
  regex plus an enclosing-`FUNC` parameter-name pass
  (`rg -P -g '*.mfb' -g '*.rs' 'RETURN\s+collections::(append|set|insert|prepend|removeAt|removeKey|add|remove)\(\s*\w+\s*[,)]'`,
  then a Python pass matching the first argument against the nearest enclosing
  `(?:FUNC|SUB)\s+\w+\s*\(([^)]*)\)`'s `(\w+)\s+AS` names). examples (4 param /
  14 local), packages (5/5) and benchmark (485/0) reproduce the plan's numbers
  exactly. Two cells were wrong:
  - **tests: 7 param / 2 local -> 12 param / 0 local.** plan-146 added `String`
    fixtures. This grows letter D/E's test-corpus shape count, not their design.
  - **builtins: 6 param / 0 local -> 2 param / 4 local.** The original pass could not
    see the enclosing `FUNC` because the builtin bodies live inside Rust raw strings
    (`r#"FUNC …`), so it attributed all six to the parameter shape. Checked by hand:
    parameter-shape are `__http_addPart` (`builtins/http/helper_add_part.rs:25`,
    `parts`) and `__regex_setCap` (`builtins/regex/helper_set_cap.rs:12`, `caps`);
    local-shape are `__canvas_glyphFlags` (`helper_glyph.rs:88`, `flags`),
    `__canvas_glyphCoords` (`:125`, `out`), `__canvas_lineEdge` (`:295`, `out`) and
    `__crypto_keccakRound` (`helper_keccak_round.rs:53`, `out`, a `MUT out` local).
    This **moves four builtin sites from letter D/E's population into letter B's** —
    B's in-place `RETURN OP(local, …)` now covers four builtin hot paths it was not
    credited with. No letter is re-split: the work still scales with shapes.

- **Phase 1 acceptance corrected: the values are pinned by `build.log`, not by the
  `.run` golden.** The phase text says to "read the `.run` by hand against the
  expected values". That is not what the harness compares: `scripts/test-accept.sh:575`
  states outright that a `<pkg>.run` golden "is a MERGE TRIGGER" whose "contents are
  never [compared]" — its only job is to force the full `mfb build` + execute path.
  The program's stdout is captured into `build.log`, which **is** an exact-compared
  golden (`:319`). So the twelve semantics lines are pinned by
  `golden/build.log`; `golden/owned_argument_semantics_rt.run` carries the same text
  only for readability, matching the model fixture
  `tests/rt-behavior/arithmetic/float-call-boundary-finite-rt/`. The acceptance
  criterion is unchanged in strength — it is still an exact comparison of every
  printed value, just against the file that is actually diffed.

- **S9's fixture uses `LET x`, not `MUT x`.** The first draft captured a `MUT` local
  in the lambda and the compiler refused it:
  `error[2-203-0019 TYPE_LAMBDA_CAPTURE_UNSUPPORTED]: Lambda captures mutable local
  \`x\`; mutable captures are invalid`. §14.4 is about closures capturing **`LET`s**
  by value, so `LET` is the shape S9 is actually about; the fixture matches the rule.

## Summary

A is the semantic contract. The only rule the design really leans on is §14.2's
"old value remains live". Refusing hand-over whenever anything could read the
old value covers it. The only text that changes is §14.6's `MUT`-only wording.
