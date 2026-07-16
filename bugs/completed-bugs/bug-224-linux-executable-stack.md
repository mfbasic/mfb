# bug-224: emitted Linux executables have no PT_GNU_STACK → kernel marks the stack executable (RWX)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: security (defense-in-depth)

Status: Fixed (2026-07-15) — both static Linux ELF encoders (encode_static_elf for aarch64/riscv64 and encode_static_elf_x86) now emit a PT_GNU_STACK program header (type 0x6474e551, flags R|W=6, all sizes 0) and bump e_phnum 2->3, so the loader no longer falls back to an executable (RWX) stack. The dynamic encoder already emitted it (bug-186/LNK-02). Regression Test: the encode_static_elf_x86 and encode_static_elf unit tests now assert the PT_GNU_STACK phdr; all 29 linux link tests pass.

The ELF encoder emits no `PT_GNU_STACK` program header, so the loader falls back
to an executable stack (RWX) for every `mfb build` Linux binary.

Trigger: any Linux build (static `e_phnum=2`, dynamic `e_phnum=5`) →
`readelf -l` shows no `GNU_STACK` entry; the stack is mapped executable, removing
a standard exploit-mitigation barrier.

Root cause: `src/os/linux/link/elf.rs:39,107,181` (`e_phnum` / program-header
construction) never add a `PT_GNU_STACK` header.

Fix: add a `PT_GNU_STACK` program header (type `0x6474e551`, flags `R|W = 6`, all
sizes 0) to both the static and dynamic encoders and bump `e_phnum` accordingly.
