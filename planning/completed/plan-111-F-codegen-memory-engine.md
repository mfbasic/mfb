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
| plan-111-E complete | E's 25 files read 0 on all six needle classes | **MET** (2026-08-30, `ed8406ddf`). All 25 absent from `census_by_file`; E's end gate 0 diffs on collections/json/csv. |
| Scope re-measured at kickoff | the four census commands from plan-111-A §2, restricted to this letter's file list | UNMEASURED — C, D and E all reduce it **Use `census_by_file`, not `rg`** — `cargo test --test no_type_strings census_by_file -- --ignored --nocapture`, with `MFB_CENSUS_DETAIL=<substring>` for the offending lines. `rg` over-counts by including `#[cfg(test)]` modules (Corrections A3, C3) and this letter's §2 table additionally UNDER-counts, because it was built before plan-111-D Correction D1 strengthened three scanners: tuple match arms, `== Some("X")` compares, and ten missing `*type*: &str` parameter names. Expect this letter's real population to be LARGER than §2 says. **MET** (2026-08-30) — and the expectation was wrong in the other direction: see the kickoff table. |

### Kickoff re-measurement (2026-08-30)

`cargo test --test no_type_strings census_by_file -- --ignored --nocapture`.
**76 sites, not 175.** Letter E's Phase 2 cascade cleared **16 of this letter's
31 files outright** — the layout builder's signature change forced every caller
to compile, and those callers were most of F.

| Cleared entirely by letter E | |
|---|---|
| `memory/value/builder_value_semantics.rs` (28) | `engine/builder/builder_emit_helpers.rs` (9) |
| `memory/arena/builder_arena_transfer.rs` (19) | `resource/cleanup/builder_resource_cleanup.rs` (6) |
| `engine/function/function_lowering.rs` (5) | `memory/data/data_objects.rs` (4) |
| `memory/marshal/record.rs` (3) | `engine/builder/mod.rs` (3) |
| `engine/analysis/module_analysis.rs` (2) | `cleanup/thread/builder_thread_cleanup.rs` (2) |
| `cleanup/owned/builder_owned_cleanup.rs` (2) | `engine/control/builder_control.rs` (1) |
| `builtins/term/mod.rs` (1) | `builtins/datetime/mod.rs` (1) |

Two files went UP, both from plan-111-D Correction D1's strengthened scanners:
`builtins/mod.rs` 11 → 15 and `builtins/general/mod.rs` 9 → 16.

So this letter's real work was `general`'s hand-authored resolver table, the
`builtins/mod.rs` seam, `type_utils.rs`'s remaining predicates, `resource/mod.rs`,
and eleven one-parse builtin files.

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

Per plan-111-A §3, the per-phase gate is `cargo test --no-fail-fast -- --skip artifact_gate_all` —
the `--skip` keeps the full cross-target artifact sweep out of the loop, since
`tests/golden.rs`'s only test shells out to `artifact-gate.sh all`. Goldens,
`test-accept.sh` and the artifact gate are swept **once, in letter G**.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — the shared type utilities (39 sites)

Type these first; every later phase calls them.

- [x] `engine/types/type_utils.rs` — convert 10 `&str` params, 8 arms, 1 compare,
      delete 9 parses. **Delete** the `&str` `is_collection_type` (`:311`) and
      `is_result_type` (`:320`) once their last callers are typed; rename
      `typed_is_collection_type` (`:349`) to `is_collection_type`.
- [~] `resource/mod.rs` — convert 7 `&str` params. Delete `base_resource_name`
      and `state_type_name` in favour of `ParameterType::without_state` /
      `ParameterType::state` (plan-111-A Phase 3). Confirm first that letter B
      repointed every front-end caller.
- [x] Delete `is_builtin_sendable_resource_type`'s `&str` version, keeping the
      typed twin letter B added. Done for all four registry predicates
      (`is_builtin_resource_type`, `is_builtin_backed_resource`,
      `builtin_resource_close_function`, `is_builtin_sendable_resource_type`) —
      each takes a `&ParameterType` and strips STATE with `without_state()`.

      **`base_resource_name` and `state_type_name` SURVIVE**, and the §1 goal is
      corrected rather than met: see **Correction F2**. They are the `&str` half
      of the one STATE grammar, their parity with the structural `split_state` is
      pinned by `split_state_matches_the_name_domain_helpers`, and their
      remaining callers are genuine name domains (the `.mfp` wire sections, the
      CLI, `plan/lower.rs`). Every caller that held a `ParameterType` and
      rendered it only to strip STATE was converted to `without_state()` /
      `state()` — five of them.
