# plan-78-B: Flip `CodeInstruction.fields` to typed `Operand` storage

Last updated: 2026-08-02
Effort: large (3h–1d)
Depends on: plan-78-A (the `Operand` type + proven render fidelity must exist)

Change the stored operand representation from `String` to the typed `Operand`
introduced in A: `CodeInstruction.fields: Vec<(&'static str, Operand)>`. Every
*write* site builds an `Operand`; every *read* site keeps working by reading the
rendered string through an unchanged-behavior `get()`. This is the heavy
blast-radius phase, and it delivers **no perf win by itself** — its whole job is
to flip the representation while keeping every emitted byte identical, so C can
then read typed registers on the hot path.

The single behavioral outcome: `CodeInstruction.fields` is `Operand`-valued, the
compiler builds and all tests pass, and `artifact-gate … all` is diff-free (the
`.ncode`/`.mir` dumps render `Operand` back to the identical text).

References:

- plan-78-A (`planning/plan-78-A-operand-type.md`) — the `Operand` type, `render`,
  and the round-trip proof this sub-plan relies on.
- Census in the plan-78 investigation — the write/read site inventory below.

## Prerequisites

See plan-78-A's Prerequisites table (stated once for the feature). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-78-A complete (Operand + render + corpus test landed) | `ls planning/completed/plan-78-A-* 2>/dev/null` OR A's phases all ticked | NOT MET until A lands |
| A's round-trip corpus test green | `cargo test --bin mfb operand_round_trip` (or A's test name) → ok | NOT MET until A lands |

> If A is not complete, B cannot start — full stop. B does not re-derive or
> partially build `Operand`; it consumes A's proven type.

## 1. Goal

- `CodeInstruction.fields: Vec<(&'static str, Operand)>` (`types.rs:38`).
- All ~9 inline struct-literal sites and the ~90 `abi.rs` constructors build
  `Operand` values; every other producer keeps compiling via `impl Into<Operand>`.
- All read sites keep their current behavior: `get()` returns the rendered
  string; the encoders and `finalize_frame`/peephole/fma_fusion see identical
  values.
- `artifact-gate … all` is diff-free.

### Non-goals (explicit constraints)

- **No hot-path typed reads.** The regalloc analysis still reads via the
  rendered-string path in B — migrating it to typed reads is C. (B alone gives no
  measurable speedup; that is expected and fine.)
- **No byte change / no golden regeneration.** A diff means a `render()` gap —
  fix `Operand`, never re-baseline (AGENTS.md).
- **`MirInstruction` stays `String`-valued** (mir.rs:28) — out of scope.
- **No `-regalloc bump` change.**

## 2. Current State

Post-A: `Operand` exists and renders faithfully, but `CodeInstruction.fields`
still stores `String` (A stored `operand.render()`). The write and read surfaces
to migrate (from the census, with commands):

### Measured populations

| Surface | Count | Command / sites |
|---|---|---|
| `abi.rs` typed constructors (build field values) | ~90 | `src/target/shared/abi.rs` (move_register:443, move_immediate:449, load_u64:795, branch_eq:718, vector-op macro:1051, …) |
| Inline `CodeInstruction {…}` literals (non-test) | 9 | `grep -rn "CodeInstruction {" src --include=*.rs \| grep -v "\-> CodeInstruction {"` → linear_scan.rs:388, aarch64/select.rs:68,73,78, x86_64/select.rs:793,887,912,935,944, riscv64/select.rs:624 |
| In-place `fields` mutation sites (write) | finalize_frame + fma_fusion | `codegen_utils.rs:825,845` (+ parallel copy `linux_common/code.rs:1193,1321`); `fma_fusion.rs:93,116` |
| Read via `get()` / encoder `field()` | many | `code_impl.rs:16`, `encode_operand.rs:15` — keep working via render |
| Dump formatters (render to golden text) | 2 | `code_impl.rs:258` (`.ncode`), `mir.rs:748` (`.mir`) |

### Verified properties

- **`get()` currently returns `Option<&str>`** (`code_impl.rs:16`) borrowing the
  stored `String`. After the flip it must return the *rendered* value; a rendered
  `String` cannot be borrowed from the stored `Operand`, so the signature changes
  to `Option<String>` (or `Cow`). Verified there is no path relying on `get()`
  returning a borrow tied to instruction lifetime beyond the immediate compare/
  parse (`get()` results are used as `== "x0"`, `.parse()`, or cloned) — so
  `.as_deref()`/owned return is a mechanical caller update. (Re-confirm during
  execution; this is the main ripple.)
