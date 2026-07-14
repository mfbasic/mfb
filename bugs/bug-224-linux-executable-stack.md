# bug-224: emitted Linux executables have no PT_GNU_STACK → kernel marks the stack executable (RWX)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: security (defense-in-depth)

Status: Open

The ELF encoder emits no `PT_GNU_STACK` program header, so the loader falls back
to an executable stack (RWX) for every `mfb build` Linux binary.

Trigger: any Linux build (static `e_phnum=2`, dynamic `e_phnum=5`) →
`readelf -l` shows no `GNU_STACK` entry; the stack is mapped executable, removing
a standard exploit-mitigation barrier.

Root cause: `src/os/linux/link/elf.rs:39,107,181` (`e_phnum` / program-header
construction) never add a `PT_GNU_STACK` header.

Fix: add a `PT_GNU_STACK` program header (type `0x6474e551`, flags `R|W = 6`, all
sizes 0) to both the static and dynamic encoders and bump `e_phnum` accordingly.