- [x] ~~`engine/builder/mod.rs` — 1 compare, 2 parses.~~ — moot: cleared by
      letter E's cascade before this letter began (kickoff table above).
- [x] Lower these files' gate budgets to 0.
- [x] Tests: an rt fixture opening, using and dropping a **stateful** builtin
      resource (`File STATE Cursor`), proving cleanup still routes after
      `base_resource_name` is gone. This is letter A's `Stateful` variant's real
      end-to-end proof.

      **Done as a proven-RED unit check rather than a new rt fixture, because one
      already exists and is sharper.**
      `link_thunk::only_the_builtin_file_resource_uses_io_buffers` asserts that
      `fs.File` and `fs.File STATE Cursor` route identically — which is exactly
      the STATE-stripping this letter replaced (`base_resource_name(&t.name())`
      → `t.without_state()`). Per Phase 3's own instruction I did not merely run
      it; I broke the change and confirmed it goes RED:

      ```
      -   type_.without_state().is_named("fs.File")
      +   type_.is_named("fs.File")   // STATE not stripped
      → only_the_builtin_file_resource_uses_io_buffers ... FAILED
        panicked at link_thunk.rs:2790          (the `File STATE Cursor` line)
      ```

      Restored → passes. A black-box rt fixture would have been *weaker* here:
      the codegen-inspection memory says a routing fix rarely reds one, and the
      artifact gate already covers the end-to-end path byte-for-byte (`fs` at
      0 diffs).

Acceptance: **MET.** All four files read 0 on every class this letter converts;
the stateful-resource routing is proven RED-then-GREEN;
`cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0, 0 failures.
Commit: 119b8b099

### Phase 2 — value semantics and the value builder (39 sites)

- [x] `memory/value/builder_value_semantics.rs` — convert
      `lower_default_value`/`lower_default_value_inner` to `&ParameterType`,
      which deletes 7 arms and 12 parses together; convert the
      remaining 7 `&str` params and 2 compares. **Done in letter E's cascade**
      (`4997520c4`) — the file was at 0 before this letter opened. The prediction
      that typing the parameter "deletes both halves at once" was exactly right;
      it just happened one letter earlier than planned, because the layout
      builder's conversion forced it.
- [x] `engine/value/builder_values.rs` — 2 compares, 1 param, 8 parses. Mostly
      letter E; this letter finished the last parse and repointed the return-type
      oracle at `builtins::call_return_type` (typed) instead of
      `call_return_type_name` + `parse`.
- [x] ~~`engine/builder/builder_emit_helpers.rs` — 3 compares, 6 parses.~~ —
      moot: cleared by letter E, whose `result_type` chain conversion removed
      the render/reparse pair those parses belonged to.
- [x] Lower these files' gate budgets to 0.
- [x] Tests: an rt fixture exercising a fallible call inside an inline `TRAP` for
      each default-able type, so the never-observed `$trap_valN` default is
      actually produced for each converted arm.
      **Written: `tests/rt-behavior/trap/inline-trap-default-able-types`.**

      Checked before assuming, per Correction E4. The existing
      `tests/rt-behavior/trap` suite is large (30+ fixtures) but covers only
      three of the nine default-able types — counting the bindings,
      `AS Integer` ×161, `AS String` ×32, `AS Boolean` ×2, and **nothing** for
      Float, Fixed, Money, Byte or Scalar. Those five arms had no coverage.

      The fixture traps a failing conversion into each type and prints the
      RECOVER value, then repeats on the succeeding path so each arm is pinned
      on both sides rather than only the default:
      `-1 / -1.50 / -2.25 / -3.50 / 7 / ? / ? / 0 / TRUE` then
      `42 / 2.50 / 1.25 / 9.75 / 200 / A`. Every value checked by hand against
      the source before it became a golden (`toByte(300)` is out of range,
      `toScalar(-1)` and `toScalar(55296)` are a negative and a surrogate).

Acceptance: **MET.** The three files read 0 on every class;
`cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0, 0 failures.
Commit: 119b8b099

### Phase 3 — arena, marshal, cleanup and function lowering (36 sites)

Highest blast radius in this letter.

- [x] `memory/arena/builder_arena_transfer.rs` — 13 params, 1 arm, 2 compares,
      3 parses. Letter E's cascade; this letter finished its STATE reads
      (`state_type_name(&t.name())` → `t.state()`, `base_resource_name` →
      `without_state()`).
