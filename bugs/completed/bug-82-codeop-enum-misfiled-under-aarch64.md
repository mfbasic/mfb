<!-- Bug document: organizational/tech-debt, not a correctness bug. -->

# bug-82: neutral cross-arch MIR opcode enum `CodeOp` is misfiled under `src/arch/aarch64/ops.rs`

Last updated: 2026-07-10
Effort: small (<1h)

`CodeOp` — the target-neutral MIR opcode vocabulary consumed by **every** backend
(aarch64, x86_64, riscv64) and by the shared code layer (peephole, regalloc,
fma_fusion, abi) — is defined in `src/arch/aarch64/ops.rs`. It lives there only
for historical reasons: aarch64 was the original backend, and the enum was never
relocated when x86_64 and riscv64 were added and the plan-00 series neutralized
the op set into a shared MIR vocabulary. As a result the x86 and riscv backends,
and shared/arch-agnostic modules, all write `use crate::arch::aarch64::ops::CodeOp`
— importing a *neutral* type through an *aarch64* path. The single correct end
state a fix produces: `CodeOp` lives at a neutral location (e.g.
`src/arch/ops.rs`, module path `crate::arch::ops::CodeOp`), every consumer imports
it from there, and no aarch64 path is on the import of a cross-arch type. This is
purely an organizational defect — there is no runtime symptom and no change to
generated code; the risk is that the misfiling misleads readers into thinking the
op set is aarch64-specific (the spec even documents the aarch64 import path as if
it were correct).

References:

- `src/docs/spec/architecture/15_x86_64-instruction-set.md:18` — currently
  documents `crate::arch::aarch64::ops::CodeOp` as the enum "imported ...
  throughout the x86 backend", cementing the misfiling in the spec.
- plan-00 MIR-neutralization series (00-A … 00-H): the work that turned the
  aarch64 op set into a shared neutral MIR vocabulary but left the enum's home
  under `aarch64/`.
- Found during a code-organization review of `src/arch/` (this session).

## Failing Reproduction

Not a runtime bug — the "failure" is structural and is observable by grep: a
neutral type is imported through an arch-specific path by unrelated backends.

```
# Every backend and the shared layer reach a "neutral" enum through aarch64:
grep -rl "aarch64::ops::CodeOp" src/arch/x86_64 src/arch/riscv64 src/target/shared
```

- Observed: `src/arch/aarch64/mod.rs:7` declares `pub(crate) mod ops;`, and
  16 non-defining sites across x86_64, riscv64, and the shared code layer import
  `crate::arch::aarch64::ops::CodeOp`.
- Expected: `CodeOp` is declared at a neutral module (`crate::arch::ops`), and
  no non-aarch64 code names an `aarch64::` path to obtain it.

Contrast: `src/arch/mod.rs` is the natural neutral home — sibling to the three
per-arch subdirs — but currently declares no `ops` module.

## Root Cause

`src/arch/aarch64/ops.rs` defines `pub(crate) enum CodeOp` (714 lines: the enum
+ its `impl`) and `src/arch/aarch64/mod.rs:7` exposes it as `mod ops`. The enum
is target-neutral — its own doc comments describe "Neutral MIR semantics"
(`ops.rs:202`) and x86-only / plan-00-G/H behaviors — yet its module home was
never lifted out of `aarch64/` when the backend set grew. Because Rust module
paths encode physical location, every out-of-arch consumer must spell the
aarch64 path. Nothing about the enum's *content* is aarch64-specific; only its
*location* is.

## Goal

- `CodeOp` (and its `impl`) are defined at a neutral module path
  (recommend `crate::arch::ops`).
- All 19 referencing files (including the spec markdown) import/name it from the
  neutral path; zero non-aarch64 references to `aarch64::ops` remain.
- `cargo build` / `cargo test` pass unchanged, and the artifact/byte-gate is
  unaffected (pure module move + import rewrite → identical generated code).

### Non-goals (must NOT change)

- **No change to `CodeOp`'s variants, `impl`, or any semantics.** This is a
  move + re-path only.
- **No change to generated code / goldens / `.nobj` bytes.** A relocation of a
  Rust module must not shift any emitted output; if the byte-gate moves, the fix
  is wrong.
- Do not "fix" this by adding a re-export alias at
  `crate::arch::aarch64::ops` and calling it done — leaving the neutral enum
  physically under `aarch64/` (or re-exported from there) perpetuates the
  misfiling. The enum's file must actually move to the neutral location.
- Do not fold in unrelated cleanups to the op set while moving it.

## Blast Radius

All references found by `grep -rl "aarch64::ops" src/` (19 files). Each must
have its import path updated; none share a *behavioral* hazard (this is a
path rename):

Definition / module wiring:
- `src/arch/aarch64/ops.rs` — the definition; file moves to `src/arch/ops.rs`.
- `src/arch/aarch64/mod.rs:7` (`pub(crate) mod ops;`) — declaration moves to
  `src/arch/mod.rs`.

