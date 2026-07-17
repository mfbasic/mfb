# plan-50-D: the `str_u16` sized-store primitive

Last updated: 2026-07-16
Effort: small (<1h)
Depends on: nothing (independent of A–C; may land in any order before plan-50-E)

Adds the one memory primitive the struct marshaler needs and the compiler does not
have: a 16-bit store. `ldr_u16` is encoded on every backend; **`str_u16` is
encoded on none**, and there is no `abi::store_u16` to emit it.

A primitive with no callers — the lowest-risk, separately-valuable work in
plan-50, and safe to land first. Without it, plan-50-E cannot marshal a
`CInt16`/`CUInt16` struct field and would have to either reject those field types
(a hole in plan-50-B's allow-list) or silently store 4 bytes over a 2-byte field
(memory corruption).

The single behavioral outcome: `abi::store_u16` emits a `str_u16` instruction that
each of the three backends encodes to the architecture's 16-bit store — `strh` on
aarch64, `mov word ptr` on x86-64, `sh` on riscv64 — and the existing `ldr_u16`
reads back exactly what it wrote.

References (read first):

- `src/target/shared/abi.rs:780-829` — the sized memory helpers. Present:
  `load_u64` (`:780`), `load_u32` (`:788`), `load_u16` (`:796`, `#[allow(dead_code)]`),
  `load_u8` (`:803`), `store_u64` (`:810`), `store_u32` (`:817`), `store_u8` (`:824`).
  **Absent: `store_u16`.** Note the asymmetry is exactly one function.
- `src/arch/x86_64/encode/emitter.rs:608-612` — the width-dispatch precedent:
  ```rust
  "ldr_u32" => mem_load(instruction, MemWidth::U32),
  "ldr_u16" => mem_load(instruction, MemWidth::U16),
  "str_u32" => mem_store(instruction, MemWidth::U32),
  ```
  `MemWidth::U16` already exists and is already used by the load side.
- `src/arch/riscv64/encode/emitter.rs:228-229` — `ldr_u32` → `lwu` (funct3 `0b110`),
  `ldr_u16` → `lhu` (funct3 `0b101`), via `emit_load`. The store side needs `sh`.
- `src/arch/aarch64/encode/tests.rs:568` — the encoder's op inventory, which lists
  `"ldr_u64", "ldr_u32", "ldr_u16", "ldr_u8", "str_u64", "str_u32", "str_u8"` —
  `str_u16` is conspicuously missing from a list that is otherwise symmetric.
- `src/arch/riscv64/encode/tests.rs:783,790` — the same asymmetry, explicit:
  loads loop `["ldr_u32", "ldr_u16", "ldr_u8"]`, stores loop `["str_u32", "str_u8"]`.
- `.ai/compiler.md` — the Fast codegen gate (`scripts/artifact-gate.sh`) and the
  acceptance requirement.
- Memory `plan-99-rv64-impl` / `bug-87-linux-exe-nondeterminism` — rv64 encoder
  work is hardware-validated on the 2229 box.

## 1. Goal

- `abi::store_u16(src, base, offset)` exists and emits a `str_u16` instruction,
  mirroring `store_u32` (`src/target/shared/abi.rs:817`) exactly in shape.
- All three backends encode `str_u16`:
  - **aarch64** — `strh w<src>, [<base>, #offset]`
  - **x86-64** — `mem_store(instruction, MemWidth::U16)` (a one-line arm; the
    width already exists)
  - **riscv64** — `sh` (S-type, funct3 `0b001`) via the existing `emit_store`
- A store/load round-trip through `str_u16`/`ldr_u16` preserves the low 16 bits
  and writes **exactly 2 bytes** — the adjacent bytes are provably untouched.
- Every existing binary is **byte-identical**: this adds an op nothing emits yet.

### Non-goals (explicit constraints)

- No new callers. `abi::store_u16` is `#[allow(dead_code)]` until plan-50-E, the
  same posture `load_u16` has today (`:795`).
- No sign-extending 16-bit load (`ldrsh`). `ldr_u16` zero-extends; plan-50-E
  sign-extends in a separate step for signed fields. Adding `ldrsh` here would be
  a second primitive with no caller and no test.
- No change to any existing op's encoding. Zero golden churn is the acceptance bar.
- Windows x86-64 (plan-47) shares the x86-64 emitter, so it is covered by the same
  arm; no separate work.

## 2. Current State

The sized memory helpers in `src/target/shared/abi.rs` are symmetric except for one
hole:

| width | load | store |
|---|---|---|
| 64 | `load_u64` `:780` | `store_u64` `:810` |
| 32 | `load_u32` `:788` | `store_u32` `:817` |
| **16** | `load_u16` `:796` | **missing** |
| 8 | `load_u8` `:803` | `store_u8` `:824` |

Each helper is a thin `CodeInstruction::new("<op>")` with `src`/`dst`, `base`, and
`offset` fields — `store_u16` is a five-line function by inspection of `store_u32`.

On the encoder side, `ldr_u16` is live on all three backends (x86_64
`emitter.rs:609`, riscv64 `emitter.rs:229` `lhu`, aarch64 per its op inventory at
`tests.rs:568`), but `str_u16` appears in **no** emitter and in **no** test
inventory. The riscv64 test at `:790` makes the gap unambiguous: its store loop is
`["str_u32", "str_u8"]`.

`load_u16` is `#[allow(dead_code)]` — it has been encodable but uncalled. That is
the exact posture `store_u16` will hold until plan-50-E, and it is why this lands
with no behavior change.

`link_thunk.rs` uses only `load_u64`/`store_u64` today (`src/target/shared/abi.rs:780-840`);
it has never needed a sized store because every ABI slot is one register-width value.

## 3. Design Overview

Four small edits, one per layer:

```
  src/target/shared/abi.rs:store_u16          <-- new, mirrors store_u32:817
        │  emits CodeInstruction "str_u16"
        ├── src/arch/aarch64/encode/  ->  strh
        ├── src/arch/x86_64/encode/emitter.rs -> mem_store(.., MemWidth::U16)
        └── src/arch/riscv64/encode/emitter.rs -> sh (funct3 0b001)
```

There is no design latitude here — the shape is dictated three times over by the
adjacent `store_u32`, `store_u8`, and `ldr_u16` implementations. The work is
mechanical; the *risk* is entirely in the encodings being correct at the bit level.

**Where the risk concentrates:** the three encoders. A wrong opcode/funct3 is not
caught by `cargo build` — it produces a valid binary that stores the wrong width to
the wrong place. Mitigated by: per-backend encoder unit tests asserting the exact
expected bytes against the architecture manual, and a runtime round-trip proof on
real hardware for each arch (§Validation). riscv64 in particular is validated on
the 2229 box, per the established `plan-99-rv64-impl` practice.

Rejected alternative: **skip `str_u16` and have plan-50-B reject `CInt16`/`CUInt16`
struct fields.** Rejected on two counts: it would put a permanent hole in the ctype
allow-list for no reason (`SF_INFO`/`SF_FORMAT_INFO` happen not to use 16-bit
fields, but the next binding will), and `.ai/compiler.md` forbids an "unsupported"
stand-in as a substitute for a real implementation. The primitive is under an hour.

Rejected alternative: **emulate a 16-bit store with read-modify-write on a 32-bit
word.** Rejected: it is wrong at a struct's trailing edge (it would read and
rewrite the 2 bytes past the field, which may be padding the C library relies on,
or past the buffer entirely), and it is slower and more code than the one
instruction every ISA already has.