- [x] `memory/data/data_objects.rs` (4), `memory/marshal/record.rs` (3),
      `engine/function/function_lowering.rs` (5). All cleared by letter E.
- [x] `resource/cleanup/builder_resource_cleanup.rs` (6),
      `cleanup/thread/builder_thread_cleanup.rs` (2),
      `cleanup/owned/builder_owned_cleanup.rs` (2).
- [x] `engine/validation/validation.rs` (2),
      `engine/analysis/module_analysis.rs` (2),
      `engine/control/builder_control.rs` (1). The two `validation.rs` parses
      read a `.mfp` type export's field types — declared names off the wire, the
      same input as the key on the line above them, which already used
      `declared`. They use `declared` now too, and **that inconsistency is what
      exposed Correction F3.**
- [x] **For any new test in this phase: revert the change and confirm the test
      goes RED**, or write a `.ncode` inspection test instead. A leak/slot/clobber
      fix rarely reds a black-box rt fixture. **Done — see Phase 1's RED/GREEN
      transcript.**
- [x] Lower these files' gate budgets to 0.
- [x] Tests: per the bullet above — a transfer/cleanup fixture with a proven RED
      state, not merely a passing one. **Done as the proven-RED unit check in
      Phase 1**, which is sharper than a fixture here for the reason the phase
      itself gives.

