# bug-929: `Operand::Raw` keeps operands as untyped strings — remove the arm and give every operand kind a typed variant

Last updated: 2026-10-05
Effort: huge (>3d)
Severity: MEDIUM
Class: Other (codegen representation)

Status: Open
Regression Test: a compile-time guarantee (no `Raw` variant exists) plus byte-identical artifact gate over the full fixture corpus on every target

**Owner decision (2026-10-05):** remove `Raw` as an option. Every operand the code
generator carries is a typed enum value. Operands are not strings re-classified by
prefix sniffing or by field-name role. This is a large change, filed as its own bug.

`src/codegen/engine/operand/operand.rs:Operand` has typed arms (`VReg`, `Phys{class,index,name}`,
`Imm`, `Abi{convention,role,index}`) plus `Raw(Box<str>)`, "the long tail, rendered verbatim:
labels, symbols, type names, stack-offset sentinels, booleans, and the
`%scratch`/`%sysnr`/`%local` occupancy tokens". Physical registers that reach the code layer
as a bare `&str` also go through `Raw` (module docs, plan-78-A).

References:

- bug-917 item 9 (`.mir` "target-neutral" dump) and the owner's follow-up question.
- `mfb spec architecture mir-instruction-set`, `native-ir`; `.ai/codegen-invariants.md`.

## Why `Raw` exists (measured by grep at HEAD 28a15a153)

`Raw` is not chosen deliberately at most sites. It is the **default conversion**. These
impls make any string a `Raw` operand so that "every existing `.field(name, value)` call
compiles unchanged":

- `impl From<&str> for Operand` (`Operand::Raw(value.into())`)
- `impl From<&String> for Operand`, `impl From<String> for Operand`
- `impl From<&&str> for Operand` (the `ci(op, &[("dst", "x0"), …])` table builders, used by arch
  `select`/encoder tests **and the production riscv64 `select`/`v128` builders**)
- `Operand::parse` falls back to `Raw` for anything that isn't a vreg or a canonical decimal.
  The module's own tests assert `Operand::parse("%argMFB0")` and `"%retMFB0"` are `Raw`, so the
  ABI tokens are typed only when a producer builds `Operand::Abi` directly.

The non-test files that construct or match `Raw` (28 hits, `grep -rn 'Operand::Raw' src`):

- `src/codegen/engine/operand/operand.rs` (the `From` impls, `parse`, render)
- `src/target/shared/abi.rs` (`RegRepr::Token(spelling) => Operand::Raw(..)`, `role_token(text)`
  re-parse)
- `src/codegen/engine/mir/mir.rs` (arena-base rename matches `Raw` text)
- `src/codegen/engine/builder/code_impl.rs` (`.field` inspects `Raw` text)
- `src/codegen/engine/types/types.rs`
- `src/arch/aarch64/select.rs`, `src/arch/aarch64/encode/sizing.rs`
- `src/arch/riscv64/encode/sizing.rs`, `src/arch/riscv64/encode/relax.rs` (writes a `Raw` label)

Plus every caller that passes a string operand through the `From` impls. Those callers are
the bulk of the work, especially the hand-written per-backend runtime helpers that spell
physical registers as strings (see bug-930).

## Goal

- `Operand` has no `Raw` arm and no `From<&str>/String` conversion. Each operand kind the
  stream carries has a typed variant: labels, symbols/relocation targets, type names,
  stack-offset sentinels, booleans, condition codes, occupancy tokens, ABI tokens, physical
  and virtual registers, immediates.
- Consumers (allocator liveness, select, encoders, sizing/relaxation, the MIR rename,
  dumps) match on the variant, never on string contents.
- Every target's emitted bytes are unchanged: a full byte-identity artifact gate passes on
  macOS AArch64, Linux AArch64/x86-64/riscv64, Windows x86-64 (`.ai/testing-gates.md`).

### Non-goals (must NOT change)

- Emitted machine code, object/executable bytes, and every golden (`-mir`/`-nir` dumps may
  change only if a rendering was string-sniffed. Prove any such diff, don't re-baseline it).
- Forbidden wrong fix: renaming `Raw` or wrapping strings in a newtype that is still parsed
  by prefix.

## Blast Radius

- Every `.field(name, <string>)` call site and every `ci(...)` table builder (fixed by this bug).
- bug-930 (runtime helpers in `.mir` are per-architecture). Removing `Raw` forces the
  helpers' physical-register strings to become typed `Phys`. That is a prerequisite or
  companion to whatever bug-930 decides, but it does not by itself make the helpers neutral.
