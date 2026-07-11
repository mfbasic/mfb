# bug-39: Two latent LOW ELF-linker defects — SysV `DT_HASH` chain off-by-one, and RISC-V `auipc` hi20 silent truncation

Last updated: 2026-07-08
Effort: small (<1h)

Two LOW-severity latent defects in the Linux ELF linker (`src/os/linux/link/**`).
Batched (same subsystem, both LOW, both latent).

**(1) SysV symbol hash chain (`DT_HASH`) written with an off-by-one shift.**
`src/os/linux/link/elf.rs:492-504` (dynamic ELF hash section). For N imported
symbols the `chain` array should be `chain[0]=0` (unused null-symbol slot),
`chain[i]=i+1` for the linked list, `chain[N]=0` terminator. The code writes the
first computed chain value into `chain[0]` (`:496-504`) instead of `chain[1]`, so
`chain[0]` is `2` (should be `0`) and every real entry is shifted down by one; a
by-name lookup of the 2nd+ imported symbol terminates early / skips symbols.
**Latent**: our imports are all *undefined* symbols resolved by the dynamic loader
via relocation `r_info` symbol indices, not via our `DT_HASH`; nothing consults this
table, so hardware-validated binaries load fine. Becomes real if an *exported*
symbol is ever added or a tool consults the hash.

**(2) RISC-V `auipc` hi20 silently truncated (LNK-06 class, new site).**
`src/os/linux/link/mod.rs:396-400` (`riscv_hi_lo`) returns `((hi as u32) & 0xfffff,
lo)` — masking the `auipc` immediate to 20 bits with **no range check**. For a
displacement exceeding the `auipc`+I-type ±2 GB reach (a >2 GB text segment, or a
GOT/data target beyond ±2 GB), the high bits drop silently and the patched
`auipc`/`jalr` jumps to a wrong address. Same silent-truncation class as the known
**LNK-06** (`branch_imm26`/`page21`/x86 `rel32`), at the RISC-V sites
(`riscv_call`/`riscv_pcrel`/`riscv_got`/`emit_import_stub`). **Latent**: requires
>2 GB reach.

The single correct behavior a fix produces: the `DT_HASH` chain is emitted with
`chain[0]=0` and correctly-indexed entries; RISC-V `auipc` displacement is
range-checked and returns `Err` on overflow (mirroring the LNK-06 remediation).

Severity LOW for both (latent).

References:

- `src/os/linux/link/elf.rs:492-504` (hash chain; first value into `chain[0]`).
- `src/os/linux/link/mod.rs:396-400` (`riscv_hi_lo`, unchecked 20-bit mask), used by
  `riscv_call`/`riscv_pcrel`/`riscv_got`/`emit_import_stub`.
- Known class: audit-1 LNK-06 (silent branch/rel truncation on the other backends).
- Found during goal-01 review of `src/os/linux/link/**`.

## Failing Reproduction

(1) Add an exported symbol (or a tool that walks `DT_HASH`) → a by-name lookup of
the 2nd+ symbol skips/terminates early.
(2) A build whose text/data exceeds ±2 GB reach → a masked `auipc` jumps to the
wrong address.

- Observed: (1) shifted chain; (2) silent wrong branch/data target.
- Expected: (1) `chain[0]=0`, correct links; (2) an `Err` on out-of-range
  displacement.

Contrast: (1) `nbucket`/`nchain`/`bucket[0]` are correct — only the chain body is
misaligned. (2) in-range small deltas encode correctly.

## Root Cause

(1) The chain-writing loop puts the first entry at `chain[0]` and never zeroes the
null-symbol slot. (2) `riscv_hi_lo` masks to 20 bits without validating the
displacement fits the `auipc`+lo12 signed-32-bit range.

## Goal

- `DT_HASH` chain: `chain[0]=0` then N correctly-indexed entries.
- `riscv_hi_lo`: range-check the displacement and `Err` on overflow.

### Non-goals (must NOT change)

- `nbucket`/`nchain`/`bucket[0]`; in-range RISC-V encodings.

## Blast Radius

- `elf.rs:492-504` (hash chain); `mod.rs:396-400` and its four RISC-V patch callers.

## Fix Design

(1) Emit `put_u32(0)` for `chain[0]`, then `for index in 1..=N { put_u32(if index==N
{0} else {index+1}) }`.
(2) Range-check `delta` against the `auipc` reach (`~ -0x8000_0000 - 0x800 ..=
0x7fff_ffff`) and return `Err` on overflow, consistent with LNK-06.

## Phases

### Phase 1 — failing test + audit

- [ ] (1) A hash-chain unit test for N≥2 symbols asserting `chain[0]=0` and correct
      links. (2) A `riscv_hi_lo` overflow test asserting `Err`.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Fix the chain indexing; add the RISC-V displacement range check.

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh`; ELF binaries byte-identical for in-range builds;
      riscv64 hardware validation still byte-matches aarch64.

## Validation Plan

- Regression test(s): the two unit tests above.
- Full suite: `scripts/artifact-gate.sh` + riscv64 validation.

## Summary

A one-slot hash-chain shift (masked today because nothing reads our `DT_HASH`) and
an unchecked RISC-V `auipc` mask (LNK-06 class at a new site); both fixes are small
and preserve in-range byte output.
