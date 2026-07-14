# plan-41-C: Scalar primitive — native codegen

Last updated: 2026-07-13
Effort: medium
Depends on: plan-41-A, plan-41-B

Emit real machine code for `Scalar`: a register-carried 32-bit value with an
immediate load, `=`/`<>`/ordering comparisons, register-copy value semantics,
default materialization, and inline collection-element layout — on all three
backends (aarch64, x86_64, riscv64). After this sub-plan a program that binds a
`` `A` `` literal, compares scalars, defaults a `MUT c AS Scalar`, and stores
scalars in a `List OF Scalar` **compiles and runs**, byte-deterministically
across targets. This is the first runnable milestone for the feature.

References (read first):

- plan-41-A (front-end: `Type::Scalar`, `Expression::Scalar(u32)`), plan-41-B
  (`TYPE_SCALAR`, const-pool encoding, `COLLECTION_TYPE_SCALAR`).
- `mfb spec memory scalar-storage` — the register-carried scalar model.
- The `Byte`/`Integer` codegen paths are the template: register-carried, immediate
  load, integer-comparison. `Byte` is 1 byte / `Integer` is 8; `Scalar` is 4.

## 1. Goal

- `LET c = `` `A` `` emits a 4-byte immediate load of codepoint 65 into a GPR; the
  value lives in a register, never on the heap.
- `` `a` = `a` ``, `` `a` <> `b` ``, and `` `a` < `b` `` emit integer
  compares over the 32-bit codepoint and produce the correct `Boolean`, matching
  the front-end ordering (codepoint order) on all three backends.
- `MUT c AS Scalar` with no initializer materializes codepoint 0.
- `List OF Scalar` stores each element as a 4-byte inline payload (alignment 4);
  read-back yields the stored codepoint.
- `toString(`` `A` ``)` — deferred to plan-41-D — is NOT required here; this
  sub-plan proves the primitive itself runs.
- Emitted binaries are byte-identical across repeated builds (determinism gate).

### Non-goals (explicit constraints)

- **No arithmetic emit.** `Scalar` must not be added to the unary-negation guard
  (`builder_values.rs:1367`) or any arithmetic op lowering. Only load, compare,
  copy, default, and collection layout.
- **No conversions here.** `toScalar`/`toInt`/`toString` and the strings seam are
  plan-41-D.
- **4-byte payload/alignment.** Introduce alignment 4 to the collection-layout
  tables (currently only 1 and 8 exist); do not shoehorn `Scalar` into the 8-byte
  group.

## 2. Current State

Storage classes: `src/target/shared/plan/mod.rs:133-146`
(`StorageClass::Byte` :136, `Money` :140); size/align in
`src/target/shared/plan/lower.rs:150-171` (`Byte` 1/1 :154, `Integer`/`Money`
8/8); JSON tag in `src/target/shared/plan/json.rs:156-160`.

Immediate/value lowering: `native_immediate_value`
`src/target/shared/code/type_utils.rs:345-365` (Float/Fixed/Money produce bit
patterns; the `_` arm passes an integer value through — a codepoint uses this);
call site `src/target/shared/code/builder_values.rs:222`; `move_immediate`
`src/target/shared/abi.rs:387-392` → `mov_imm` (`src/arch/ops.rs:235,390`) with
per-arch realization in `src/arch/{aarch64,x86_64,riscv64}/encode/`. A 32-bit
immediate is already expressible as an integer immediate; verify sizing in e.g.
`src/arch/aarch64/encode/sizing.rs`.

Value semantics / default: `src/target/shared/code/builder_value_semantics.rs:58-66`
— the `Byte|Integer|Float|Fixed|Money` register-scalar group (default via
`mov_imm 0`, copy via register move). Add `Scalar`.

Collection inline layout: `src/target/shared/code/builder_collection_layout.rs:4-72`
— `inline_collection_payload_size` (`Boolean|Byte|String`→1 :62,
`Integer|Float|Fixed|Money`→8 :63) and `collection_payload_alignment`
(1 :62 / 8 :63). Also `src/target/shared/code/type_utils.rs:63-95`
(`collection_type_code` + `collection_payload_alignment_for_code`, currently the
8-byte set). `Scalar` needs size 4 / alignment 4 — a **new width** in these
tables.

Comparison lowering: scalar comparisons ride the integer-compare path; the
Boolean-result comparison codegen is shared. `String` (orderable, non-numeric)
is the model for wiring an ordered compare that is not arithmetic. Static
const-folding of value semantics is `builder_value_semantics.rs:58` and
`data_objects.rs:369` (global data-object primitive allowlist).

## 3. Design Overview

Five additive codegen edits, each mirroring `Byte`/`Integer` with a 4-byte width:

1. **StorageClass::Scalar** (size/align 4/4) + JSON tag `"scalar"`.
2. **Immediate load** — route `Expression::Scalar(cp)` through
   `native_immediate_value`'s pass-through integer path; confirm `mov_imm`
   emits a correct 4-byte (zero-extended) load on each arch.
3. **Comparison** — add `Scalar` to the ordered-compare dispatch beside the
   numeric/String cases so `= <> < <= > >=` lower to integer compares on the
   32-bit codepoint.
4. **Value semantics + default** — add `Scalar` to the register-scalar group in
   `builder_value_semantics.rs` (copy = register move, default = `mov_imm 0`) and
   the data-object allowlist.
5. **Collection layout** — add `Scalar` → size 4 / alignment 4 to the four layout
   tables, introducing alignment 4.

**Risk concentrates in two places:** (a) the **32-bit immediate width** on each
backend — a codepoint like U+1F600 (128512) needs a real 4-byte load, so verify
per-arch `mov_imm` sizing rather than assuming the 8-byte path truncates safely;
and (b) **alignment 4 in the collection layout**, a width the layout tables have
never emitted — the element stride/padding math must be correct or reads return
garbage. Both are covered by the runtime acceptance below on all three arches.

Rejected alternatives:
- *Store `Scalar` as 8 bytes to reuse the existing 8-byte layout group* —
  rejected; wastes 4 bytes per element and diverges from the "narrowest correct
  width" precedent. Alignment 4 is the correct, if new, path.
- *Lower comparison through the numeric promotion path* — rejected; `Scalar` is
  non-numeric (plan-41-A). It uses the ordered-non-numeric compare path, like
  `String` but with a fixed-width integer compare instead of a byte-wise one.

## Compatibility / Format Impact

Additive codegen only. New `StorageClass::Scalar`, a new `"scalar"` plan JSON
tag, and alignment-4 collection payloads. No change to existing type layouts. The
`.mfp` const-pool/wire format is unchanged from plan-41-B.

## Phases

### Phase 1 — Storage + immediate load

- [ ] Add `StorageClass::Scalar` (`src/target/shared/plan/mod.rs:133-146`),
      size/align 4/4 (`src/target/shared/plan/lower.rs:150-171`), JSON tag
      `"scalar"` (`src/target/shared/plan/json.rs:156-160`).
- [ ] Route `Expression::Scalar(cp)` through `native_immediate_value`
      (`type_utils.rs:345-365`) / `builder_values.rs:222`; confirm `move_immediate`
      → `mov_imm` emits a correct 4-byte load on each arch (check
      `src/arch/{aarch64,x86_64,riscv64}/encode/` sizing), including a codepoint >
      0xFFFF such as U+1F600.
- [ ] Tests: an artifact/codegen unit test that a `` `A` `` literal lowers to a
      4-byte immediate load; a `` `\u{1F600}` `` literal loads 128512 correctly.

Acceptance: `LET c = `` `\u{1F600}` `` : io::print(toInt-free debug) — a program
binding a scalar literal compiles and the emitted immediate equals the codepoint
on all three arches (via artifact inspection or a Phase-3 runtime check).
Commit: —

