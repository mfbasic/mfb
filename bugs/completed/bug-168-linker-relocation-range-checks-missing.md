# bug-168 — aarch64/x86 linker relocation encoders silently truncate out-of-range displacements (no reach check); Mach-O section offsets narrow to u32 unguarded

Last updated: 2026-07-12
Severity: LOW — all latent (require >128 MiB text, >±4 GiB data, or >4 GiB output).
Class: Footgun / Correctness (silent truncation instead of a link error).
Status: FIXED
Resolution: `branch_imm26` now returns `Result` and reach-checks the ±128 MiB
BL/B range; new `adrp_page21` reach-checks the ±4 GiB ADRP page delta; new
`rel32` reach-checks the ±2 GiB x86 displacement — all on both the Linux and
macOS linkers, mirroring `riscv_hi_lo`. Mach-O section/linkedit/symtab/dyld_info
file-offset casts route through a `u32_field` guard (bug-88 style). All latent
(>128 MiB text / >±4 GiB data / >4 GiB image); normal builds are byte-identical
(acceptance green).

## Finding

The riscv path (`riscv_hi_lo`) errors when a relocation target is out of reach;
the aarch64 and x86 paths silently mask/truncate instead, so an out-of-range
displacement produces a wrong instruction rather than a clear link error. Sites:

- `src/os/linux/link/mod.rs:120-129, 166-175` — aarch64 `adrp` `page_delta`
  (i64 `>>12`) cast `as u32` and masked to 21 bits with no ±2^20 reach check
  (shared with elf.rs stubs via `emit_import_stub`, mod.rs:461).
- `src/os/linux/link/mod.rs:511-514` — `branch_imm26` masks `(delta/4) as i32`
  to 26 bits with no ±128 MiB check (BL out of range → wrapped branch).
- `src/os/linux/link/mod.rs:200-205, 208-219, 225-242` — x86 `call_pc32`/
  `data_pc32`/`got_pc32` narrow the displacement to i32 with no range check
  (>±2 GiB → wrapped rel32).
- `src/os/macos/link/mod.rs:491-494` (branch_imm26) and `:220-229, 264-273`
  (adrp page21) — same as the Linux aarch64 cases.
- `src/os/macos/link/commands.rs:215` (+`:229, :235-237, :297-305`) — Mach-O
  section/linkedit/symtab/dyld_info file offsets `as u32` with no
  bug-88-style `u32::try_from` guard (>4 GiB image → truncated offsets). The
  code-signature path already has the `u32_field` guard (commands.rs:502); these
  offset casts do not.

## Trigger

A build whose text section exceeds ~128 MiB (BL), whose data/GOT sits >±4 GiB
from an `adrp`/lea site, or whose output executable exceeds 4 GiB. Realistically
unreachable for normal programs, so all LOW/latent — but the discipline is
asymmetric (riscv errors, aarch64/x86 wrap).

## Fix

Range-check each displacement against its encodable reach before masking and
return `Err` on overflow (mirror `riscv_hi_lo`); route the Mach-O offset casts
through a `u32::try_from(...)` that errors, matching the code-signature
`u32_field` helper.
