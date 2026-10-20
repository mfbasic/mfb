# plan-180-C: `EXIT FOR <var>` / `CONTINUE FOR <var>`

Last updated: 2026-10-19
Effort: large (3h–1d)
Depends on: plan-180-B (prerequisites are stated in plan-180-A)

`EXIT FOR i` leaves, and `CONTINUE FOR i` continues, the enclosing `FOR i` or
`FOR EACH i` loop — whichever loop owns the variable `i` — through any number of inner
loops of any kind, including inner `FOR` loops. Bare `EXIT FOR` / `CONTINUE FOR` keep their
meaning (innermost `FOR`). `DO` and `WHILE` take no variable. A variable that is not an
enclosing `FOR`/`FOR EACH` loop's variable is `UNKNOWN_FOR_IDENTIFIER`, for both `EXIT` and
`CONTINUE`.

Observable outcome: in `FOR i … FOR j … EXIT FOR i … NEXT … NEXT`, the `EXIT FOR i` ends
the outer loop and runs every cleanup the inner loop's exit would, then continues after the
outer `NEXT` — on every native target and every optimization level.

References:

- `./mfb spec language control-flow` (`10_control-flow.md`, the `EXIT`/`CONTINUE` paragraph);
  `./mfb spec language bindings-and-scope` (`05_bindings-and-scope.md`, "No shadowing of an
  in-scope local" — why a variable names exactly one loop).
- `./mfb spec package ir-section` (`08_ir-section.md`, op table + `version`);
  `./mfb spec architecture ir` (`04_ir.md`).
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/testing-gates.md`.
- `plan-180-A-os-exit.md` (prerequisites), `plan-180-B-exit-continue-unknown.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-180-B complete | `grep -c "Commit: —" planning/plan-180-B-exit-continue-unknown.md` → 0 (or the file is in `planning/completed/`) | NOT MET (2026-10-19) |
| `ExitTarget` is gone (AST `Exit` carries `LoopKind`) | `grep -rn "ExitTarget" src` → no matches | NOT MET (2026-10-19) |

If plan-180-B is not complete, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every
> command before you continue, and again before you decide to stop. If you stop, report the
> status of *all* prerequisites.

## 1. Goal

- `EXIT FOR <ident>` and `CONTINUE FOR <ident>` parse, check and run as described above.
- Programs without a named exit compile to byte-identical IR, NIR and machine code.

### Non-goals (explicit constraints)

- Bare `EXIT`/`CONTINUE <kind>` semantics, including reaching past inner loops of other
  kinds, are unchanged (owner, 2026-10-19).
- `DO`/`WHILE` loops get no names or labels (owner, 2026-10-19, option 3a).
- No change to any `.ir`/`.nir`/`.nplan` JSON or text for a bare exit: the new field is
  written only when present.
- No back-compat reader for Binary Representation 7.

## 2. Current State

- AST/HIR (after plan-180-B): `Exit { kind: LoopKind, line }`, `Continue { kind, line }`.
- Lowering: `src/ir/lower.rs` lowers `HirStatement::For` to an `IrOp::For` whose `name` is a
  **temporary** (`make_temp_local_name(context, "for_iter")` → `$for_iterN`), and binds the
  user variable inside the body; `HirStatement::ForEach` lowers to `IrOp::ForEach` whose
  `name` **is** the user variable. `$` is not allowed in user identifiers, so `$for_iterN`
  never equals a source name. Per-function lowering state is saved/restored where
  `context.current_return_type` is (`take()`/restore around `lower_function_body`, and
  around the computed-default function).
- IR/NIR ops: `ExitLoop { kind, loc }` / `ContinueLoop { kind, loc }` (`src/ir/op.rs`),
  `NirOp::ExitLoop { kind }` / `ContinueLoop { kind }` (`src/target/shared/nir/mod.rs`).
  Binary tags 11/12 = `put_loop_kind` + loc (`src/ir/binary.rs`);
  `BINARY_REPR_VERSION = 7`, decoder requires exactly that.
- Every place that **resolves** a loop exit by kind (`grep -rnE "(Ir|Nir)Op::(ExitLoop|ContinueLoop) \{ kind" src --include='*.rs'` → 22 sites in 13 files, read):
  - `src/ir/verify/ops.rs`: `loop_stack: Vec<LoopKind>` (pushed by While, For, DoUntil,
    ForEach arms); `EXIT_NO_MATCHING_LOOP` / `CONTINUE_NO_MATCHING_LOOP`.
  - `src/ir/verify/resources.rs:leaves_scanned_loop`: `back_edge_scan = (own kind, nested
    kinds)` — an exit leaves the scanned loop unless a nested loop has its kind.
  - `src/codegen/engine/control/builder_control.rs`: `loop_stack: Vec<LoopLabels>`
    (`src/codegen/engine/builder/mod.rs:LoopLabels { kind, continue_label, exit_label,
    cleanup_depth, temp_depth }`), searched innermost-first by kind; four push sites
    (While, DoUntil, For, ForEach).
  - `src/codegen/engine/analysis/last_use.rs`: `LoopFrame { kind, exit, continue_ }`,
    `loop_target(kind, exit)`, frames pushed by `fixed_point(kind, …)`.
  - `src/optimizer/opt1/plans/loops.rs:captures_loop_control` (used by `peel.rs`,
    `rotate.rs`) — whether a transform's newly introduced loop would capture an exit.
  - Serializers: `src/ir/json.rs`, `src/target/shared/nir/json.rs`, `src/ir/binary.rs`,
    `src/target/shared/plan/function_builder.rs` (`.nplan` text "exit for").
  - `src/target/shared/nir/lower.rs` (IR→NIR copy); `src/ir/tests.rs`,
    `src/ir/variant_corpus_tests.rs` (construct ops).
- Loop-duplicating optimizer passes rename declared names in the copy:
  `src/optimizer/opt1/plans/loops.rs:freshened_clone` (`declared_names` includes `For` and
  `ForEach` names; `rename_op` skips `ExitLoop`/`ContinueLoop`), verified by
  `plans/reads.rs:NameUses::census` (does not count exit ops). Users: `peel.rs`,
  `unswitch.rs` (copies a whole `For` loop), `fission.rs`. Fission/fusion only move
  `pure_statement`s, which excludes exit ops (`pure_statement`'s `_ => false`).
- Other walkers match `ExitLoop { .. }` / `ContinueLoop { .. }` and need no change
  (`grep -rnE "(Ir|Nir)Op::(ExitLoop|ContinueLoop) \{ \.\. \}" src --include='*.rs' | wc -l` → 73).
- Committed `.mfp` files that embed Binary Representation: 163
  (`git ls-files | grep -c '\.mfp$'`). Precedent for a version bump: `44288aae1` (v6→7)
  followed by `cfd159f10` "packages: rebuild committed .mfp for BINARY_REPR 7", whose
  message lists the procedure (sync script for 149 copies, five crafted security vectors via
  `tools/security-package-sources/pkg-0{2,2b,2c,3,7}/generate.py`, vendored libsnd and
  sqlite3 rebuilt with their original manifests).

### Measured populations

| What | Count | Command |
|---|---|---|
| Kind-resolving exit sites | 22 in 13 files | `grep -rnE "(Ir\|Nir)Op::(ExitLoop\|ContinueLoop) \{ kind" src --include='*.rs' \| wc -l` |
| Wildcard exit sites (unchanged) | 73 | as above with `\{ \.\. \}` |
| `.ir` goldens with an exit/continue op (must not move) | 94 | `grep -rlE '"op": "(exitLoop\|continueLoop)"' tests --include='*.ir' \| wc -l` |
| Committed `.mfp` | 163 | `git ls-files \| grep -c '\.mfp$'` |

### Verified properties

- **A variable names one loop.** `05_bindings-and-scope.md` forbids re-declaring an
  in-scope name, so `FOR i` cannot enclose another `FOR i`/`FOR EACH i`; the innermost
  match is the only match in source.
- **Resolved targets never collide after optimization.** A clone made by `freshened_clone`
  renames its loop names; with exit targets renamed too (Phase 2), every target in the copy
  names the copy's loop. Uncloned siblings (`unswitch`'s then-copy) keep the original name,
  but are siblings, not nested, so innermost-first lookup is still exact.
- **No transform introduces a `For`/`ForEach`.** `rotate` introduces `DoUntil`, `peel`
  copies a `While` body out of its loop, `unswitch` duplicates an existing loop; a named
  target only ever matches a `For`/`ForEach` by name, so `captures_loop_control` is `false`
  for it.

## 3. Design Overview

- **Source.** `Statement::Exit { kind, target: Option<String>, line }`, same on `Continue`
  and the HIR. The parser accepts an identifier after `EXIT FOR` / `CONTINUE FOR` only;
  after `DO`/`WHILE` an identifier hits the existing "Expected end of statement after
  EXIT/CONTINUE." error. AST JSON writes `"var": "<name>"` only when present.
- **Resolution at lowering.** `LowerContext` gains `loop_targets: Vec<(String, String)>`
  — (source variable, IR loop name) — pushed around `For` (`(name, $for_iterN)`) and
  `ForEach` (`(name, name)`) body lowering, taken/restored with `current_return_type` so it
  never crosses a function or lambda boundary. A named exit lowers to
  `ExitLoop { kind: For, target: Some(ir_name) }` using the innermost match; when no entry
  matches, `target: Some(source_name)` — a name no enclosing loop carries.
- **Check.** `ir::verify` `loop_stack` becomes `Vec<(LoopKind, Option<String>)>` (For and
  ForEach push their op `name`). A named exit with no stack entry of that name emits
  `UNKNOWN_FOR_IDENTIFIER` (`2-203-0148`): "`<x>` is not the variable of an enclosing FOR
  loop." — the unresolved target still holds the source name, so the message names it.
- **IR/NIR/binary.** `target: Option<String>` on both ops in IR and NIR; JSON writes
  `"target"` only when `Some`; binary tags 11/12 append `put_opt_str`;
  `BINARY_REPR_VERSION` 7 → 8.
- **Every resolver** matches `target: Some(t)` by loop name (innermost entry whose name is
  `t`) and `None` by kind exactly as today: verify `loop_stack`, `leaves_scanned_loop`
  (scan state carries the scanned loop's name and the nested loops' names), codegen
  `LoopLabels` (new `name: Option<String>`, `Some` at the For/ForEach pushes),
  `last_use` `LoopFrame` (same field, passed through `fixed_point`), `captures_loop_control`
  (named → `false`), `.nplan` text (`exit for $for_iter3` when named).
- **Optimizer renames.** `NameUses::visit_op` counts an exit's target; `rename_op` renames
  it. A missed rename is then caught by `freshened_clone`'s own census check (it refuses
  the clone) instead of producing a dangling target.

