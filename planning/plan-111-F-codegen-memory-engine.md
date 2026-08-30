# plan-111-F: type codegen's memory, engine and builtin-package remainder

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-111-E (collections and layout are typed; several of this
letter's files call into them).

The last mechanical codegen letter, and the one that takes codegen to zero: the
value/memory semantics layer, the arena transfer paths, the type-utility module
that other letters have been calling, the resource cleanup paths, the registry's
remaining internals, and the per-package builtin modules.

175 violation sites across 31 files (§2). After this letter, `rg` for any of the
six needle classes anywhere in `src/codegen/` returns nothing, and letter G's
job is to prove it and lock it.

See plan-111-A for the shared prerequisites, the five sanctioned boundaries, the
tiered gate policy, and the rejected alternatives.

References:

- `src/codegen/memory/value/builder_value_semantics.rs:117-171` —
  `lower_default_value(&mut self, type_: &str)`, the plan's canonical example:
  a `match` on `"Nothing" | "Boolean" | "Byte" | "Integer" | …` that then
  re-parses the same `&str` to fill the result's `type_` field. 28 sites.
- `src/codegen/engine/types/type_utils.rs` — 28 sites, including
  `is_collection_type` (`:311`) and `is_result_type` (`:320`), the two `&str`
  predicates letter B stopped calling from the front end. Their `&str`
  signatures die here.
- `src/codegen/resource/mod.rs:51,59,94` — `base_resource_name`,
  `state_type_name`, `builtin_resource_close_function`: the `&str` originals
  whose typed twins letters A and B introduced. 7 sites.
- `.ai/codegen-invariants.md` (record layout, vreg-alloc order, register
  lifetimes) and `.ai/resources-packages.md` (the RES system and builtin-package
  authoring seams). **Read both before starting.**

## Prerequisites

See plan-111-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-111-E complete | E's 25 files read 0 on all six needle classes | NOT MET until E lands |
| Scope re-measured at kickoff | the four census commands from plan-111-A §2, restricted to this letter's file list | UNMEASURED — C, D and E all reduce it |
| Kickoff `N ran` baseline recorded | `scripts/test-accept.sh <target> /tmp/accept-111f` → record `N ran` | UNMEASURED |

## 1. Goal

- Every file in §2's list takes and matches `ParameterType`.
- **`rg` for all six needle classes over `src/codegen/` returns 0 hits.** That is
  this letter's defining outcome; the gate budgets for the entire `codegen`
  directory read 0.
- The `&str` twins that letters A and B superseded are **deleted**, not left
  beside their typed versions: `is_collection_type`, `is_result_type`,
  `base_resource_name`, `state_type_name`,
  `is_builtin_sendable_resource_type`, and `resource_base_type_name` if any
  copy survives.

### Non-goals (explicit constraints)

- See plan-111-A §1 Non-goals — all apply.
- **No record-layout change.** Field offsets, strides, adjacency and the
  inline-headroom rules are untouched. A `rt_*` codegen-inspection test going red
  here is usually a stale hardcoded offset, not a regression — dump the `.ncode`
  before concluding either way (the codegen-inspection memory).
- **No change to resource ownership or cleanup ordering.** `resource_closers`
  routing (bug-374, bug-377) and the `CLOSE BY` resolution path are semantics,
  not representation.
- **No change to arena transfer semantics.** The copy/free/transfer rules in
  `builder_arena_transfer.rs` stay exactly as they are.
- Do not convert `src/target/**`. Its 4 `&str` type params and 2 spelling
  compares are in letter G's sweep, not here.

## 2. Current State

This is the layer beneath the emitters plan-106-E retyped — the reason its census
line 6 recorded 109 surviving parses. `lower_default_value` is the clearest
instance: a caller holding a `ParameterType` renders it, this function matches the
spelling to pick a default, then parses the spelling back to label the result.

### Measured populations

At HEAD (`fd09ea809`), 2026-08-29, tests excluded. `a` = spelling match arms,
`e` = spelling `==`/`!=`, `p` = `&str` type parameters, `parse` =
`ParameterType::parse` sites.

| File (under `src/codegen/`) | a | e | p | parse | total |
|---|---|---|---|---|---|
| `memory/value/builder_value_semantics.rs` | 7 | 2 | 7 | 12 | 28 |
| `engine/types/type_utils.rs` | 8 | 1 | 10 | 9 | 28 |
| `memory/arena/builder_arena_transfer.rs` | 1 | 2 | 13 | 3 | 19 |
| `registry/mod.rs` | 0 | 0 | 3 | 8 | 11 |
| `engine/value/builder_values.rs` | 0 | 2 | 1 | 8 | 11 |
| `builtins/mod.rs` | 5 | 0 | 5 | 1 | 11 |
| `engine/builder/builder_emit_helpers.rs` | 0 | 3 | 0 | 6 | 9 |
| `builtins/general/mod.rs` | 0 | 6 | 2 | 1 | 9 |
| `resource/mod.rs` | 0 | 0 | 7 | 0 | 7 |
| `resource/cleanup/builder_resource_cleanup.rs` | 0 | 0 | 6 | 0 | 6 |
| `engine/function/function_lowering.rs` | 0 | 0 | 1 | 4 | 5 |
| `memory/data/data_objects.rs` | 0 | 0 | 2 | 2 | 4 |
| `memory/marshal/record.rs` | 0 | 1 | 2 | 0 | 3 |
| `engine/builder/mod.rs` | 0 | 1 | 0 | 2 | 3 |
| `engine/validation/validation.rs` | 0 | 0 | 0 | 2 | 2 |
| `engine/analysis/module_analysis.rs` | 0 | 1 | 1 | 0 | 2 |
| `cleanup/thread/builder_thread_cleanup.rs` | 0 | 1 | 0 | 1 | 2 |
| `cleanup/owned/builder_owned_cleanup.rs` | 0 | 0 | 2 | 0 | 2 |
| `engine/control/builder_control.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/term/mod.rs` | 0 | 0 | 1 | 0 | 1 |
| `builtins/strings/gen_graphemes.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/strings/func_to_bytes.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/strings/func_split.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/http/func_route.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/datetime/mod.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/crypto/func_sign.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/crypto/func_seal.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/crypto/func_random_bytes.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/crypto/func_open.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/crypto/func_hash.rs` | 0 | 0 | 0 | 1 | 1 |
| `builtins/crypto/func_generate.rs` | 0 | 0 | 0 | 1 | 1 |
| **Total** | **21** | **20** | **63** | **71** | **175** |

### Verified properties

- **`lower_default_value` re-parses its own `&str` argument** — read
  `src/codegen/memory/value/builder_value_semantics.rs:117-171`: the `"Nothing"`,
  `"Boolean"`, `"Byte" | "Integer" | "Float" | "Fixed" | "Money" | "Scalar"` and
  `"String"` arms each fill `type_: ParameterType::parse(&type_)` from the
  spelling they just matched. Converting the parameter to `&ParameterType`
  deletes both halves at once.
- **`type_utils.rs` already has typed twins for its two hottest predicates** —
  read `:311` (`is_collection_type` → `typed_is_collection_type(&ParameterType)`
  at `:349`) and `:320` (`is_result_type` → a one-line `matches!`). The `&str`
  versions exist only for callers this plan has been removing letter by letter.
- **UNVERIFIED: whether the 8 `registry/mod.rs` parses survive letter C.** C
  collapses the dual API; some of these are its internals. Re-measure at kickoff.
- **UNVERIFIED: whether `resource/mod.rs`'s 7 `&str` params have callers outside
  codegen.** Letter B repointed the front-end callers to typed twins, so they
  should be codegen-internal now. Confirm before deleting the `&str` versions.

## 3. Design Overview

Four phases, ordered so that shared utilities are typed before their callers, and
the highest-blast-radius file (memory/value semantics) lands with the rt suite
already exercising the typed utilities beneath it.

Where correctness risk sits:

1. **`builder_arena_transfer.rs` (13 `&str` params).** Arena transfer decides
   copy-vs-move and what gets freed. A signature change that perturbs which
   branch runs is a leak or a double-free, and neither necessarily reds a
   black-box rt fixture — regalloc and linking can mask it (the
   register/slot/import-bug memory). Phase 3 requires a revert-and-RED check on
   any new test written here, or a `.ncode` inspection test instead.
2. **`builder_value_semantics.rs` (12 parses).** `lower_default_value` feeds the
   inline-`TRAP` desugar's never-observed temp and the `STATE` payload init. A
   wrong default is a wrong *value* on an error path — the least-observed code in
   the compiler.
3. **`resource/mod.rs` + `builder_resource_cleanup.rs` (13 params).** Deleting
   `base_resource_name`/`state_type_name` removes the last consumers of the
   stateful-resource spelling. If letter A's `Stateful` variant got the top-level
   rule wrong, this is where it surfaces — as a resource that fails to close.

Per plan-111-A §3, the per-phase gate is `cargo test --no-fail-fast` (including
`golden.rs`) plus `diag-set-diff.sh`; the cross-target `artifact-gate.sh all` is
letter G's single run.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — the shared type utilities (39 sites)

Type these first; every later phase calls them.

- [ ] `engine/types/type_utils.rs` — convert 10 `&str` params, 8 arms, 1 compare,
      delete 9 parses. **Delete** the `&str` `is_collection_type` (`:311`) and
      `is_result_type` (`:320`) once their last callers are typed; rename
      `typed_is_collection_type` (`:349`) to `is_collection_type`.
- [ ] `resource/mod.rs` — convert 7 `&str` params. Delete `base_resource_name`
      and `state_type_name` in favour of `ParameterType::without_state` /
      `ParameterType::state` (plan-111-A Phase 3). Confirm first that letter B
      repointed every front-end caller.
- [ ] Delete `is_builtin_sendable_resource_type`'s `&str` version, keeping the
      typed twin letter B added.
- [ ] `engine/builder/mod.rs` — 1 compare, 2 parses.
- [ ] Lower these files' gate budgets to 0.
- [ ] Tests: an rt fixture opening, using and dropping a **stateful** builtin
      resource (`File STATE Cursor`), proving cleanup still routes after
      `base_resource_name` is gone. This is letter A's `Stateful` variant's real
      end-to-end proof.

Acceptance: the four files read 0 on all six needle classes; the stateful-resource
cleanup fixture passes; `cargo test --no-fail-fast` green including `golden.rs`
and every `rt_*` test.
Commit: —

### Phase 2 — value semantics and the value builder (39 sites)

- [ ] `memory/value/builder_value_semantics.rs` — convert
      `lower_default_value`/`lower_default_value_inner` to `&ParameterType`
      (`:117`, `:126`), which deletes 7 arms and 12 parses together; convert the
      remaining 7 `&str` params and 2 compares.
- [ ] `engine/value/builder_values.rs` — 2 compares, 1 param, 8 parses.
- [ ] `engine/builder/builder_emit_helpers.rs` — 3 compares, 6 parses.
- [ ] Lower these files' gate budgets to 0.
- [ ] Tests: an rt fixture exercising a fallible call inside an inline `TRAP` for
      each default-able type, so the never-observed `$trap_valN` default is
      actually produced for each converted arm.

Acceptance: the three files read 0 on all six needle classes;
`cargo test --no-fail-fast` green; `scripts/diag-set-diff.sh` 0 differing.
Commit: —

### Phase 3 — arena, marshal, cleanup and function lowering (36 sites)

Highest blast radius in this letter.

- [ ] `memory/arena/builder_arena_transfer.rs` — 13 params, 1 arm, 2 compares,
      3 parses.
- [ ] `memory/data/data_objects.rs` (4), `memory/marshal/record.rs` (3),
      `engine/function/function_lowering.rs` (5).
- [ ] `resource/cleanup/builder_resource_cleanup.rs` (6),
      `cleanup/thread/builder_thread_cleanup.rs` (2),
      `cleanup/owned/builder_owned_cleanup.rs` (2).
- [ ] `engine/validation/validation.rs` (2),
      `engine/analysis/module_analysis.rs` (2),
      `engine/control/builder_control.rs` (1).
- [ ] **For any new test in this phase: revert the change and confirm the test
      goes RED**, or write a `.ncode` inspection test instead. A leak/slot/clobber
      fix rarely reds a black-box rt fixture.
- [ ] Lower these files' gate budgets to 0.
- [ ] Tests: per the bullet above — a transfer/cleanup fixture with a proven RED
      state, not merely a passing one.

Acceptance: the ten files read 0 on all six needle classes; every new test has a
recorded RED-check; `cargo test --no-fail-fast` green including every `rt_*` test.
Commit: —

### Phase 4 — the builtin-package modules (61 sites, 17 files)

Small independent edits; batch by commit, keep each file self-contained.

- [ ] `builtins/mod.rs` (11), `builtins/general/mod.rs` (9),
      `registry/mod.rs` (11 — re-measure; letter C may have cleared these).
- [ ] `builtins/term/mod.rs` (1), `builtins/datetime/mod.rs` (1),
      `builtins/http/func_route.rs` (1).
- [ ] `builtins/strings/{gen_graphemes,func_to_bytes,func_split}.rs` (1 each).
- [ ] `builtins/crypto/{func_sign,func_seal,func_random_bytes,func_open,func_hash,func_generate}.rs`
      (1 each).
- [ ] Lower every remaining `codegen` budget to 0.
- [ ] Tests: the per-package `rt_*` suites cover these; run them explicitly and
      record the counts.

Acceptance: **`rg` for all six needle classes over `src/codegen/` returns 0
hits**, and every `codegen` gate budget reads 0; the letter's end gate below
passes.
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast` — never plain `cargo test`. Note that a red
  `rt_*` codegen-inspection test after this letter is most likely a stale
  hardcoded offset/stride, not a regression — dump the `.ncode` first.
- Gate: `cargo test --test no_type_strings` — the whole `codegen` directory at 0,
  budgets tight.
- Coverage check: `lower_default_value`'s per-type arms and the arena transfer
  branches are the two places to verify coverage rather than assume it. `mfb` is
  a binary crate — measure with `cargo llvm-cov --bin mfb`, not `--lib`, and note
  that integration coverage comes from the uncaptured release subprocess
  (`.ai/build-tooling.md`).
- Runtime proof: `scripts/test-accept.sh` with scratch `/tmp/accept-111f` at the
  kickoff `N ran` and 0 mismatches, plus `MFB_OPT=3 scripts/test-accept.sh`.
- Artifact gate: **not run in this letter** (plan-111-A §3) — letter G's single
  run covers it, and letter G opens with the attribution procedure for any diff.
- Diagnostics: `scripts/diag-set-diff.sh` → 0 differing.
- Doc sync: `.ai/codegen-invariants.md` and `.ai/resources-packages.md` for the
  deleted `&str` helpers.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

None. If a decision appears necessary, the conversion is changing behavior —
stop and record it in Corrections.

## Corrections

<Filled in DURING execution.>

## Summary

Risk is arena transfer (a perturbed copy-vs-move branch is a leak that a
black-box fixture may not red — hence the mandatory RED-check) and the stateful
resource path (the end-to-end proof of letter A's `Stateful` variant, three
letters after it landed).

After this letter `src/codegen/` is string-free. What remains for letter G:
`src/target/**` (4 params, 2 compares), `src/binary_repr/`'s non-wire residue,
the final tree-wide census, the single `artifact-gate.sh all` run, and locking
the gate at hard zero.
