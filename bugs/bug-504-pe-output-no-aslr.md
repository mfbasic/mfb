# bug-504: emitted Windows PE binaries have no ASLR (RELOCS_STRIPPED, no .reloc, DYNAMIC_BASE clear)

Last updated: 2026-09-03
Effort: medium (3h–1d — needs a `.reloc` section)
Severity: HIGH
Class: security (missing exploit mitigation in emitted binaries)

Status: Open (found in audit-3, Surface 7 LNK-13; code-verified by the lead, not executed on Windows)

Regression Test: a PE-header assertion in the Windows link tests that `DllCharacteristics` has `DYNAMIC_BASE|NX_COMPAT|HIGH_ENTROPY_VA` and that `RELOCS_STRIPPED` is clear with a non-empty `BASERELOC` directory.

## Summary

Every Windows binary the compiler emits loads at the fixed preferred base
`0x140000000` on every run on every machine — no address-space layout
randomization at all. This is the Windows analog of bug-186 (non-PIE Linux, fixed)
and leaves code, constant data, the IAT, and writable globals at run-invariant
addresses for ROP / IAT-overwrite once any in-program memory-safety bug is found.

## Mechanism

Three independent reasons ASLR cannot happen, all in the PE header writer:

```rust
// src/os/windows/link/pe.rs:189-191 — image declares itself non-relocatable
w.u16(IMAGE_FILE_EXECUTABLE_IMAGE | IMAGE_FILE_LARGE_ADDRESS_AWARE | IMAGE_FILE_RELOCS_STRIPPED);
// src/os/windows/link/pe.rs:221 — no opt-in
w.u16(0x0100 | 0x8000); // NX_COMPAT | TERMINAL_SERVER_AWARE (DYNAMIC_BASE clear)
// src/os/windows/link/pe.rs:15 — fixed base, no .reloc
pub(super) const IMAGE_BASE: u64 = 0x0001_4000_0000;
```

`IMAGE_FILE_RELOCS_STRIPPED` alone is decisive: the loader treats it as "must load
at the preferred base" and ignores `DYNAMIC_BASE` even if set. There is no
`.reloc` section and `DIR[5] BASERELOC` is `(0,0)`. Lead-verified all three lines
in current source; `DllCharacteristics = 0x8100` (no `DYNAMIC_BASE=0x40`, no
`HIGH_ENTROPY_VA=0x20`).

## Reproduction

Code-verified (the header bytes are unambiguous). Not executed on Windows (needs
box 2230); the agent parsed a freshly emitted PE header showing
`chars=0x0023` (RELOCS_STRIPPED set) and `DllCharacteristics=0x8100`.

## Best fix

Emit a `.reloc` section listing the image's absolute address fixups, clear
`IMAGE_FILE_RELOCS_STRIPPED`, populate `DIR[5] BASERELOC`, and set
`DYNAMIC_BASE | HIGH_ENTROPY_VA` (and, ideally, `GUARD_CF` with a load-config —
see LNK-14) in `DllCharacteristics`. If the emitted code is already
position-independent (0 absolute pointers), the `.reloc` section is empty/tiny and
the change is close to free.

## Non-goals

Do not break the fixed-base assumption anywhere the runtime hard-codes an address
without a relocation; keep NX_COMPAT.

## Prior art

bug-186 (`bugs/completed/`) fixed the identical class for Linux (non-PIE). The PE
target has had no prior security audit. Related: LNK-14 (no
`IMAGE_LOAD_CONFIG_DIRECTORY` → no CFG / `/GS` cookie), MEDIUM, a companion
hardening gap on the same target.