Correctness risk concentrates in codegen's cleanup on a named exit (it must use the
*target* loop's `cleanup_depth`/`temp_depth`, which `emit_cleanup_branch_to_depth` already
takes from the found `LoopLabels`) and in `last_use` liveness (the exit's live set is the
target frame's `exit`, not the innermost frame's). Both are covered by Phase 4 runtime tests
with a resource-owning inner loop. Gate class: programs without a named exit must be
**byte-identical** in IR/NIR/`.ncode`; the only expected diffs are the Binary Representation
version byte in `.br` dumps and rebuilt `.mfp` files (and package-id-derived symbols in
fixtures that import a rebuilt `.mfp`, as in `cfd159f10`).

Rejected alternatives:
- *Depth counts* (`skip: n` same-kind loops) — any transform that adds, removes or wraps a
  same-kind loop silently retargets the exit; names are stable under every existing pass.
- *Matching the source variable at codegen* — counted `FOR` loops are keyed by
  `$for_iterN`, not the variable; resolving once at lowering keeps every later layer
  name-only.
- *Labels on all loops* — owner chose `FOR`-only (3a).
- *New op tags instead of a version bump* — the format rule is one version per format; the
  `.mfp` rebuild has a script and a precedent.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work;
> `- [~]` for partial; strike through moot tasks with evidence. **An unticked box means NOT
> DONE.**

