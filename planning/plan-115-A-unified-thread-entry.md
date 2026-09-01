# plan-115-A: Any `ISOLATED FUNC` is a thread entry

Last updated: 2026-08-31
Overall Effort: large (3h–1d) — the whole plan-115 feature, split A → B → C
Effort: medium (1h–2h)
Depends on: nothing (first letter)

`thread::start` currently accepts an entry only when it is reached *through an
import*: an imported package's `EXPORT ISOLATED FUNC`, or — inside a
`kind: "package"` project — one of its own via the reserved `IMPORT self`
specifier. This letter deletes the import-reachability requirement and the
visibility requirement, leaving `ISOLATED` as the sole marker.

After this letter: **any `ISOLATED FUNC` declared in the current project is a
valid `thread::start` entry, whatever its visibility (`PRIVATE`, `PUBLIC`, or
`EXPORT`) and whatever the project's `kind`.** An imported package's
`EXPORT ISOLATED FUNC` remains valid exactly as today. The thread's global
namespace is a fresh instance of the project that *declares* the entry.

This letter does not remove `IMPORT self` — `self::worker` keeps working
throughout, because it canonicalizes to the bare name before lowering
(`src/ir/lower.rs:3047`). That removal is letter B, and it is only safe once
bare entries are accepted here.

References:

- `mfb spec language threads` §16 — the entry-point rules this letter rewrites.
- `mfb spec language functions` §6 — the `ISOLATED` declaration rule.
- `mfb spec language modules-and-packages` §13 — visibility scoping
  (`PUBLIC`/`EXPORT` are project-wide; only `PRIVATE` is file-local).
- `planning/completed/plan-81-import-self.md` — the plan that introduced
  `IMPORT self`; its §4.3/§4.4 are the design this supersedes.
- `bugs/completed/` — bug-227 (the `PRIVATE ISOLATED` rejection this letter
  lifts) and bug-369 (worker arena global sizing, the runtime invariant this
  letter must not disturb).
- `AGENTS.md` — "Never edit a test/golden to pass"; the four-question gate
  applies to every behavioral fixture this letter touches.

## Prerequisites

**These are a precondition on the whole plan-115 feature (A, B and C), not a
dependency to negotiate.** Letters B and C point here.

| Must be true | Command | Status |
|---|---|---|
| bug-480 (package name resolution) is fixed and archived | `ls bugs/bug-480-*.md` → no matches (moved to `bugs/completed/`) | **MET** (measured 2026-09-01: `ls bugs/bug-480-*.md` → `no matches found`; `ls bugs/completed/ | grep 480` → `bug-480-package-name-resolution.md`) |
| bug-482 (`thread::start` input sendability never fires) is fixed and archived | `ls bugs/bug-482-*.md` → no matches (moved to `bugs/completed/`) | **NOT MET** (measured 2026-09-01: `ls bugs/bug-482-*.md` → `bugs/bug-482-thread-start-input-sendability-check-never-fires.md`, still Open; and the defect is live in source — the `imported_entry` early-return the bug names is verbatim present at `src/ir/verify/resources.rs:691-696`, found by `grep -rn "imported_entry" src/`) |

**bug-482 is not merely an unarchived doc — it reproduces at main's tip.**
Measured 2026-09-01 by `/follow-plan 115` at commit `781a82f07`, with
`target/release/mfb` rebuilt from that tip (a bug report is not evidence; the
run is):

```
$ ./target/release/mfb build /tmp/b482/wpkg
Wrote package to /tmp/b482/wpkg/wpkg.mfp
$ ./target/release/mfb build /tmp/b482/consumer
Wrote executable to /tmp/b482/consumer/build/consumer.out
[exit 0]
$ /tmp/b482/consumer/build/consumer.out
worker returned 43 (expected 43)
[exit 0]
```

