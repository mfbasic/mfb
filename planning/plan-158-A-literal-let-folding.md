# plan-158-A: Literal `LET` folding at the default level

Last updated: 2026-09-26
Overall Effort: huge (>3d) — plan-158 is bug-740's three mechanisms, one letter each
Effort: medium (1h–2h)
Depends on: bug-744 (fixed first; this row reuses its guard)

bug-740 mechanism (a). A module-level `LET` whose initializer is a number or
Boolean literal is read from the globals block on every use (`add x8, x19,
#0xf38; ldr x9, [x8]` for `DROWS` in brogue's `gi`). This plan adds a Level-1
Opt1 row that replaces such a read with the literal, so the value becomes an
immediate the Level-1 folding rows can see through (`DCOLS * DROWS - 1` becomes
one constant). The single behavioral outcome: at the default `-O1`, a read of a
scalar-literal `LET` in code that runs after initialization compiles to the
literal, and every program computes and raises exactly what it did before.

References:

- `bugs/bug-740-brogue-dig-leaf-call-and-spill-overhead.md` — the profile and the
  three mechanisms. Mechanism (a) is this plan.
- `bugs/completed/bug-744-o2-constify-folds-reads-that-run-during-initialization.md`
  — the init-order hazard this row must not repeat, and the guard it reuses.
- `src/optimizer/opt1/globals.rs` — the Level-2 constification row this mirrors.
- `planning/optimizations.md` — the pass catalog; `src/optimizer/catalog.rs` —
  the rows `mfb man optimizations` renders.
- `mfb spec tooling cli-reference` (`src/docs/spec/tooling/07_cli-reference.md`)
  — states what `-O1` runs.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| bug-744 fixed (the `initializer_reachable` guard exists) | `grep -n "fn initializer_reachable" src/optimizer/opt1/plans/globals.rs` → one hit | MET (2026-09-26) |

Everything below is written against a tree where that guard exists.

## 1. Goal

- At `-O1`, every read of a module-level `LET` whose initializer is a `Const` of
  the binding's own type, with that type one of `Integer`, `Byte`, `Boolean`,
  `Float`, `Fixed` or `Money`, becomes that `Const` in every function the global
  initializer cannot reach. `mfb build -v examples/brogue` reports the row firing.
- Every program's output, exit code and runtime errors (code, file, line,
  column) are unchanged at every level.

### Non-goals (explicit constraints)

- **Never a `MUT`.** The row keys on `!mutable` (the source `LET`) *and* on "no
  store outside the initializer" (`GlobalUse::never_written`).
- **Never a computed initializer.** A call, an arithmetic expression, a negation
  node, a string, a collection, a record, a union or an enum member is not a
  `Const` of a word scalar and is never folded: evaluating it at each read could
  trap, allocate, or run code a different number of times.
- **Never a read that can run during initialization** (bug-744): the global
  initializer SUB and everything it reaches keep their reads.
- No change to the `.mfp` package format, the globals block layout, or any
  global's storage: the global is still stored and still readable by anything
  this row leaves alone.
- No change to `-O0` codegen.

## 2. Current State

- NIR globals carry `mutable` from the source (`LET` → `false`) and the
  initializer as `value` (`src/target/shared/nir/mod.rs:NirGlobal`). In brogue,
  576 of 595 globals are `public`, immutable and `const`-initialized (`mfb build
  -nir examples/brogue`, then a JSON count of `globals` by `(visibility, mutable,
  value.kind)`).
- Function bodies read them as `NirValue::Global { name, type_ }`; the codegen
  loads the slot from the globals block every time.
- The only row that folds a global is Level-2 constification
  (`src/optimizer/opt1/globals.rs:constify`): private globals only, so brogue's
  `PUBLIC LET DROWS` is a memory load even at `-O3` (verified: `-O3 -ncode` of
  `gi` still loads `x19+3896`).
- bug-744 added `plans::globals::initializer_reachable` and the shared
  substitution `substitute_outside_initialization`.

### Measured populations

| What | Count | Command |
|---|---|---|
| brogue reads the row folds | 303 | `mfb build -v examples/brogue` → `Literal \`LET\` folding: 303` |
| brogue ratio, prototype of this row alone (lock NOT held, median of 5) | 17.5x vs 17.6x base | `/tmp/b740-measure.py 5 oracle base a_only` (session probe; binaries built by the base and prototype compilers) |
| `.nir` goldens that contain a global read | 2 of 23 | `find tests -name '*.nir' \| xargs grep -l '"kind": "global"' \| wc -l` |

The measured share of this row on its own is **zero within noise**. It lands
first because it is cheap and because plan-158-C's inlined bodies need it: an
inlined `gi` with `DROWS` still a load keeps a memory access in the hottest loop.

### Verified properties

- A `LET` is never assigned: the resolver rejects it, so `StoreGlobal` of a `LET`
  appears only in the initializer SUB. Checked twice in code anyway
  (`!mutable` and `never_written`).
- The folded `Const` is the same NIR node the initializer store lowers, so the
  value the read produces is bit-identical to the stored one. Verified by the
  runtime cases in Phase 1 (printing each scalar type through a function).
- The initializer SUB is the only code that runs before `main`
  (`mfb spec memory program-startup`); worker threads re-run the same SUB before
  their entry, so the same reachability set covers them.

## 3. Design Overview

One new Opt1 row, `globals::fold_literal_lets`, gated `level_enabled(1)`, run
first in `optimize_nir` (before the local rewrites, so they fold through the
literal). It builds the `{name → Const}` map from the three checks above and
hands it to bug-744's `substitute_outside_initialization`. A catalog row
("Literal `LET` folding", Level 1, stage NIR) and a stats counter make it visible
under `-v` and in `mfb man optimizations`.