## 4. Detailed Design

### 4.1 The helper

```rust
// src/target/shared/abi.rs, immediately after store_u32:817
#[allow(dead_code)]
pub(crate) fn store_u16(src: &str, base: &str, offset: usize) -> CodeInstruction {
    CodeInstruction::new("str_u16")
        .field("src", src)
        .field("base", base)
        .field("offset", &offset.to_string())
}
```

### 4.2 The encodings

- **aarch64** — `strh w<src>, [<base>, #imm12]`: `01 111 0 01 00 imm12 Rn Rt`
  (`STRH (immediate)`, unsigned offset). The `ldr_u16` counterpart (`ldrh`) differs
  only in the `L`/opc bit; implement beside it and mirror its offset scaling
  (`imm12` scales by 2 for halfword) and its offset-range validation. Add
  `"str_u16"` to the op inventory at `tests.rs:568`.
- **x86-64** — one arm beside `:612`:
  ```rust
  "str_u16" => mem_store(instruction, MemWidth::U16),
  ```
  `MemWidth::U16` is already exercised by `ldr_u16` at `:609`, so the operand-size
  prefix (`0x66`) path already exists.
- **riscv64** — one arm beside the store dispatch, `sh` is S-type funct3 `0b001`
  (`sb`=`0b000`, `sh`=`0b001`, `sw`=`0b010`, `sd`=`0b011`), via the existing
  `emit_store`. Add `"str_u16"` to the store loop at `tests.rs:790`.

### 4.3 Offset range

Each backend already validates/scales the offset for its existing widths; `str_u16`
must reuse that path, not bypass it. aarch64's scaled `imm12` for halfword covers
0..8190 in steps of 2 — far beyond `MAX_CSTRUCT_SIZE` (1024, plan-50-B §4.4), so
plan-50-E's struct buffers can never exceed it. An unaligned or out-of-range offset
must **error**, matching how `str_u32` behaves today (`tests.rs:618-619` asserts
`str_u32` errors on a bad operand).