That is bug-482 Case 1: a **capturing** `LAMBDA() -> captured + 1` is passed as
`thread::start`'s `data`, builds clean with no `TYPE_THREAD_NOT_SENDABLE`, and is
invoked on the worker — dereferencing a closure environment in the *parent's*
arena. Corroborating static evidence that the check has never fired:
`grep -rn "Call to .thread.start. input" tests/ | wc -l` → **0**, i.e. not one
golden in the corpus carries that diagnostic, while the type-driven walk's
sibling message ("Thread message type requires …") is pinned in
`tests/syntax/threads/func_thread_start_invalid/golden/build.log`.

**Why bug-480 gates this plan.** bug-480 Defect B is that an imported package's
value types resolve *without* their required prefix while the correctly prefixed
spelling *fails*. That is the same package-keyed name table this plan's entry
resolution reads through (`src/ir/shape.rs:2095-2132`,
`src/ir/lower.rs:canonical_import_name`). Landing a resolution change on top of a
known-broken resolution table destroys attribution: a failure after this plan
cannot be told apart from bug-480's, and the repo's own discipline is that
distinguishing new breakage from pre-existing noise is the whole game. Fix the
table first, then change what reads it.

**Why bug-482 gates this plan.** Letter A modifies the `imported_entry`
early-return at `src/ir/verify/resources.rs:588-595` — the exact gate bug-482
identifies as the likely reason the input-sendability check is dead. This plan
widens the set of accepted entries, which makes the same-project path the common
one. Landing that on a check that never fires means shipping a wider hole and
having no way to verify the gate change, because the code behind it is
unreachable either way. bug-482 must be fixed so that `require_thread_sendable`
is live *before* this letter changes who reaches it.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you.

Everything below is written against the world where both hold. There are no
hedges for the world where they don't.

## 1. Goal

A `thread::start` entry is valid **iff** it names an `ISOLATED FUNC` — either one
declared in the current project (any visibility, any project `kind`) or an
imported package's `EXPORT ISOLATED FUNC`. Concretely, all four of these compile
and run correctly after this letter, and the first two are rejected today:

1. An executable declaring `PUBLIC ISOLATED FUNC w(...)` and calling
   `thread::start(w, data)`.
2. Any project declaring `PRIVATE ISOLATED FUNC w(...)` and calling
   `thread::start(w, data)` from the same file.
3. A package calling `thread::start(self::w, data)` — unchanged from today.
4. A consumer calling `thread::start(pkg::w, data)` — unchanged from today.

And the worker in cases 1 and 2 observes the same isolation cases 3 and 4 already
prove: its own fresh copy of the declaring project's globals, initialized from
the same declarations, with the parent's copy untouched.

### Non-goals (explicit constraints)

- **No runtime or codegen change.** The worker arena sizing
  (`arena_global_slots`, `src/codegen/engine/builder/mod.rs:1158`), the arena
  block allocation (`src/codegen/runtime/thread/runtime_helpers.rs:608`) and the
  trampoline's initializer run (`arena_init.in_run_order()`, same file line 1097)
  are all correct as-is and must not be touched. This letter only changes which
  entries the front end *accepts*.
- **No change to `is_thread_sendable`** (`src/ir/verify/resources.rs:416`).
  `Func` and `ThreadHandle` stay non-sendable.
- **`EXPORT` keeps exactly one meaning** — "writes a symbol into the compiled
  `.mfp` public API" (`src/ir/shape.rs:205`). `EXPORT_IN_EXECUTABLE` stays.
  This plan does **not** give executables an exported interface.
- **`ISOLATED` stays invalid on `SUB`, lambdas, closures and local functions.**
  Only the *visibility* half of `TYPE_ISOLATED_NOT_VISIBLE` is lifted.
- **`IMPORT self` keeps working** through this letter. Its removal is B.
- **No `.mfp` binary-format change**, so no `.mfp` regeneration in this letter.

## 2. Current State

### The three seams that reject a bare entry

1. `src/ir/shape.rs:2112` `thread_start_entry_valid` — the source-path check. It
   bails immediately at `name.split_once('.')`, so an unqualified entry is never
   even considered; a `self::`-qualified one takes the `SELF_IMPORT` arm and is
   additionally required to be `Visibility::Export` (line 2121).
