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
| plan-78-A complete (Operand + render + corpus test landed) | A's phases all ticked (commits c4156d5b4, 23248a016) | MET (2026-08-02) |
| A's round-trip corpus test green | `cargo test --bin mfb operand::tests::parse_render_round_trips_over_the_corpus` → ok | MET (2026-08-02: 1 passed) |

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

- [x] Change `get()` to `Option<String>` (clones from the still-`String` store);
      update all `get()` callers mechanically (~200 sites: a field-name-restricted
      `.as_deref()` transform for the `== Some("lit")`/`!= Some`/`, Some` shapes,
      plus hand fixes for tuple-match scrutinees, fn-arg passes, `Vec<&str>`
      collects, and `HashSet<&str>`→`HashSet<String>` lookups). peephole's
      `Effect<'a>` became an owned `Effect` (nothing to borrow from a rendered
      value). **Correction — no `operand()` stub in Phase 1.** `operand()` must
      return `&Operand`, which cannot be produced from a `String` store (a rendered
      `&Operand` would borrow a temporary); it is real only after the flip, so it
      lands in Phase 2 with `fields: …Operand`. The Phase-1 "stub" is moot.
- [x] Tests: full `cargo test --bin mfb` green (3762 passed, 0 failed); no golden
      change.
- [x] `artifact-gate … all` — **0 diff(s)** (1144 tests, 1286 builds, 1549
      goldens), verified 2026-08-02 with storage still `String`.

Acceptance: `cargo test --bin mfb` green, `artifact-gate … all` diff-free with
storage still `String` — proving the read-side ripple is fully absorbed before
the flip. Verified 2026-08-02.
Commit: 4eafd3830

### Phase 2 — Flip storage + migrate all write sites

The atomic representation change.

- [x] Flip `types.rs:38` to `Operand`; `field()` stores `Operand`; `operand()`
      returns `&Operand`; `get()` renders it.
- [x] ~~Migrate the ~90 `abi.rs` constructors and the 9 inline literals to build
      `Operand`.~~ — **moot as written / re-scoped:** because `field` takes
      `impl Into<Operand>` and `From<&str>`→`Raw` (plan-78-A), the ~90 constructors
      *already* build `Operand` (their `&str` args become `Raw`) with **no
      signature change** — verified byte-identical by the gate. Typing *register*
      operands as `VReg`/`Phys` is not done here: `allocate_register`'s `String`
      result feeds **1794** call sites (`grep -c allocate_register\|temporary_vreg`
      → far beyond B's blast radius), and the coupling is to plan-78-C's typed
      reads, so register typing lands in C. The inline `CodeInstruction {…}`
      literals in `arch/*/select.rs` were migrated to build `Operand` (via
      `code_fields_from_mir` for Mir→Code conversions, `Operand::from` for string
      literals). **Immediates ARE typed `Imm`** at the `finalize_frame` offset
      rewrite (small, byte-identical), so the `Imm` arm is production-live.
- [x] Migrate `finalize_frame` (`codegen_utils.rs`) + the `linux_common/code.rs`
      twin, and `fma_fusion.rs`, to build/mutate `Operand` (offset rewrites build
      `Operand::imm`, base rewrites `Operand::from`; readers render).
- [x] Update the `.ncode` `CodeInstruction` dump formatter to render
      (`code_impl.rs`). (`.mir` dumps `MirInstruction`, which stays `String`, so no
      change there; the `lower_to_mir` Code→Mir path renders via the new
      `mir_fields_from_code` helper.)
- [x] Tests: added `operand::tests::code_instruction_stores_typed_operands` —
      asserts `operand("dst") == Some(&Raw("x0"))`, `operand("value") ==
      Some(&Imm(42))`, and that `get()` renders both to the identical string.
- [x] `artifact-gate … all` — **0 diff(s)** (1144 tests, 1286 builds, 1549
      goldens), verified 2026-08-02.

Acceptance: compiler builds; `cargo test --bin mfb` green (3763 passed);
`artifact-gate … all` byte-identical across all four targets. `bench-lowering.sh`
no-regression check deferred to C's before/after (B is perf-neutral by design and
adds a `render()` per read, which C removes with typed reads).
Commit: (recorded next commit)

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

- **Phase 1 has no `operand()` stub; `operand()` lands in Phase 2.** A stub can
  only be meaningful once `fields` stores `Operand` — over the still-`String`
  Phase-1 store, `operand()` would have to return `&Operand` borrowed from a
  temporary rendered value, which does not compile. So the accessor is added with
  the flip (Phase 2). No behavior lost; the Phase-1 read-ripple de-risk (get() →
  `Option<String>` + caller churn) stands on its own and is proven diff-free.
- **`get()`-caller ripple was ~200 sites, absorbed mechanically.** Most were
  `inst.get("field") == Some("lit")` needing `.as_deref()`; applied as a
  field-name-restricted, compile-verified transform, with hand fixes for the
  residual shapes (tuple-match scrutinees in `peephole`/`fma_fusion`, `fold_pair`
  args, `Vec<&str>`/`HashSet<&str>` collections that became owned). peephole's
  `Effect<'a>` dropped its lifetime to own `String`. All green + gate diff-free,
  so the ripple changed no emitted byte.
- **Register-operand typing (`VReg`/`Phys`) is deferred to plan-78-C, not done in
  Phase 2.** The plan assigned write-site migration to B and reads to C, but for
  *registers* they are coupled: to store a `VReg` the vreg must be typed at its
  source (`allocate_register`), which feeds 1794 sites; to store a `Phys` the
  physical name must be typed at selection / the coloring rewrite. B keeps every
  register operand as `Raw` (byte-identical) and C does the register typing
  alongside the typed reads it enables. B still delivers the representation flip
  (`fields: Vec<(&str, Operand)>`) and types immediates (`Imm`). Evidence:
  `grep -rn 'allocate_register\|temporary_vreg' src | grep -v 'fn ' | wc -l` → 1794.
- **Added `Display` + `PartialEq<str>`/`PartialEq<&str>` on `Operand`.** The flip
  left ~250 string-shaped reader sites (`value == "x30"`, `format!("{value}")`,
  arch token realization). Rather than render at every one, `Operand` gained a
  render-based `Display` and string equality so those readers keep working
  verbatim; C replaces the hot-path ones with typed matches. The remaining
  fn-arg / `.strip_prefix` / write sites render explicitly (`value.render()`) or
  wrap (`Operand::from(...)`).
- **Added `mir_fields_from_code` / `code_fields_from_mir` /
  `rename_operand_field_values` (`mir.rs`).** `MirInstruction` stays
  `String`-valued while `CodeInstruction` is now `Operand`-valued, so the
  `select` / `lower_to_mir` boundary needs explicit field-bag converters (render
  one way, `Operand::from` the other); the arena-base rename on selected
  `CodeInstruction` streams needed an `Operand` twin of `rename_field_values`.

## Summary

B is the high-blast-radius, perf-neutral flip. Its risk is entirely
byte-identity through `finalize_frame`'s in-place rewrite (and its twin) and the 9
inline arch literals — all gated on `artifact-gate … all`. No hot-path read is
changed here; the speedup arrives in C.