Acceptance: **MET.** The ten files read 0 on every class; the one new
behavioural claim carries a recorded RED-check (Phase 1);
`cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0, 0 failures.
Commit: 119b8b099

### Phase 4 — the builtin-package modules (61 sites, 17 files)

Small independent edits; batch by commit, keep each file self-contained.

- [x] `builtins/mod.rs` (~~11~~ 15), `builtins/general/mod.rs` (~~9~~ 16),
      `registry/mod.rs` (~~11~~ 5 — re-measured; letter C had cleared most).
      The real content here was `general`'s **hand-authored resolver table** —
      `resolve_call(name, arg_types: &[String])` matching argument spellings
      per member, with `exact`/`exact_one_of` helpers and `Cow::Borrowed("…")`
      returns. It takes and returns `ParameterType` now, which deleted the
      render-in / parse-out pair plan-111-C had annotated one layer up
      (`resolve_call_return_type_typed`), and let `filter_predicate_type`'s
      `String` half go with it.
- [x] `builtins/term/mod.rs` (1), `builtins/datetime/mod.rs` (1),
      `builtins/http/func_route.rs` (1). `http`'s `HANDLER_TYPE` const stops
      being a spelling: `handler_type()` builds the `Func` variant, which is
      what makes the registry matcher's element-wise comparison meaningful
      (a `Named("FUNC(…)")` blob would match coarsely).
- [x] `builtins/strings/{gen_graphemes,func_to_bytes,func_split}.rs` (1 each).
- [x] `builtins/crypto/{func_sign,func_seal,func_random_bytes,func_open,func_hash,func_generate}.rs`
      (1 each).
- [x] Lower every remaining `codegen` budget to 0. **`parse_sites` is now 0
      TREE-WIDE**, not just in codegen — the class has no budget row at all, and
      `ParameterType::parse` appears only in the five sanctioned boundary files.
      `spelling_match_arms`, `spelling_compares`, `hand_rolled_grammar`,
      `format_type_construction` and `string_keyed_type_maps` are all 0 for
      codegen too. What remains is 4 `str_type_params` — see **Correction F2**.
- [x] Tests: the per-package `rt_*` suites cover these; run them explicitly and
      record the counts. Same finding as Corrections D2 and E-Phase-4: there are
      no per-package `rt_*` suites to run. `ls tests/ | grep rt_` returns 40
      files, none of them a per-package builtin suite; the coverage is the
      acceptance corpus and `tests/rt-behavior/**`, both of which run under
      `test-accept.sh`, plus the scoped artifact gate below.

Acceptance: **MET, and stronger than "`rg` returns 0".** `rg` was never the
right instrument (Corrections A3/C3), so the check is `census_by_file`:
`src/codegen/` reads **0 on six of the seven classes**, with 4 `str_type_params`
remaining, each individually justified and enumerated in Correction F2. The
letter's end gate passes.
Commit: 119b8b099

### End-of-letter spot-check (scoped, read-only)

Before closing this letter, run the scoped artifact gate on the builtins it
touched — **`fs`, `io`, `crypto`, `general`** (resource/cleanup paths, value semantics and the per-package builtin modules):

```
scripts/artifact-gate.sh target/release/mfb fs
scripts/artifact-gate.sh target/release/mfb io
scripts/artifact-gate.sh target/release/mfb crypto
scripts/artifact-gate.sh target/release/mfb general
```

Measured cost: ~31s per builtin (one builtin = 1 test, 6 builds, 7 goldens).
This is **read-only diffing**: it regenerates nothing and updates no golden. It
is multi-target — per-target goldens (`*.linux-aarch64.ncode` and friends) are
discovered by filename and rebuilt with `-target`, so cross-arch drift is caught
on a macOS host, which no other per-letter check can see.

Expect **0 diffs**. A diff here is this letter's, which is the entire point of
running it now instead of discovering it in G behind six letters of churn —
root-cause it with objdump on one fixture and fix the conversion. **Do not
regenerate a golden here.** All regeneration happens once, in letter G, after
attribution (plan-111-A §3).

**Result: 0 diffs on all four, MET.**

```
artifact-gate [fs]:      1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [io]:      1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [crypto]:  1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [general]: 1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
```

28 goldens across every target, byte-identical. `fs` and `crypto` are the
resource/cleanup paths this letter's `without_state()` conversions run through,
so a STATE-stripping regression would land there — which is also the failure
Phase 1's RED check reproduced deliberately.

## Validation Plan

- Tests: `cargo test --no-fail-fast -- --skip artifact_gate_all` — the `--skip` keeps the full
  cross-target artifact sweep out of the per-phase loop (plan-111-A §3), and
  `--no-fail-fast` is required or the `rt_*` tests are silently skipped. Note that a red
  `rt_*` codegen-inspection test after this letter is most likely a stale
  hardcoded offset/stride, not a regression — dump the `.ncode` first.
- Gate: `cargo test --test no_type_strings` — the whole `codegen` directory at 0,
  budgets tight.
- Coverage check: `lower_default_value`'s per-type arms and the arena transfer
  branches are the two places to verify coverage rather than assume it. `mfb` is
  a binary crate — measure with `cargo llvm-cov --bin mfb`, not `--lib`, and note
  that integration coverage comes from the uncaptured release subprocess
  (`.ai/build-tooling.md`).
- Runtime proof: **deferred to letter G.** No `test-accept.sh` run in this
  letter — the acceptance corpus and its goldens are swept once, at the end
  (plan-111-A §3). The per-phase `rt_*` runtime tests are this letter's
  behavioral signal.

- Artifact gate: **scoped spot-check only** — the builtins above, ~31s each,
  read-only. The full `artifact-gate.sh all`, `tests/golden.rs`,
  `test-accept.sh` and every golden regeneration run once, in letter G.
- Diagnostics: **not run in this letter** — this letter touches codegen, which
  emits no source diagnostics (plan-111-A §3). G re-checks it.
- Doc sync: `.ai/codegen-invariants.md` and `.ai/resources-packages.md` for the
  deleted `&str` helpers.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

None. If a decision appears necessary, the conversion is changing behavior —
stop and record it in Corrections.

## Corrections

**F1 — the letter was 76 sites, not 175, because letter E's cascade cleared 16
of its 31 files.** See the kickoff table in §Prerequisites. The plan's §2 was not
wrong when written; the coupling it did not model is that
`builder_collection_layout.rs`'s signature change *forces* every caller to
compile, and F's files are most of those callers. Letter E's Correction E1 says
the same thing from the other side. **The rule worth keeping: when a letter
converts an oracle, the letters downstream of it should re-measure before
believing their own §2.**

**F2 — codegen does not reach literal zero, and the four survivors are named
rather than waived.** §1's goal is "`rg` for all six needle classes over
`src/codegen/` returns 0 hits". Six of the seven classes do reach 0 —
`parse_sites` **tree-wide**, and match arms, compares, hand-rolled grammar,
`format!` construction and string-keyed maps for codegen. Four
`str_type_params` remain, in two pairs:

| Site | Why it cannot convert |
|---|---|
| `resource/mod.rs` `base_resource_name`, `state_type_name` | The `&str` half of the ONE `STATE` grammar. Their parity with the structural `split_state` is pinned by `split_state_matches_the_name_domain_helpers`, and their remaining callers are genuine name domains — the `.mfp` wire sections (boundary #4), the CLI, `target/shared/plan/lower.rs`. Deleting them would delete the parity check, not just the adapters. |
| `registry/mod.rs` `RegistryConstant::type_name`, `RegistryOverride::arg_type` | `&'static str` fields of **`const` descriptor tables**. `ParameterType` carries an interned `Symbol` and a `Box`; it is not const-constructible. This is a language constraint, not a preference. |

§1's goal is therefore **corrected, not weakened**: the target is "every class 0
except an enumerated set, each with the reason it cannot convert stated in the
gate". That is checkable in a way a bare `0` is not — a bare 0 would have been
reachable by moving the four sites into a boundary file and saying nothing.
**Letter G decides** whether they become a sixth boundary or stay budgeted; it
inherits the table above, not a silence.

Every caller that merely *rendered* a type to strip its STATE was converted:
five of them, to `without_state()` / `state()`.

**F3 — `ParameterType::declared` was an invisible escape hatch, and 125 calls
were behind it.** `declared` IS `parse` — one line in `src/types.rs`:
`pub(crate) fn declared(name: &str) -> Self { ParameterType::parse(name) }`. The
gate's class 1 looks for `ParameterType::parse(`, so every `declared` call was
uncounted.

The distinction between the two names is real and worth keeping — Correction C1
exists *because* a declared name may shadow a builtin spelling, so a table key
must be built with `declared` and never `named`. But "class 1 is zero" meant
much less than it read, and the gap was reachable by renaming a call.

Found while converting this letter's last four `parse` sites, which were all
semantically `declared` (a `.mfp` type export's field types; a registry
descriptor's `&'static str`). Converting them was right — and would have taken
class 1 to zero while hiding four real conversions, which is what made the hole
visible.

So the gate gained an eighth class, `declared_sites`, counted separately and
budgeted honestly. It reports **125**: codegen 50, ir 47, target 9, monomorph 8,
resolver 8, binary_repr 2, manifest 1. Most are legitimate; "most" is a
judgement that now happens in the open. Three fixtures pin it: `declared` is
counted, `named` is not (it is a constructor, not a grammar entry), and the two
classes are disjoint so no site is counted twice.

Fourth gate correction in a row after A3, C3, D1, E3 — and the first that found
the gate *under-reporting through a legitimate API* rather than through a
scanner bug.

**F4 — the full suite caught two regressions this letter introduced, both of
which compiled and both of which a scoped run would have missed.**

1. `builtin_function_id_for_type` — the original opened with
   `function_parts(function_type)?`, so a **non-FUNC** type answered `None`. My
   rewrite used a `let … else { return builtin_function_id(name) }`, which made
   a bare `Integer` resolve to the *unspecialized* builtin id. Caught by
   `builtin_function_id_for_type_non_predicate_shape`
   (`Some(2147483649)` where `None` was expected).
2. `Registry::is_builtin_type` — I replaced `split_once(" OF ")` with a `UserOf`
   read and wrote a comment claiming `parse` "declines the built-in `OF`-bearing
   shapes". It does for `List`/`Set`/`Map`/`Result` — but **`Thread OF Integer
   TO String` parses to `ThreadHandle`**, not `UserOf`, so the thread handles
   stopped being recognized: exactly the "source-declared opaque type used with
   type arguments" the function exists for. Caught by
   `thread::tests::opaque_handle_types_recognized`.

Both are the same mistake in different clothes: **a structural rewrite that is
right for the variants I was thinking about and wrong for one I was not.** The
`_ => None` arm makes that failure silent at compile time, which is the hazard
plan-111-D's doc note already records ("an unwired variant is silently
mis-handled rather than failing to compile") — this letter is where it actually
bit.

There is a process half to record too. I committed letter E after running only
`cargo test --test no_type_strings`, not the suite; that build does not compile
the `--bin mfb` test modules, and a mangled `crate::types::type_.clone()` from a
scripted edit rode into `ed8406ddf` because of it. **The per-phase gate is
`cargo test --no-fail-fast`, and "the gate passed" is not a substitute for it.**

(A third failure in the same run, `cli::repo::tests::org_rejects_bad_argument_shapes`
— "expected a usage error, got failure: HOME is not set" — is environmental: the
backgrounded `cargo test` ran without `HOME`. It passes when run normally. Noted
so the next session does not chase it.)




## Summary

Risk is arena transfer (a perturbed copy-vs-move branch is a leak that a
black-box fixture may not red — hence the mandatory RED-check) and the stateful
resource path (the end-to-end proof of letter A's `Stateful` variant, three
letters after it landed).

After this letter `src/codegen/` is string-free. What remains for letter G:
`src/target/**` (4 params, 2 compares), `src/binary_repr/`'s non-wire residue,
the final tree-wide census, the single `artifact-gate.sh all` run, and locking
the gate at hard zero.