2. `src/ir/verify/compat.rs:433-451` — the IR-path check. Already permissive: it
   only requires a `FunctionRef` typed `Func(_, _, true)` (isolated), and
   explicitly defers to `ir::shape` on the source path via `self.source_path`.
   Its message string is stale but its logic largely survives.
3. `src/ir/verify/resources.rs:588-595` — the `imported_entry` early-return,
   which skips the boundary rules for a same-project entry. bug-482 (prerequisite)
   establishes that everything behind this gate is currently dead.

### The visibility restriction

`src/ir/verify/mod.rs:180`:

```rust
if function.isolated && (function.kind != "func" || function.visibility == "private") {
```

The comment above it states the reason: *"An ISOLATED function is a thread entry
point, reached by name from another package's `thread::start`, so it must be a
project-visible FUNC (bug-227)."* Once an entry need not be reached from another
package, that reason no longer holds.

### Verified properties

| Claim | How verified |
|---|---|
| `PUBLIC ISOLATED FUNC` already compiles today; only `PRIVATE` is rejected | Read `src/ir/verify/mod.rs:180`; `accepts_public_isolated_func` at `src/ir/verify/tests.rs:7758` asserts it |
| `self::worker` is pure sugar — it canonicalizes to the bare name before lowering | Read `src/ir/lower.rs:3040-3050`: `if package == SELF_IMPORT { return rest.to_string(); }` |
| A worker gets *initialized* globals, not zeroed ones | Read `src/codegen/runtime/thread/runtime_helpers.rs:1097-1099` — the trampoline runs `arena_init.in_run_order()` (LINK init then global init) before calling the entry |
| The global region is whole-program, not per-package | Read `src/codegen/engine/builder/mod.rs:1042`: `globals_base = module.globals.len() + package_global_count`; the worker arena is sized from that same single number |
| The per-module namespace model is enforced by *scoping*, not by partitioning | Follows from the two rows above plus §13 visibility: a worker can only *name* its declaring project's globals plus its imports'; the extra slots are unreachable by name. **This is the design's load-bearing premise — see §3.** |
| `PUBLIC`/`EXPORT` are project-wide; only `PRIVATE` is file-local | `src/docs/spec/language/13_modules-and-packages.md:10-25` |

### Measured populations

| What | Count | Command |
|---|---|---|
| `SELF_IMPORT` seams in `src/` | 10 occurrences, 6 files | `grep -rn "SELF_IMPORT" src/ \| wc -l` → 10 |
| `thread::start(self::` call sites | 8, in 4 files | `grep -rn "thread::start(self::" --include="*.mfb" tests/ examples/ tools/ \| wc -l` → 8 |
| `ISOLATED FUNC` declarations tree-wide | 84 | `grep -rnE "EXPORT ISOLATED FUNC\|PUBLIC ISOLATED FUNC\|^[[:space:]]*ISOLATED FUNC" --include="*.mfb" tests/ examples/ tools/ \| wc -l` → 84 |
| rt-behavior thread fixtures | 46 | `ls -d tests/rt-behavior/threads/*/ \| wc -l` → 46 |
| syntax thread fixtures | 34 | `ls -d tests/syntax/threads/*/ \| wc -l` → 34 |
| `TYPE_ISOLATED_NOT_VISIBLE` sites | 8 occurrences, 6 files | `grep -rn "TYPE_ISOLATED_NOT_VISIBLE" src/ tests/ \| wc -l` → 8 |

## 3. Design Overview

Three independent edits, layered: relax the declaration rule, relax the entry
rule, then prove the semantics with a runtime fixture.

**Where design uncertainty concentrates — schedule first.** The one premise this
plan rests on that is *not* directly implemented anywhere is: "a worker can only
name its declaring project's globals." Nothing enforces it as a rule; it falls
out of visibility scoping. If that is wrong — if some path lets a worker reach a
global of a project other than the entry's — the namespace model in the goal is
false and the spec text in letter C would be a lie. **Phase 1 is the cheapest
experiment that tests it**, and it is scheduled first for that reason.