aarch64 backend consumers (re-path):
- `src/arch/aarch64/select.rs`, `src/arch/aarch64/encode/mod.rs`,
  `src/arch/aarch64/encode/tests.rs` — fixed by this bug.

x86_64 backend consumers (re-path):
- `src/arch/x86_64/select.rs`, `src/arch/x86_64/encode/mod.rs`,
  `src/arch/x86_64/encode/emitter.rs` — fixed by this bug.

riscv64 backend consumers (re-path):
- `src/arch/riscv64/select.rs`, `src/arch/riscv64/encode/mod.rs`,
  `src/arch/riscv64/v128.rs` — fixed by this bug.

Shared code layer consumers (re-path):
- `src/target/shared/code/mod.rs`, `src/target/shared/code/peephole.rs`,
  `src/target/shared/code/fma_fusion.rs`,
  `src/target/shared/code/regalloc/analysis.rs`,
  `src/target/shared/abi.rs` (doc-link at `abi.rs:928`) — fixed by this bug.

Target consumers (re-path):
- `src/target/linux_x86_64/code.rs`, `src/target/linux_gtk/bootstrap.rs`,
  `src/target/linux_gtk/mod.rs`, `src/target/macos_aarch64/app/term_view.rs`
  — fixed by this bug.

Docs:
- `src/docs/spec/architecture/15_x86_64-instruction-set.md:18` — update the
  documented import path to the neutral one — fixed by this bug.

Unaffected: nothing else references the module; there is no external/public
surface (`pub(crate)`), so the move cannot break downstream crates.

## Fix Design

Mechanical, single-commit refactor:

1. `git mv src/arch/aarch64/ops.rs src/arch/ops.rs`.
2. In `src/arch/mod.rs`, add `pub(crate) mod ops;`; remove the declaration from
   `src/arch/aarch64/mod.rs`.
3. Rewrite every `crate::arch::aarch64::ops::CodeOp` →
   `crate::arch::ops::CodeOp` (and any `aarch64::ops::` module references),
   across the 19 files above, plus the spec markdown reference.
4. `cargo build` + `cargo test`, then run the artifact/byte-gate
   (`scripts/artifact-gate.sh`) to prove the generated output is byte-identical.

Rejected alternative: leave the file in place and add a neutral re-export
(`pub(crate) use crate::arch::aarch64::ops as ...` from `src/arch/mod.rs`).
Rejected because it hides rather than fixes the misfiling — the enum's physical
home stays wrong and readers still find it under `aarch64/`.

Naming note (open decision below): the module could be renamed `mir`/`mir_ops`
to make its neutral role explicit, but the minimal, lowest-risk fix keeps the
name `ops` and only changes the directory.

## Phases

### Phase 1 — audit (no behavior change)

- [ ] Confirm the reference list above is exhaustive
      (`grep -rl "aarch64::ops" src/`) and that no reference is behavioral
      (all are import/path or doc-link).
- [ ] Confirm `CodeOp` is `pub(crate)` (no external consumers) so the move has
      no public-surface impact.

Acceptance: reference list complete with a per-site verdict; no non-path usage
found.
Commit: —

### Phase 2 — move + re-path

- [ ] `git mv` the file to `src/arch/ops.rs`; move the `mod ops;` declaration
      to `src/arch/mod.rs`.
- [ ] Rewrite all imports/paths to `crate::arch::ops::CodeOp` across the 19
      source files.
- [ ] Update `src/docs/spec/architecture/15_x86_64-instruction-set.md` to the
      neutral path.

Acceptance: `cargo build` + `cargo test` green; zero remaining
`aarch64::ops` references.
Commit: —

### Phase 3 — byte-gate + full validation

- [ ] Run `scripts/artifact-gate.sh` (execution-free codegen gate) and confirm
      the emitted artifacts are byte-identical (no `.nobj` delta).
- [ ] Run the project's full test suite.

Acceptance: byte-gate clean (no generated-output change); full suite green.
Commit: —

## Validation Plan

- Regression guard: a grep assertion (or reviewer check) that
  `grep -rl "aarch64::ops" src/` returns nothing.
- Runtime proof: byte-gate proving generated code is unchanged — the correctness
  bar for a pure relocation is "zero output delta".
- Doc sync: update the x86 instruction-set spec's cited import path; no other
  spec change expected.
- Full suite: `cargo test` + `scripts/artifact-gate.sh`.

## Open Decisions

- Module name — keep `ops` (minimal move) vs. rename to `mir`/`mir_ops` to make
  the neutral role explicit. Recommend keeping `ops` for the smallest,
  lowest-risk diff; a rename can be a separate follow-up. (§Fix Design)

## Summary

Zero runtime risk: `CodeOp` is a `pub(crate)` neutral enum and this is a module
relocation plus import re-path. The only real engineering check is the byte-gate
proving the move shifts no generated output. The enum's variants, `impl`, and
all semantics stay untouched; only its physical home and the import paths change.