## Compatibility / Format Impact

None. A new op that nothing emits: no `.mfp` change, no IR change, no golden
change, no existing instruction re-encoded. `BINARY_REPR_VERSION` untouched.

## Phases

One landable unit.

### Phase 1 — the primitive and its three encodings

- [ ] Add `abi::store_u16` to `src/target/shared/abi.rs` beside `store_u32:817`
      (§4.1), `#[allow(dead_code)]`.
- [ ] aarch64: encode `str_u16` → `strh`; add it to the op inventory at
      `src/arch/aarch64/encode/tests.rs:568` and the round-trip list at `:595`.
- [ ] x86-64: add the `"str_u16" => mem_store(instruction, MemWidth::U16)` arm at
      `src/arch/x86_64/encode/emitter.rs:~612`; extend the op list at
      `src/arch/x86_64/encode/tests.rs:362`.
- [ ] riscv64: encode `str_u16` → `sh` (funct3 `0b001`) beside
      `src/arch/riscv64/encode/emitter.rs:228-229`; add `"str_u16"` to the store
      loop at `src/arch/riscv64/encode/tests.rs:790`.
- [ ] Tests: per-backend encoder tests asserting the **exact bytes** for a
      representative `str_u16` against the architecture manual — not merely that
      it encodes without error.
- [ ] Tests: per-backend negative test that a bad operand errors, mirroring
      `src/arch/aarch64/encode/tests.rs:618-619`.
- [ ] Spec: `src/docs/spec/memory/**` (or the topic owning the MIR op vocabulary —
      find it with `mfb spec`) gains `str_u16` beside `str_u32`/`ldr_u16` in the
      sized-memory op table. Cite `[[src/target/shared/abi.rs:store_u16]]`.

Acceptance: each backend encodes `str_u16` to the exact expected bytes in a unit
test; a `str_u16`/`ldr_u16` round-trip through a stack slot returns the low 16 bits
**and leaves the two adjacent bytes unchanged** (the test must assert the
neighbours, since a wrong width passes a naive round-trip);
`scripts/artifact-gate.sh` shows **zero** instruction-byte churn in every existing
program.
Commit: —

## Validation Plan

- Tests: exact-bytes encoder tests + negative operand tests in
  `src/arch/{aarch64,x86_64,riscv64}/encode/tests.rs`. These are the substance —
  a round-trip test alone cannot distinguish a 16-bit store from a 32-bit one, so
  the adjacent-bytes assertion is mandatory.
- Runtime proof: a small MFBASIC program whose generated code stores `0xFFFF` via
  `str_u16` into a known stack slot pre-filled with a sentinel, then reads back
  both the field and its neighbours — run natively on **each** architecture:
  aarch64 (local/2223), x86_64 (2227 or 2228), riscv64 (2229). Per `.ai/compiler.md`
  a codegen change is not done until it executes correctly; per
  `plan-99-rv64-impl`, rv64 encoder work is hardware-validated on 2229.
  Note: this needs a temporary caller to emit the op — a throwaway test harness, not
  a shipped one. If wiring a caller proves disproportionate for a dead-code
  primitive, the honest alternative is to defer this runtime proof to plan-50-E
  (which supplies the first real caller) and say so on the phase's `Commit:` line
  rather than claim the primitive is runtime-verified.
- Doc sync: the MIR sized-memory op table; `cargo build`,
  `cargo test --bin mfb spec`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` with
  **zero** golden churn, plus `scripts/artifact-gate.sh`.

## Open Decisions

- **Runtime-prove here, or at plan-50-E?** Recommend proving the encodings by
  exact-bytes unit tests here, and taking the *runtime* proof at plan-50-E where a
  real caller exists — a dead-code op has no natural runtime exercise, and
  fabricating one risks more than it proves. State this explicitly on the phase's
  ledger line so the gap is visible, not silent. Alternative: build the throwaway
  harness now for a fully self-contained sub-plan.
- **Also add `ldrsh`/sign-extending 16-bit load?** Recommend no — plan-50-E
  sign-extends narrow signed fields with an explicit shift pair (the same technique
  `link_thunk.rs:453-454` already uses for `CInt32`), so a second load op earns
  nothing.

## Summary

Mechanical work whose entire risk is three bit-level encodings that `cargo build`
cannot check. Contained by exact-bytes tests against the architecture manuals and
per-arch hardware validation, with zero-churn acceptance proving nothing existing
moved.

Untouched: everything else. This op has no callers until plan-50-E.