**Where correctness risk concentrates — schedule last.** Phase 3's rt fixture is
the only place a wrong answer is silent (a worker reading the wrong arena
produces a number, not a crash). It lands behind Phases 1–2.

**Byte-identity is NOT this plan's gate.** This letter changes which programs
compile; behavior legitimately changes. The gate is rt-behavior plus syntax
fixtures. Codegen for every *currently valid* program should be unchanged, and
`scripts/artifact-gate.sh all` is run as a *sentinel* on that — but a diff there
is a bug to root-cause (objdump one fixture), not a signal the design is dead.
No target is expected to diff in this letter.

### Rejected alternatives

- **Allow `IMPORT self` in an executable** (relax `IMPORT_SELF_IN_EXECUTABLE`
  alone). Useless on its own: `self::` resolves only to `EXPORT` declarations
  (`src/ir/shape.rs:2121`) and an executable cannot declare `EXPORT`
  (`EXPORT_IN_EXECUTABLE`, `src/ir/shape.rs:207-250`), so the namespace would
  bind empty. Would require giving executables an export surface, splitting
  `EXPORT` into two kind-dependent meanings.
- **Keep the visibility requirement, lift only import-reachability.** Leaves
  `PRIVATE ISOLATED` rejected with no surviving rationale — bug-227's stated
  reason ("reached by name from another package") is exactly what this plan
  deletes.
- **Give the worker *no* globals at all** ("gets nothing"). Requires a new
  transitive reachability analysis (entry + everything it calls may not touch a
  top-level binding, including through function values); `ISOLATED` enforces
  nothing about state access today. Without that analysis, skipping the
  initializer yields silent zeroes — a global reading 5 on the main thread reads
  0 in the worker with no diagnostic. It would also make the same source function
  behave differently depending on host project kind.

## Compatibility / Format Impact

- **Source compatibility: additive.** Every program valid today stays valid.
  `PRIVATE ISOLATED FUNC` and bare-name entries become newly valid.