### Phase 1 — carry the target through IR, NIR and the package format

Representation only; nothing creates a target yet, so all output is byte-identical except
the version.

- [x] `src/ir/op.rs`, `src/target/shared/nir/mod.rs`: `target: Option<String>` on
      `ExitLoop`/`ContinueLoop`; every constructor passes `None`
      (`src/ir/lower.rs`, `src/ir/tests.rs`, `src/ir/variant_corpus_tests.rs`, optimizer
      tests in `peel.rs`/`rotate.rs`/`unswitch.rs`/`uce.rs`).
- [x] `src/target/shared/nir/lower.rs`: copy `target`.
- [x] `src/ir/json.rs`, `src/target/shared/nir/json.rs`: write `"target"` only when `Some`.
- [x] `src/ir/binary.rs`: tags 11/12 encode/decode `put_opt_str`/`opt_string` after the
      kind; `BINARY_REPR_VERSION = 8`; round-trip unit test with `Some("$for_iter1")`.
- [x] `src/docs/spec/package/08_ir-section.md`: version **8**, what v8 adds, why v7 is
      refused; `src/docs/spec/architecture/04_ir.md`: `ExitLoop`/`ContinueLoop` carry an
      optional target.

Acceptance: every IR/NIR golden unchanged; the binary round-trips a target.
  Check: `cargo test --bin mfb ir::binary` → pass; `bash scripts/artifact-gate.sh
  target/release/mfb collections` → only `.br` version diffs (est. 2 min).
