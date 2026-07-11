# bug-38: Static aarch64/riscv64 ELF appends `data` unaligned, but relocations are patched against a page-aligned `data_vmaddr` → wrong data pointers

Last updated: 2026-07-08
Effort: small (<1h)

`src/os/linux/link/elf.rs::encode_static_elf` (the aarch64/riscv64 static,
no-import path; `e_machine = 183` = EM_AARCH64) lays out the file as
`resize(TEXT_FILE_OFFSET); extend(text); extend(data)` (`:40-42`) — `data` starts
at file offset `TEXT_FILE_OFFSET + text.len()` with **no page alignment** — and its
single PT_LOAD maps the file linearly from offset 0 (`p_filesz = p_memsz =
TEXT_FILE_OFFSET + text.len() + data.len()`, `:9,36-37`). So a data byte's real
runtime address is `IMAGE_BASE + TEXT_FILE_OFFSET + text.len()`.

But the shared relocation patcher computes every data symbol's address as
**page-aligned**: `data_vmaddr = IMAGE_BASE + align(TEXT_FILE_OFFSET + text.len(),
PAGE_SIZE)` (`src/os/linux/link/mod.rs:47`, symbol vmaddr for Data at `:117-141`).
Unless `TEXT_FILE_OFFSET + text.len()` is already a multiple of `PAGE_SIZE`, every
`page21`/`pageoff12` (aarch64) or `pcrel` (riscv64) relocation to `data` is off by
`align(...) - (TEXT_FILE_OFFSET + text.len())` bytes → every string/constant
pointer is wrong → garbage reads or SIGSEGV.

The x86 static path does **not** have this bug: `encode_static_elf_x86` page-aligns
data (`data_offset = align(text_offset + text.len(), PAGE_SIZE)`, `elf.rs:64,109`)
and its comment explicitly says it does so to match `write_executable`'s
`data_vmaddr`. The dynamic ELF path also page-aligns. The aarch64/riscv static path
is the lone outlier.

The single correct behavior a fix produces: on the static aarch64/riscv path,
`data`'s file offset (and PT_LOAD sizing) is page-aligned to exactly the address the
relocation patcher uses, so data pointers resolve correctly.

Severity MEDIUM: a definite layout bug producing wrong data pointers, but **latent
reachability** — it needs a no-libc-import aarch64/riscv64 build (static path) that
also carries referenced constant `data`. In practice aarch64/riscv64 console builds
dynamically link libc, routing to the correct (page-aligning) dynamic path; the
generator's emission of the static-path-with-data combination could not be
confirmed. If the path is hit, it is a hard correctness failure.

References:

- `src/os/linux/link/elf.rs:9` (`file_size` no align), `:40-42` (data appended
  unaligned), `:36-37` (single PT_LOAD filesz/memsz).
- Contrast (correct): `encode_static_elf_x86` (`:56-65,109`, page-aligns data with
  a comment tying it to `data_vmaddr`); dynamic ELF path (`data_offset` page-aligned).
- `src/os/linux/link/mod.rs:47` (`data_vmaddr` page-aligned), `:117-141` (Data
  symbol vmaddr).
- Found during goal-01 review of `src/os/linux/link/**`.

## Failing Reproduction

A no-import aarch64 (or riscv64) build carrying a referenced rodata constant, where
`TEXT_FILE_OFFSET + text.len()` is not page-aligned:

- Observed: a data relocation resolves to `IMAGE_BASE + align(TEXT_FILE_OFFSET +
  text.len(), PAGE_SIZE)`, but the constant actually lives at
  `IMAGE_BASE + TEXT_FILE_OFFSET + text.len()` → the pointer is off by the padding
  delta → wrong reads / SIGSEGV.
- Expected: the data pointer resolves to the constant's real address.

Contrast: the x86 static path and all dynamic paths align data and are correct.

## Root Cause

`encode_static_elf` appends `data` without the page alignment that the shared
`data_vmaddr` relocation formula assumes. The two disagree whenever text does not
end on a page boundary.

## Goal

- Static aarch64/riscv64 ELF places `data` at the page-aligned offset the
  relocation patcher uses, and sizes PT_LOAD accordingly.

### Non-goals (must NOT change)

- The x86 static and dynamic paths (already correct).
- The relocation `data_vmaddr` formula (it is the intended layout).

## Blast Radius

- `encode_static_elf` (`elf.rs:40-42`) only. Mirror `encode_static_elf_x86`'s
  alignment.

## Fix Design

Before appending `data`, `bytes.resize(align(TEXT_FILE_OFFSET + text.len(),
PAGE_SIZE), 0)`, and compute `p_filesz`/`p_memsz` from that aligned data offset plus
`data.len()` — exactly mirroring `encode_static_elf_x86`. (If a separate writable
data PT_LOAD is warranted, match the x86 two-segment layout.)

## Phases

### Phase 1 — failing test + audit

- [ ] Confirm whether the generator emits a static aarch64/riscv64 build with
      referenced data (determines live vs. latent). Add a linker test asserting the
      data symbol's vmaddr equals the relocation `data_vmaddr` for a non-page-aligned
      text length.
- [x] Code-level confirmation of the asymmetry complete (above).

### Phase 2 — the fix

- [ ] Page-align data in `encode_static_elf` and size PT_LOAD from it.

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh`; aarch64/riscv64 runtime validation of a
      static-path build with a rodata constant (byte-identical to the working path).

## Validation Plan

- Regression test(s): the vmaddr-equals-reloc-address linker test.
- Runtime proof: a static aarch64/riscv64 program that reads a constant string
  prints it correctly (or confirm the path is unreachable and downgrade).
- Full suite: `scripts/artifact-gate.sh`.

## Summary

The aarch64/riscv static ELF path skips the data page-alignment its own relocation
patcher assumes; mirroring the x86 path fixes the data-pointer offset. Latent until
the static-with-data path is confirmed emitted.