- **A's corpus test proves every value B round-trips is byte-stable**, so the
  encoders (which re-parse `get()`'s string) and the dumps produce identical
  output.

## 3. Design Overview

The flip is inherently atomic (a struct field type cannot be half-changed), but
its ripple is bounded by two funnels:

1. **Writes** all go through `.field(impl Into<Operand>)` (A) or the ~90 `abi.rs`
   constructors + 9 inline literals. Migrate the constructors and literals to
   build typed `Operand`; everything else already passes `impl Into<Operand>`.
2. **Reads** all go through `get()` / encoder `field()`. Make `get()` render, so
   every read site is behavior-identical with at most a `&str → String` caller
   tweak. Encoders are **untouched** in B — they call `field()` → `get()` →
   rendered string → the same parse they do today.

Correctness risk concentrates in: `finalize_frame`'s in-place `fields` mutation
(it rewrites base/offset operands — must build `Operand`, and its parallel copy
in `linux_common/code.rs` must stay in lockstep), and the 9 inline literals in
arch selection. Both are guarded by `artifact-gate … all`.

Rejected: keeping `String` storage and only interning in the analysis — that is
the incremental approach explicitly not chosen; B commits to the representation.

## 4. Detailed Design

- Flip `types.rs:38` to `Vec<(&'static str, Operand)>`.
- `field()` stores the `Operand` directly (drop the A-era `.render()` call).
- `get(name) -> Option<String>` renders; add `operand(name) -> Option<&Operand>`
  for C's hot path (unused in B).
- `abi.rs` constructors: replace `.field("dst", reg_string)` with typed
  `Operand::phys/vreg/imm` where the kind is known; `Raw` for symbols/labels/
  types. (These constructors already know each operand's kind by construction.)
- 9 inline literals: build `fields: vec![("op", Operand::…)]`.
- `finalize_frame` (`codegen_utils.rs`) + its `linux_common/code.rs` twin: the
  offset/base rewrites construct `Operand::Raw`/`Imm` instead of formatting
  strings; `fields` iteration/mutation adapts to the `Operand` element type.
- `fma_fusion.rs` mutation sites build `Operand`.
- Dump formatters (`code_impl.rs:258`, `mir.rs:748` for the CodeInstruction side
  only): render the `Operand` value (identical text).

## Compatibility / Format Impact

Externally none — `.ncode`/`.mir`/executables byte-identical. Internal only: the
`fields` element type and `get()`'s return type change (both crate-private).

## Phases

> **NOTE — keep boxes/`Commit:` current; run `artifact-gate … all` after each.**

### Phase 1 — `get()` renders; add typed accessor (still String storage)

De-risk the read ripple before the flip: change `get()` to return the rendered
value from the (still-String) store — a no-op that surfaces every caller needing
`&str → String`/`.as_deref()`.

- [ ] Change `get()` to `Option<String>` (render/clone from the current String
      store) and add `operand()` stub; update all `get()` callers mechanically.
- [ ] Tests: existing suite must stay green; no golden change.
- [ ] `artifact-gate … all` — zero diffs.

Acceptance: `cargo test --bin mfb` green, `artifact-gate … all` diff-free with
storage still `String` — proving the read-side ripple is fully absorbed before
the flip.
Commit: —

### Phase 2 — Flip storage + migrate all write sites

The atomic representation change.

- [ ] Flip `types.rs:38` to `Operand`; `field()` stores `Operand`; `operand()`
      returns `&Operand`; `get()` renders it.
- [ ] Migrate the ~90 `abi.rs` constructors and the 9 inline literals to build
      `Operand`.
- [ ] Migrate `finalize_frame` (`codegen_utils.rs`) + the `linux_common/code.rs`
      twin, and `fma_fusion.rs`, to build/mutate `Operand`.
- [ ] Update the `.ncode`/`.mir` `CodeInstruction` dump formatters to render.
- [ ] Tests: add a codegen test asserting a representative instruction's
      `operand("dst")` is the expected typed `Operand` (not a string).
- [ ] `artifact-gate … all` — zero diffs.

Acceptance: compiler builds; `cargo test --bin mfb` green; `artifact-gate … all`
byte-identical across all four targets; `bench-lowering.sh` shows **no
regression** (B is perf-neutral by design).
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb` (incl. the new typed-operand assertion); encoder
  test modules (`aarch64/x86_64 encode/tests.rs`) stay green.
- Byte-identity: `artifact-gate.sh … all` diff-free after every phase — the core
  guardrail; a diff is a render gap, not a re-baseline.
- Cross-target: the gate's `all` sweep covers aarch64/x86_64/riscv64 + the
  windows/linux data images.
- Coverage: `scripts/coverage-check.sh` — the migrated code stays ≥95%.
- Doc sync: none.
- Acceptance: `cargo test --workspace` + `artifact-gate … all` green.

## Open Decisions

- **`get()` return type** — `Option<String>` vs. `Option<Cow<str>>`. Recommend
  `Option<String>` for simplicity (the render is cheap and B is perf-neutral);
  revisit only if profiling in C shows `get()` rendering on a hot path. (§4)
- **`finalize_frame` twin** — unify `codegen_utils.rs` and `linux_common/code.rs`
  copies now vs. later. Recommend migrate both in lockstep in B; a dedup is a
  separate cleanup, not this plan. (§3)

## Corrections

<Filled in during execution.>

## Summary

B is the high-blast-radius, perf-neutral flip. Its risk is entirely
byte-identity through `finalize_frame`'s in-place rewrite (and its twin) and the 9
inline arch literals — all gated on `artifact-gate … all`. No hot-path read is
changed here; the speedup arrives in C.