Correctness risk concentrates in the initializer-order guard — already fixed and
tested by bug-744 — and in the type check (a `Const` whose type differs from the
global's is a coercion the store performs; folding it would skip the coercion).

**Gate class: not byte-identical.** Every fixture that reads a scalar-literal
`LET` (including the builtin packages' own injected `LET` tables) is expected to
change `.ncode`/`.ncodesum` on every target, and `.nir` where it carries one.
Expected diffs are a load becoming an immediate; any other diff is a bug to
localize with one fixture's `.ncode`.

Rejected alternatives:

- *Lowering the Level-2 constification row to Level 1.* It requires privacy
  (a `MUT` can be written by an importer) and would still miss brogue's
  `PUBLIC LET`s. A `LET` needs no privacy proof.
- *Emitting literal `LET`s as read-only data.* Still a load, and it would change
  what an early (pre-initialization) read observes.

## Phases

### Phase 1 — the row, with its runtime proofs

- [x] `src/optimizer/opt1/globals.rs`: `fold_literal_lets` + `is_word_scalar`,
      gated `level_enabled(1)`, reusing `substitute_outside_initialization`.
- [x] `src/optimizer/stats.rs`: `LETS_FOLDED` + `count_lets_folded`;
      `src/optimizer/catalog.rs`: the "Literal `LET` folding" row, first in the
      list (it runs first).
- [x] `src/optimizer/opt1/mod.rs`: run it before `local_rewrites::apply`.
- [x] Unit tests in `globals.rs`: folds a public `LET`; leaves a `MUT`, a
      mismatched-type `Const`, a non-`Const` initializer and a `String` alone;
      keeps initializer-reachable reads; off at `-O0`. `cargo test --bin mfb
      globals::tests` → `16 passed`.
- [x] Runtime test `tests/runtime/rt_literal_let_folding.rs`: one program per
      scalar type (`Integer`, `Byte`, `Boolean`, `Float`, `Fixed`, `Money`)
      printing the `LET` through a function, identical at `-O0` and `-O1`; an
      overflow through a folded `LET` (`BIG + 1`) raises `ErrOverflow` with the
      same code, file, line and column at `-O0` and `-O1`; and the `.ncode` at
      `-O1` has no globals-block load in the reading function.
      `MFB_TEST_EXE=target/debug/mfb cargo test --test rt_literal_let_folding
      --test rt_global_init_order_fold` → 3 + 3 passed; with the main-tip
      compiler `the_read_becomes_an_immediate` FAILS (RED at base).
      (Correction 1: the uncaught-error stderr carries no location, so the
      overflow case catches the error and prints `e.source`.)

Acceptance: `cargo test --bin mfb optimizer::opt1::globals` and
`cargo test --test rt_literal_let_folding --test rt_global_init_order_fold`
pass.
  Check: those two commands → `test result: ok` (est. 10 min, the bin test build).
Commit: —

### Phase 2 — goldens, docs, budget

- [ ] Regenerate the native goldens the row moves
      (`scripts/regen-native-goldens.sh target/release/mfb`), after classifying
      the diff against a detached main-tip baseline gate; inspect one moved
      fixture's `.ncode` to confirm the only change is load → immediate.
- [ ] `planning/optimizations.md`: the new row; `src/docs/spec/tooling/07_cli-reference.md`:
      the `-O` line lists what `-O1` runs and that levels `0`–`3` exist.
- [ ] Measure `tools/brogue-perf/bench.sh --runs 7` under the heavy lock; set
      `BUDGET_RATIO` to the new floor with the same headroom rule the file
      documents.

Acceptance: `scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`
after regeneration; `examples/brogue/check/check-terrain.sh 8 1` → 320 pairs
match; `mfb man optimizations` lists the row.
  Check: the artifact gate (est. 20 min; it is the only thing that sees every
  target's `.ncodesum`).
Commit: —

## Validation Plan

- Tests: Phase 1's unit and runtime tests; bug-744's
  `rt_global_init_order_fold` (its `LET` case is this row's init-order guard).
- Coverage check: `mfb build -v examples/brogue` shows the row firing (303).
- Runtime proof: `examples/brogue/check/check-terrain.sh 8 1` byte-identical.
- Doc sync: the catalog row renders into `mfb man optimizations`; the spec CLI
  reference and `planning/optimizations.md` updated.
- Final gate (once, under the heavy lock): `cargo test --release --no-fail-fast`
  and `scripts/artifact-gate.sh target/release/mfb all`.
- Cross-target (every native backend): the artifact gate builds every target's
  `.ncodesum`; for runtime, brogue is cross-built for `linux-aarch64` (box 2223),
  `linux-riscv64` (2229), `linux-x86_64` (2227, emulated) and `windows-x86_64`
  (the real Windows box) and each box's `dig` output for seeds 1–2 × depths
  1–40 is compared with the oracle's (generated on the Mac; the dump is
  platform-independent text). Whatever a box could not run is stated in
  Corrections.

## Open Decisions

- Fold `String` literals too? — **No** (recommended). A `String` global read is a
  pointer to a block; a literal is static data with different ownership
  handling. No measured need.

## Corrections

1. Phase 1 planned to compare the overflow's file/line/column from the
   uncaught-error stderr; that report carries only the code and message
   (`Error: 7-705-0010 …`). The test catches the error with `TRAP(e)` and prints
   `e.code`, `e.message` and `e.source.{filename,line,char}` instead, which is
   the same information and is what a program can observe.

## Summary

Small and nearly free: the risk was the initialization-order hazard, which
bug-744 found and fixed in the older row first. On its own it moves brogue by
nothing measurable; it exists so that plan-158-C's inlined helpers see constants.
