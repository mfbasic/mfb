# plan-188-D: Identical code folding

Last updated: 2026-10-26
Effort: medium (1h–2h)
Depends on: plan-188-C (letter order only — no shared code)

Monomorphization, package helpers and platform variants leave functions whose
finished machine code is identical: 550 removable functions, 51,921 instructions in
the macos-aarch64 corpus. This sub-plan lands the catalog row "Identical code
folding" (Level 1 since the 2026-10-24 review): after every function of a module
is finalized, functions with identical code, frame and relocations are folded into
one, and every reference to a removed symbol is redirected to the survivor. The
outcome: at `-O1` without `--debug`, no two functions in a native code plan have
identical normalized code; with `--debug` or at `-O0` nothing is folded; every
program prints the same output.

References:

- `src/codegen/engine/builder/lower_module.rs:lower_module_for_platform` (where the
  plan's `functions` are assembled, after `bind_deferred_relocation_libraries`).
- `src/codegen/engine/types/types.rs:NativeCodePlan`, `CodeFunction`,
  `CodeRelocation`, `CodeDataObject` (no relocation field).
- `src/arch/encode_plan.rs:encode_plan` (symbols are laid out from
  `plan.functions`).
- `src/target/shared/nir/mod.rs` (`NirModule::debug`, the `--debug` switch).
- `planning/optimizations.md` "Review of 2026-10-24 — the default-level exception"
  (the ICF bullet: exact byte equality, relocations as identity, `--debug` folds
  nothing).
- `mfb spec language types` §4.11 (functions are not comparable),
  `mfb spec tooling debugger` (symbols).

## Prerequisites

The family's prerequisites are in plan-188-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-188-C complete | `ls planning/plan-188-C-* → no matches` (archived) | NOT MET (2026-10-26) |

## 1. Goal

- Row "Identical code folding" (level 1, stage `codegen`),
  `src/optimizer/icf.rs:fold(functions, entry_symbol) -> u64`, called from
  `lower_module_for_platform` before the plan is built, gated
  `level_enabled(1) && !module.debug.enabled`.
- After it runs, no two `CodeFunction`s of the plan share a fold key (§4.1).

### Non-goals (explicit constraints)

- No merging of functions that differ in anything but local label spelling and
  self-reference (no "similar" functions, no parameterized merging — that is the
  Level-4 "Function merging (IR-level)" row).
- No change to data objects (they hold no relocations, so no data references a
  function symbol).
- `--debug` builds and `-O0` fold nothing; the entry symbol `_main` is never
  removed.
- No object-writer change: folded symbols are removed and redirected, not aliased.

## 2. Current State

Every `CodeFunction` reaches `encode_plan`, which lays out one symbol per
function. Nothing compares functions.

Measured (`python3 -I /tmp/p188/icf.py <KEEP dumps>`: per project, key = the
instruction list with function-local labels renamed in order of first appearance
and the function's own symbol replaced by `SELF`; base release `mfb` of
`d58b5397a`):

| Target | groups | removable functions | removable instructions | of all instructions |
|---|---|---|---|---|
| macos-aarch64 | 282 | 550 | 51,921 | 0.19 % of 27,057,789 |
| linux-x86_64 | 240 | 466 | 57,444 | 0.19 % of 30,519,671 |
| linux-riscv64 | 61 | 75 | 38,770 | 0.86 % of 4,506,516 |

The largest groups (macos-aarch64) are platform and package helpers duplicated
across variants: `_mfb_ifn_canvas_5FpolyCurveRecord` (322 instructions, twice, in
11 projects), `_mfb_macapp_canvas_mouseDown` / `rightMouseDown` /
`otherMouseDown` (109 each), `_mfb_ifn_canvas_5FmetalRenderable` (54, twice, in
12 projects), and many 3–8-instruction stubs. The probe does not compare frames or
relocation records separately; the row does (§4.1), so its count can only be
lower.

## 3. Design Overview

A module-level pass over the finished `Vec<CodeFunction>`. Grouping by an exact
key, survivor = earliest in plan order (or `_main`), removal plus redirect of every
reference, repeated until a round folds nothing (a redirect can make two callers
identical). Behavior-preserving because the survivor executes the same
instructions with the same relocated operands as each removed copy, and MFBASIC
cannot compare function values (§4.11) or print a stack trace.

**Correctness risk** is a reference the redirect misses: a symbol named somewhere
other than an instruction field or a `CodeRelocation`. Data objects hold none
(`CodeDataObject` has no relocation field); hand-written emitters reach functions
only through `bl`/`adrp` + relocation. A miss fails the link loudly
("internal relocation target … is not defined"), never silently.

**Byte-identity is not the gate.** Fixtures with duplicate functions move on
every target (canvas/app and package-heavy ones); classified by the row's `-v`
count and runtime equality.

Rejected alternatives:

- **Symbol aliases in the object writers** (keep both names at one offset). Needs
  a change in each container writer and leaves two names per address in debugger
  and profiler output; redirection needs neither.
- **Folding at `--debug`.** The review's ICF bullet: a `--debug` build keeps every
  symbol.

## 4. Detailed Design

### 4.1 Fold key

For a `CodeFunction f`: the sequence of its instructions, each rendered as the op
mnemonic plus its fields in order, where a field naming one of `f`'s own labels is
replaced by its first-appearance index and a field naming `f.symbol` by `SELF`;
then `f.frame.stack_size`, `f.frame.callee_saved`, `f.stack_slots` (name, type,
offset), and `f.relocations` (with `from` dropped and `to == f.symbol` as `SELF`;
`kind`, `binding`, `library` kept), in order. `params`/`returns` are excluded
(metadata only; two functions with equal code are interchangeable at every call
site). Groups are kept in a `BTreeMap` keyed by the rendered key so the result is
deterministic.

### 4.2 Fold

1. Build groups. For each group of two or more: survivor = `_main` if present,
   else the earliest in plan order; record `removed → survivor`.
2. Delete removed functions. In every remaining function, replace each instruction
   field and each `CodeRelocation.to` equal to a removed symbol by its survivor.
3. Repeat from 1 until a round removes nothing. Count removed functions in
   `stats::IDENTICAL_FUNCTIONS_FOLDED`.

Placement: in `lower_module_for_platform`, after
`bind_deferred_relocation_libraries` and before `NativeCodePlan { … }` /
`plan.validate()`, so the `-ncode` dump and every later stage see the folded plan.

## Phases

> **NOTE — keep the checkboxes current** (template rules).

### Phase 1 — the row

- [ ] `src/optimizer/icf.rs` (§4), `pub(crate) mod icf;` in `src/optimizer/mod.rs`;
      the call in `lower_module_for_platform`; catalog row "Identical code folding"
      (level 1, stage `codegen`); stats counter.
- [ ] Unit tests (`src/optimizer/icf/tests.rs`): two identical functions fold and a
      caller's `bl` is redirected; a function-value `adrp`/`add_pageoff` of the
      removed symbol is redirected; differing only in label spelling folds; a
      self-recursive pair folds; differing frame, stack slots, or one relocation's
      `library` does not; a chain that becomes identical after a redirect folds in
      round two; `_main` survives; `--debug` and `-O0` fold nothing; output order of
      survivors is stable.
- [ ] `tests/runtime/rt_codegen_forms.rs::identical_code_folding`: a fixture
      `tests/rt-behavior/functions/identical-bodies` with two user functions of
      identical multi-statement bodies (so leaf inlining, which takes only a
      single `RETURN <expression>`, leaves them) called directly and through a
      `FUNC` value prints the same
      at `-O0` and `-O1`, and its `-O1` dump has one of the two symbols.
- [ ] Golden re-baseline, every target (plan-188-A Phase 1's procedure;
      classification: the `.ncode` of a moved fixture differs only by removed
      functions and redirected symbol names, checked on one fixture per target
      with `census.py --diff`).
- [ ] Runtime: `bash scripts/test-accept.sh target/release/mfb /tmp/188d-accept`;
      `linux-runtime-proof.sh` on 2223 (linux-aarch64) and 2227 (linux-x86_64);
      the new fixture on 2229 (linux-riscv64) and on `test@192.168.1.206`
      (windows-x86_64); `scripts/test-macapp.sh` (the folded `_mfb_macapp_*`
      mouse handlers are app-mode code no console fixture runs) and
      `scripts/test-appimage.sh --libc both` (the GTK twins).

Acceptance: duplicates are gone at `-O1` and every target's output is unchanged.
  Check: `cargo test --bin mfb icf` → pass (est. 2 min);
  `cargo test --release --test rt_codegen_forms identical_code_folding` → pass
  (est. 2 min); `mfb build -O1 -v examples/canvas/rain` reports a non-zero
  "Identical code folding" count (est. 1 min); the app-mode scripts → pass
  (est. 20 min — the only execution of the folded app handlers).
Commit: —

### Phase 2 — docs

- [ ] `mfb spec tooling debugger`: at `-O1`+ without `--debug`, functions with
      identical code share one symbol; `--debug` keeps every symbol.
      `mfb spec architecture native`: the row.
- [ ] `planning/optimizations.md`: row "Identical code folding" `Imp` → `Y` with
      the measured count.

Acceptance: `target/release/mfb man optimizations | grep -c 'Identical code folding'` → 1 (est. 1 min).
Commit: —

## Validation Plan

- Tests: icf units; `rt_codegen_forms::identical_code_folding`; the app-mode
  scripts.
- Coverage check: the row's `-v` count on `examples/canvas/rain` and
  `examples/brogue`.
- Runtime proof: Phase 1's runs.
- Doc sync: Phase 2.
- Final gate (once): `bash scripts/test-fast.sh`; artifact gate `0 diff(s)` after
  classified regeneration; `test-accept.sh` full; `cargo test --workspace
  --no-fail-fast` → `EXIT=0` (est. 60–90 min).

## Decisions

- Level 1, `--debug` folds nothing — owner, 2026-10-24 review
  (`planning/optimizations.md`, "Identical code folding 4 → 1").
- Redirect instead of alias — author's call (§3).

## Corrections

(Filled in during execution.)

## Summary

Small, size-only (0.19 % of instructions on AArch64 and x86-64, 0.86 % on
riscv64), low risk: a missed reference fails the link instead of miscompiling.