Commit: —

### Phase 2 — resolve by name everywhere a loop exit is resolved

- [ ] `src/ir/verify/ops.rs`: `loop_stack: Vec<(LoopKind, Option<String>)>`; named lookup;
      `UNKNOWN_FOR_IDENTIFIER` (add `2-203-0148` to `src/rules/table.rs` and
      `01_rule-codes.md`).
- [ ] `src/ir/verify/resources.rs`: `back_edge_scan` carries own name + nested names;
      `leaves_scanned_loop` resolves named exits/continues by name.
- [ ] `src/codegen/engine/builder/mod.rs:LoopLabels` + the four pushes in
      `builder_control.rs`; the `ExitLoop`/`ContinueLoop` arms look up by name when
      `target` is `Some`.
- [ ] `src/codegen/engine/analysis/last_use.rs`: `LoopFrame.name`, `fixed_point` takes it,
      `loop_target(kind, target, exit)`.
- [ ] `src/optimizer/opt1/plans/loops.rs`: `captures_loop_control` returns `false` for a
      named exit; `rename_op` renames `target`. `plans/reads.rs:NameUses::visit_op` bumps it.
- [ ] `src/target/shared/plan/function_builder.rs`: `exit for <target>` /
      `continue for <target>` when named.
- [ ] Unit tests: verify (named hit, named miss → `UNKNOWN_FOR_IDENTIFIER`, named through an
      inner `FOR`); `captures_loop_control` named → false; `freshened_clone` renames a target
      inside a cloned `For`; `leaves_scanned_loop` named exit past a nested `For`.

Acceptance: hand-built NIR/IR with named targets verifies, plans and survives cloning.
  Check: `cargo test --bin mfb ir::verify` and `cargo test --bin mfb optimizer::opt1::plans` → pass (est. 2 min).
Commit: —

### Phase 3 — source syntax and lowering

- [ ] `src/ast/types.rs`, `src/hir/mod.rs`, `src/monomorph/lower.rs`: `target` on
      `Exit`/`Continue` (monomorph copies it unchanged — it does not rename locals).
- [ ] `src/ast/stmt.rs`: optional identifier after `EXIT FOR` / `CONTINUE FOR`.
      `src/ast/serialize.rs`: `"var"` when present.
- [ ] `src/ir/lower.rs`: `LowerContext.loop_targets` (pushed in the `For`/`ForEach` arms,
      taken/restored at every `current_return_type` save site); named exits resolve per §3.
- [ ] `src/fmt.rs`: unit test that `EXIT FOR i` / `CONTINUE FOR i` format unchanged inside
      nested loops.
- [ ] Syntax fixture `tests/syntax/control-flow/exit-for-unknown-identifier-invalid`:
      `EXIT FOR k` and `CONTINUE FOR k` where `k` is a plain local, a `DO`-loop's counter,
      and an outer function's loop variable seen from a lambda → each
      `UNKNOWN_FOR_IDENTIFIER`; `EXIT DO x` → the existing end-of-statement parse error.