- **One diagnostic narrows:** `TYPE_ISOLATED_NOT_VISIBLE` (`2-203-0113`) stops
  firing on the visibility condition and fires only on `kind != func`. Its rule
  code and name are retained (renaming is C's call, if at all).
- **No `.mfp` format change**, no binary-repr version bump, no golden `.mfp`
  regeneration in this letter.
- **No runtime/ABI change.**

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` **in the same
> commit as the work it describes**. Use `- [~]` for partial with one line on
> what remains. Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather
> than deleting it. Fill each `Commit:` the moment the phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — Falsify or confirm the namespace premise

Cheapest experiment that could kill the design. No production code changes.

- [ ] Write a throwaway probe: a package `P` with `MUT PCOUNT = 7` and an
      `EXPORT ISOLATED FUNC`, consumed by an executable with its own
      `MUT ECOUNT = 11`. Start P's entry; have it mutate `PCOUNT` and return it.
      Confirm the worker sees `PCOUNT = 7` (its own fresh copy) and that the
      executable's `ECOUNT` is unreachable by name from inside P.
- [ ] Repeat with the entry declared in the *executable* (using `self::` is not
      available there yet — use a temporary local patch to `thread_start_entry_valid`,
      or defer this half to Phase 3's fixture and note it here).
- [ ] Record the outcome in §Corrections. If a worker can reach a global of a
      project other than the entry's declaring project, **stop and re-scope**:
      the goal's namespace sentence is wrong and letter C's spec text must change.

Acceptance: the probe runs and its printed values match the "fresh copy of the
declaring project's globals, parent untouched" prediction — or the premise is
recorded as falsified with the observed values.
Commit: —

### Phase 2 — `ISOLATED` becomes orthogonal to visibility

Lifts the bug-227 restriction. Independently landable: it widens what declares,
not what `thread::start` accepts, so no entry becomes valid yet.

- [ ] `src/ir/verify/mod.rs:180` — drop the `|| function.visibility == "private"`
      condition, leaving `if function.isolated && function.kind != "func"`. Update
      the doc comment above it: the bug-227 rationale ("reached by name from
      another package's `thread::start`") no longer holds; state instead that
      `ISOLATED` marks thread-entry eligibility and is independent of visibility.
- [ ] `src/rules/table.rs` — update the `TYPE_ISOLATED_NOT_VISIBLE` message to
      drop the visibility clause. Keep the code `2-203-0113` and the name.
- [ ] `src/ir/verify/mod.rs` — update the emitted `format!` detail string
      (currently "must be a project-visible FUNC declaration (PUBLIC — the
      default — or EXPORT, not PRIVATE)").
- [ ] `src/ir/verify/tests.rs:7742` `rejects_private_isolated_func` — this test
      protects the behavior being deliberately changed. Work the four-question
      gate from `AGENTS.md` in the commit message: it was written for bug-227,
      it protects "an ISOLATED entry is reachable from another package", the
      dependants are the fixture below, and the disproof is this plan's §3
      rejected-alternative analysis. Convert it to
      `accepts_private_isolated_func`; do **not** delete `rejects_isolated_sub`.
- [ ] `tests/syntax/functions/bug227_private_isolated_func_invalid/` — the
      fixture now describes lifted behavior. Replace it with
      `tests/syntax/functions/private_isolated_func_valid/` (builds clean),
      and keep a sibling pinning `ISOLATED SUB` rejection if none exists
      (`grep -rn "ISOLATED SUB" tests/syntax/`).
- [ ] `src/docs/spec/language/06_functions.md:42` and
      `src/docs/spec/language/13_modules-and-packages.md:33` — both currently
      state the project-visible requirement. Update both to "any top-level
      `FUNC`". (Line 42 was corrected on 2026-08-31 to match the *old*
      implementation; it now needs the new one.)

Acceptance: `PRIVATE ISOLATED FUNC w(...)` in a fresh project builds with exit 0
and no diagnostic; `ISOLATED SUB` still emits `TYPE_ISOLATED_NOT_VISIBLE`;
`cargo test --no-fail-fast` green.
Commit: —

### Phase 3 — A bare `ISOLATED FUNC` is a valid entry (largest blast radius)

The semantic change. Lands last because it is the one that can silently
mis-route a worker.

- [ ] `src/ir/shape.rs:2112` `thread_start_entry_valid` — restructure. An
      unqualified name is now valid when `self.functions` has it and the entry is
      `isolated` and `kind == Func`; drop the `Visibility::Export` condition from
      the `SELF_IMPORT` arm (a `self::` entry is now just a spelling of the bare
      one). The qualified-import arm is unchanged.
- [ ] `src/ir/shape.rs:2141` `report_thread_entry` — replace the message
      "thread.start entry point must be an exported ISOLATED FUNC from an imported
      package." with one describing the new rule ("must name an `ISOLATED FUNC`").
      Same string at `src/ir/shape.rs:3908` (test expectation) and
      `src/ir/verify/compat.rs:450`.
- [ ] `src/ir/verify/compat.rs:433-451` — update the stale comment block; the
      logic (requires an isolated `FunctionRef`) already admits the new shape.
- [ ] `src/ir/verify/resources.rs:588-595` — delete the `imported_entry`
      early-return so the boundary rules run for every entry. **This depends on
      bug-482 being fixed** (prerequisite): confirm `require_thread_sendable` at
      line 598 actually fires before and after this edit, or the change is
      unverifiable.
- [ ] Confirm whether `is_package` is reachable in the shape checker's context at
      line 2112. `src/ir/shape.rs:213` has an `is_package: bool` but it is in
      `export_in_executable_diagnostics`, a *different* pass. If the checker
      lacks it, thread it through — **but first check whether it is needed at
      all**: under the new rule a bare entry is valid in both project kinds, so
      the predicate may not need `is_package`. Prefer not threading it.
- [ ] Tests: new syntax fixture
      `tests/syntax/threads/thread-start-local-entry-valid/` — an **executable**
      with `PUBLIC ISOLATED FUNC` and a `PRIVATE ISOLATED FUNC`, both started
      bare, building clean.
- [ ] Tests: new syntax fixture
      `tests/syntax/threads/thread-start-non-isolated-entry-invalid/` — a bare
      non-`ISOLATED` local function as entry, still rejected. This is the
      guardrail proving the rule narrowed to `ISOLATED` rather than vanishing.
- [ ] Tests: new rt fixture
      `tests/rt-behavior/threads/thread-executable-local-entry-rt/` — an
      executable with `MUT COUNTER = 7`, two bare-started workers adding 100 and
      200, asserting `a=107 b=207 parent=7`. This is the executable-hosted mirror
      of `thread-self-fanout-rt` and is the plan's real proof.
- [ ] `src/docs/spec/language/16_threads.md:28` — rewrite the entry-point rule.
      (The `IMPORT self` sentence stays until letter B; rewrite it again there.)

Acceptance: the new rt fixture prints `a=107 b=207 parent=7` on a real run —
proving an executable-declared worker gets its own fresh instance of the
executable's globals and does not share with the parent or the sibling.
`tests/syntax/threads/thread-start-non-isolated-entry-invalid/` still fails to
build. All 46 rt-behavior and 34 syntax thread fixtures pass unchanged.
Commit: —

## Validation Plan

- **Tests:** three new fixtures (two syntax, one rt-behavior) per Phase 3, plus
  the two converted in Phase 2. Negative cases are explicit: non-`ISOLATED` bare
  entry rejected, `ISOLATED SUB` rejected.
- **Coverage check:** confirm the changed predicates are actually exercised —
  `grep -rn "thread_start_entry_valid" src/` for callers, and verify the new
  syntax fixtures appear in the `test-accept.sh` run count (`N ran`), not silently
  skipped. A green gate that never ran the fixture proves nothing.
- **Runtime proof:** `tests/rt-behavior/threads/thread-executable-local-entry-rt/`
  executed natively, printing `a=107 b=207 parent=7`. Unit tests cannot establish
  this — it is an arena-isolation fact.
- **Doc sync:** `src/docs/spec/language/06_functions.md`,
  `13_modules-and-packages.md`, `16_threads.md`. `01_rule-codes.md` only if a
  message changes materially. Letter C consolidates the narrative.
- **Acceptance:** `cargo test --no-fail-fast` (never a single module — per
  `AGENTS.md`, a failing `golden.rs` silently skips every later `rt_*`);
  `scripts/test-accept.sh`; `scripts/artifact-gate.sh all` as a drift sentinel
  (0 diffs expected — investigate any, do not re-baseline);
  `cargo check --all-targets` at the END for test-target warnings;
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Rename `TYPE_ISOLATED_NOT_VISIBLE`?** Once visibility is irrelevant the name
  misdescribes the rule (it now means "ISOLATED on a non-FUNC"). Recommend
  **keep the code and name** in this letter — a rule-code rename ripples through
  `01_rule-codes.md` and every golden carrying it — and revisit in C. (§Phase 2)
- **Does the shape checker need `is_package`?** Recommend **no**: the new rule is
  kind-independent, so the predicate should not need it. Confirm in Phase 3
  before threading anything through. (§Phase 3)

## Corrections

<!-- Filled in DURING execution. Record every place this plan was wrong: the
     claim, what was actually true, the evidence — and whether letter B or C
     derived scope from the wrong number. -->

- (none yet)

## Summary

The real engineering risk is Phase 3's `resources.rs` gate deletion, and it is
risk *inherited*, not created: bug-482 must have made that code path live first,
or the edit cannot be verified. The namespace premise in Phase 1 is the only
unproven claim, and it is cheap to test.

Untouched: all codegen and runtime (arena sizing, the trampoline's initializer
run, queue machinery), `is_thread_sendable`, `EXPORT`'s single meaning,
`EXPORT_IN_EXECUTABLE`, and `IMPORT self` itself — which still works when this
letter lands and is removed in B.
