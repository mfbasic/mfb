# bug-263: Linux binaries emit no `PT_GNU_RELRO`; GOT/`.dynamic` stay writable despite `DF_BIND_NOW`

Last updated: 2026-07-17
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: (none yet)

Emitted Linux executables set `DT_FLAGS = DF_BIND_NOW` (all relocations resolved
at startup) but never emit a `PT_GNU_RELRO` program header, and the GOT and
`.dynamic` sit in the writable `PT_LOAD`. So after the loader finishes binding,
the GOT remains writable for the life of the process — a GOT-overwrite target an
attacker with an arbitrary-write primitive can use to hijack control flow, the
exact hazard RELRO exists to remove. `BIND_NOW` without RELRO buys the ordering
but not the protection. macOS already gets this via `__DATA_CONST` +
`SG_READ_ONLY` (bug-187); Linux is the outlier. The single correct behavior a fix
produces: after startup relocation, the GOT/`.dynamic` region is mapped read-only.

References:

- `planning/audit-2-linker-hardening.md` (LNK-03, Linux half — the macOS half is
  fixed via `SG_READ_ONLY`).
- bug-186 (PIE) explicitly deferred RELRO: `src/os/linux/link/elf.rs:345-347`
  ("PT_GNU_RELRO is deferred to compose with the bug-187 const/mutable data
  partition").
- `src/os/linux/link/elf.rs:632` — `DT_FLAGS = DF_BIND_NOW` is set.
- bug-187 delivered read-only *rodata* but did not add `PT_GNU_RELRO` over the
  GOT/dynamic region; the deferral note at `elf.rs:345` is still live.

## Failing Reproduction

```
mfb build /tmp/proj          # native Linux, or --target linux-*
readelf -l /tmp/proj/target/*/proj | grep -E 'GNU_RELRO|GNU_STACK'
```

- Observed: a `GNU_STACK` line (bug-224) but **no** `GNU_RELRO`; the GOT lives in
  the `RW` `PT_LOAD` and is writable at runtime.
- Expected: a `GNU_RELRO` segment covering the GOT/`.dynamic`, so
  `readelf -l` shows `GNU_RELRO` and the region is read-only post-relocation.

Contrast: macOS output already protects the GOT (`__DATA_CONST` `SG_READ_ONLY`,
bug-187); only Linux is exposed.

## Root Cause

The dynamic ELF encoder places the GOT and `.dynamic` in the single writable data
`PT_LOAD` and never emits a `PT_GNU_RELRO` header
(`src/os/linux/link/elf.rs`), so the loader has nothing to `mprotect` back to
read-only after `BIND_NOW` completes. The RELRO segment was consciously deferred
from bug-186 to compose with a GOT/arena-global page partition.

## Goal

- The dynamic Linux ELF (aarch64/x86_64/riscv64) emits a `PT_GNU_RELRO` program
  header covering the GOT and `.dynamic`, page-isolated from the mutable arena
  global, so those bytes are read-only after startup binding. `readelf -l` shows
  `GNU_RELRO`; a write to a GOT slot post-init faults.

### Non-goals (must NOT change)

- The arena model or the mutable-global layout (must remain writable).
- `DF_BIND_NOW` (RELRO relies on it — keep it).
- macOS output (already correct).

## Fix Design

Page-align the GOT/`.dynamic` block and separate it from the arena/mutable
globals so RELRO can cover the former without freezing the latter, then emit a
`PT_GNU_RELRO` (type `0x6474e552`) phdr over it and bump `e_phnum`. This composes
with the bug-187 const/mutable partition already in place — the remaining work is
the page-isolation of GOT vs arena global and the phdr itself. Apply to all three
arches (they share `encode_dynamic_elf`).
