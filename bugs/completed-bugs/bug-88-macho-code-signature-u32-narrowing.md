# bug-88 — Mach-O code-signature emitter narrows size fields to u32 unchecked

**Status:** OPEN (latent). Filed 2026-07-10 (goal-02 review, G5).
**Severity:** LOW — not reachable today; defense-in-depth.
**Class:** footgun / silent truncation. Extends the audit-1-linker-hardening
LNK-06 narrowing-cast class to the signature emitter.

## Finding

`src/os/macos/link/macho.rs` — `code_signature` (lines ~490–519) casts
`unsigned.len()` (the `codeLimit`), the hash-slot `page_count`, and the
superblob / CodeDirectory blob lengths to `u32` with no range guard:

- `macho.rs:490,499` — `unsigned.len() as u32` → `codeLimit`
- `macho.rs:504` — `page_count as u32` → `nCodeSlots`
- `macho.rs:510-511,517,519` — `superblob_len as u32`, CD length fields

An output image ≥ 4 GiB would silently truncate `codeLimit` / the hash-slot
count and emit a structurally invalid or under-covering ad-hoc signature with
no build error — the binary would be killed by the kernel at exec (or worse,
carry a signature that doesn't cover the tail of the image).

## Trigger

Not constructible today: emitted images are far below 4 GiB. Latent until
image size can exceed `u32`. Rank LOW; fix is a cheap
`u32::try_from(...).expect("image exceeds code-signature limits")`-style guard
(or a proper build diagnostic) at the four cast sites.

## Prior art

Same class as `audit-1-linker-hardening.md` LNK-06 (relocation value math
truncation, still open — see also bug-39 for the fixed RISC-V `riscv_hi_lo`
half). This site was not covered by LNK-06's census of relocation patchers.