### Phase 2 — Comparison + value semantics + default

- [ ] Add `Scalar` to the ordered-compare dispatch so `= <> < <= > >=` lower to
      32-bit integer compares (beside the String ordered-compare path).
- [ ] Add `Scalar` to the register-scalar group in
      `src/target/shared/code/builder_value_semantics.rs:58-66` (copy = register
      move; default = `mov_imm 0`) and the global data-object allowlist
      (`src/target/shared/code/data_objects.rs:369`).
- [ ] Tests: codegen unit tests for scalar compare and default materialization.

Acceptance: a program computing `` `a` < `b` `` and `` `b` < `a` `` and
defaulting a `MUT c AS Scalar` compiles; comparisons and default are covered by
runtime in Phase 3.
Commit: —

### Phase 3 — Collection layout + cross-arch runtime proof (highest-risk last)

- [ ] Add `Scalar` → size 4 / alignment 4 to `inline_collection_payload_size` and
      `collection_payload_alignment`
      (`src/target/shared/code/builder_collection_layout.rs:4-72`), plus
      `collection_type_code`/`collection_payload_alignment_for_code`
      (`src/target/shared/code/type_utils.rs:63-95`). Introduce alignment 4.
- [ ] Write a runtime acceptance program: bind scalar literals, compare them
      (ordering + equality), default a `MUT c AS Scalar`, build a
      `List OF Scalar`, read elements back — and assert expected output.
- [ ] Run it on all three targets (host + x86_64 + riscv64 per the repo's remote
      validation) and confirm byte-identical binaries across repeated builds
      (determinism gate).
- [ ] Tests: the acceptance program under the repo's acceptance/rt-behavior test
      folder, with a golden.

Acceptance: the acceptance program prints the expected results (correct ordering,
default 0, correct `List OF Scalar` read-back) on aarch64, x86_64, and riscv64,
with byte-identical emitted binaries.
Commit: —

## Validation Plan

- Tests: codegen/artifact unit tests (Phases 1–2) + an acceptance program with a
  golden (Phase 3), per the repo's `tests/` layout (acceptance / rt-behavior).
- Runtime proof: the Phase-3 program run on all three backends — this is the
  feature's first end-to-end runtime evidence.
- Doc sync: `src/docs/spec/memory/01_scalar-storage.md` gets `Scalar` = 4 bytes
  (numbers land here; prose sync is plan-41-E).
- Acceptance: `cargo test` green; `scripts/artifact-gate.sh` clean; cross-arch
  determinism gate passes; no memory leaks in the scalar acceptance program.

## Open Decisions

_Resolved 2026-07-13 (user)._

- **Alignment 4 vs. pad to 8 in collections — DECIDED: true alignment 4
  (introduce the new width).** It is the correct layout and the tables are small.
  Padding to 8 would work but wastes memory and hides the width mismatch.
  Verify no existing 8-byte-only assumption in the element stride math. (§3)

## Summary

Two genuine hazards — per-arch 32-bit immediate width and the new
alignment-4 collection payload — both nailed down by a cross-arch runtime
acceptance program. Everything else mirrors `Byte`/`Integer`. This sub-plan turns
`Scalar` from a type-checked abstraction into a running value.