Acceptance: named exits parse, resolve and report.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/acc-180c
  'exit-for-unknown-identifier-invalid'` → pass (est. 1 min).
Commit: —

### Phase 4 — runtime behaviour (largest blast radius)

- [ ] `tests/rt-behavior/control-flow/exit-for-named-rt/` with `.run` golden: `EXIT FOR i`
      from inside `FOR j`, from inside `WHILE` inside `FOR j`, from a `FOR EACH`, and
      `CONTINUE FOR i` from inside `FOR j` (inner loop restarts, outer increments once);
      prints the visited `(i, j)` pairs. Plus a case whose inner loop owns a `RES` and a
      `List` temp, exited by `EXIT FOR i`, run under the leak checker the rt-behavior
      harness uses (`tests/runtime/rt_scope_drop_leaks.rs` pattern) → no leak.
- [ ] Run it at every optimization level and on the remote targets per
      `.ai/remote_systems.md` (native aarch64 box 2223 for Linux; x86_64/riscv64 cross
      builds' `.run` via the standard harness).
- [ ] `src/docs/spec/language/10_control-flow.md`: the named form, its resolution rule,
      `UNKNOWN_FOR_IDENTIFIER`; `mfb man flow` guide topic gets one example
      (`src/docs/man/flow/`).
- [ ] Rebuild the committed `.mfp` files per the `cfd159f10` procedure
      (`bash scripts/sync-package-mfp.sh target/release/mfb`, then the five crafted security
      vectors and the vendored libsnd/sqlite3 copies with their original manifests), in its
      own commit.

Acceptance: the named-exit program prints the expected pairs and leaks nothing on every
target and level.
  Check: `bash scripts/test-accept.sh target/release/mfb /tmp/acc-180c 'exit-for-named-rt'`
  → pass; the leak test binary → pass (est. 5 min host; remote per box).
Commit: —

## Validation Plan

- Tests: Phase 2 unit tests; Phase 3 syntax fixture; Phase 4 rt fixture + leak check.
- Coverage check: `grep -rn "EXIT FOR [a-z]\|CONTINUE FOR [a-z]" tests` hits a `FOR`
  target, a `FOR EACH` target, an inner `FOR`, an inner `WHILE`, a resource-owning inner
  loop and every miss case.
- Runtime proof: the Phase 4 program on macos-aarch64, linux-aarch64 (box 2223),
  linux-x86_64, linux-riscv64, windows-x86_64.
- Doc sync: `10_control-flow.md`, `04_ir.md`, `08_ir-section.md`, `01_rule-codes.md`,
  `src/docs/man/flow/`.
- Final gate (once): `bash scripts/test-fast.sh`; the optimizer sweep
  `cargo test --bin mfb builtins_codegen::optimizer::` (named exits meet every loop
  transform); `bash scripts/artifact-gate.sh target/release/mfb all` (15–20 min — the only
  proof that no program without a named exit moved) and a full `bash
  scripts/test-accept.sh target/release/mfb /tmp/acc-180c-full`. Expected diffs: `.br`
  version bytes, rebuilt `.mfp`, package-id-derived symbols in fixtures importing them.

## Decisions

- `EXIT FOR <var>` / `CONTINUE FOR <var>` target the `FOR` or `FOR EACH` loop that owns
  `<var>`; `DO`/`WHILE` take no variable — owner, 2026-10-19 (option 3a).
- Bare `EXIT`/`CONTINUE <kind>` stay, innermost-of-kind, still reaching past inner loops of
  other kinds — owner, 2026-10-19.
- An unknown loop variable is `UNKNOWN_FOR_IDENTIFIER` for both `EXIT` and `CONTINUE` —
  owner, 2026-10-19.
- Targets are resolved once in IR lowering to the IR loop's name — author (counted `FOR`
  loops are keyed by a temporary).
- Binary Representation bumps to 8 — author (one version per format, precedent `44288aae1`).

## Corrections

- **Phase 1 `.br` prediction was wrong (measured).** The plan expected `.br` version-byte diffs
  from `artifact-gate.sh collections`. `git ls-files | grep -c '\.br$'` → 0 and
  `grep -n '\.br\b' scripts/artifact-gate.sh` → no match, so no `.br` file is in any golden
  and the gate has nothing to diff. Phase 1 result: `bash scripts/artifact-gate.sh
  target/release/mfb collections` → `1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`.
  The version byte is still exercised by `cargo test --bin mfb ir::binary` (4 passed) and
  `named_loop_exit_round_trips_its_target` (1 passed).
- **Phase 1 expected package breakage (measured, owned by Phase 4).** `git ls-files | grep -c
  '\.mfp$'` → 163 committed packages still embed `BINARY_REPR` 7, which the v8 decoder refuses.
  Phase 1 does not import them, so its acceptance is unaffected. Phase 4's rebuild restores
  them; the plan's `44288aae1` → `cfd159f10` precedent accepts the same intermediate state.
- **Phase 1 site count (measured).** The new `target` field needed a value at every constructor
  and pattern site that `cargo check --workspace --all-targets` reported (constructors get
  `target: None`, patterns get `target: _`). No behaviour change: every op is `None` until
  Phase 2 creates a target.

## Summary

Risk is in codegen cleanup depth and liveness for an exit that crosses inner loops; both
reuse the found loop's recorded depths and are pinned by the Phase 4 leak test. Programs
without named exits must not change.
