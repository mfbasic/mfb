# bug-186: Linux binaries are non-PIE (ET_EXEC at fixed 0x400000) → main-image ASLR defeated

Last updated: 2026-07-14
Effort: x-large (1d–3d)
Severity: HIGH
Class: Security

Status: Fixed (dynamic path — every real program; PT_GNU_RELRO deferred to bug-187)
Regression Test: tests/linux_pie_headers.rs

Every executable the linker emits for Linux (aarch64, x86_64, riscv64) is
`ET_EXEC` with a fixed load base of `0x400000`. The main image — code, constant
data, GOT, `.dynamic` — therefore loads at the same virtual addresses on every
run, with no ASLR slide. Any information leak or memory-corruption primitive in
an emitted program (and this platform has a live class of size-arithmetic /
free-list hazards) gets exact, run-invariant code/data/GOT addresses for ROP or
GOT overwrite, an advantage a PIE (`ET_DYN`) image would deny. macOS output is
already PIE (`MH_PIE`); Linux is the outlier. The single correct behavior a fix
produces: Linux executables load as position-independent (`ET_DYN`) with a
randomized base, self-relocating at startup, with identical runtime behavior.

This is the still-open audit-1 finding **LNK-01**, re-verified against current
code and observed on freshly built binaries (`readelf -h` → `Type: EXEC`,
`Entry point 0x401000`). See `planning/audit-2-linker-hardening.md`.

References:

- `planning/audit-2-linker-hardening.md` (LNK-01), `planning/old-plans/audit-1-linker-hardening.md`
- `src/os/linux/link/mod.rs:7` — `IMAGE_BASE = 0x400000`.
- `src/os/linux/link/elf.rs:30` (static aarch64/riscv `e_type = ET_EXEC`), `:98`
  (static x86), `:168` (dynamic), entry `e_entry = text_vmaddr + entry`.
- macOS PIE reference: `src/os/macos/link/macho.rs:72` (`MH_PIE` in the header flags).

## Failing Reproduction

```
mfb init /tmp/pieproj
# minimal program
mfb build --target linux-arm64 /tmp/pieproj   # or build natively on Linux
readelf -h /tmp/pieproj/target/*/pieproj | grep -E 'Type|Entry'
```

- Observed: `Type: EXEC (Executable file)`, `Entry point address: 0x401000`;
  across runs on Linux the main image maps at `0x400000` every time (no slide).
- Expected: `Type: DYN (Position-Independent Executable file)` and a randomized
  main-image base per run.

Contrast: `otool -hv` on a macOS build shows `PIE` in the header flags — the
macOS path is already position-independent.

## Root Cause

The Linux ELF writer hardcodes an `ET_EXEC` type and an absolute `IMAGE_BASE`:

- `src/os/linux/link/elf.rs:30,98,168` set `e_type = 2` (`ET_EXEC`).
- `src/os/linux/link/mod.rs:7` sets `IMAGE_BASE = 0x400000`; `text_vmaddr`,
  `data_vmaddr`, and `got_vmaddr` are all `IMAGE_BASE + offset` absolutes
  (`mod.rs:34,48,344`), and relocations are patched to those absolute addresses.

An `ET_EXEC` image with absolute vaddrs cannot be slid by the loader, so the main
image is exempt from ASLR (only shared libs / mmap get randomized).

## Goal

- Linux executables are emitted as `ET_DYN` PIE with a load base of 0, a
  `PT_PHDR`, and self-relocation at startup (`R_*_RELATIVE` entries applied by a
  static-PIE-style startup, or `_dl_relocate_static_pie` equivalent), producing a
  randomized main-image base with unchanged observable runtime behavior.

### Non-goals (must NOT change)

- macOS / iOS output (already PIE).
- The dynamic-linking model, symbol resolution, or the `.mfp`/ABI formats — only
  the ELF type, base, and startup relocation change.
- Observable program behavior, exit codes, or golden outputs (other than the ELF
  header/`readelf` check the new test asserts).

## Blast Radius

- `src/os/linux/link/elf.rs` (static + dynamic writers, all three arches share
  `encode_static_elf`/`encode_dynamic_elf`) — fixed by this bug.
- `src/os/linux/link/mod.rs` absolute-vaddr / relocation patching — must become
  base-relative + emit `R_*_RELATIVE` self-relocations.
- riscv64 shares the same writer (only `e_machine`/`e_flags` differ) — covered by
  the same change.
- macOS writer — unaffected.

## Fix Design

Convert the Linux writer to emit `ET_DYN`: load base 0, add a `PT_PHDR`, and for
every absolute address the code/data currently bakes in, emit a corresponding
`R_AARCH64_RELATIVE` / `R_X86_64_RELATIVE` / `R_RISCV_RELATIVE` dynamic
relocation (or a `DT_RELA` array) that a minimal self-relocating startup applies
before `main`. The static case needs a `_dl_relocate_static_pie`-style bootstrap;
the dynamic case leans on the loader. This is the canonical, well-understood PIE
conversion — large but mechanical — and it also unlocks RELRO (LNK-03 Linux) and
naturally pairs with `PT_GNU_STACK` (LNK-02). Rejected alternative: keeping
`ET_EXEC` and randomizing `IMAGE_BASE` at build time — gives per-binary but not
per-run randomization; not real ASLR.

## Phases

### Phase 1 — failing test + audit
- [x] Added `tests/linux_pie_headers.rs` (build + inspect `e_type`/entry/PT_GNU_STACK
      on all three arches); confirmed it failed against the `ET_EXEC` build.
- [x] Audit result — **no self-relocation needed for the common case**: every
      emitted code reference is already PC-relative (`adrp`/`add`, `auipc`+`addi`,
      `%rip`-relative `lea`), and `push_symbol_address` materializes even internal
      symbol addresses PC-relatively, so `image.data` holds *no* absolute internal
      pointers. Only the loader-relocated metadata (GOT, `.rela` r_offsets, `DT_*`
      pointers) needs the base — the dynamic loader applies that itself. The one
      absolute internal pointer, `DT_INIT_ARRAY`, gets `R_*_RELATIVE` relocs.

### Phase 2 — the fix
- [x] `encode_dynamic_elf`: `ET_DYN`, load base 0 (`DYN_IMAGE_BASE`), and one
      `R_AARCH64/X86_64/RISCV_RELATIVE` per `DT_INIT_ARRAY` slot (appended to
      `.rela.dyn`; `DT_JMPREL`/`DT_PLTRELSZ` stay the import prefix). Every real
      program links libc, so it takes this dynamic path; the loader applies the
      slide. (The import-less `encode_static_elf` path is unreachable for real
      builds — even a pure-arithmetic program links the libc runtime entry — so it
      is left `ET_EXEC`; only the hand-built linker unit tests exercise it.)
- [x] Folded in `PT_GNU_STACK` (NX stack, LNK-02). `PT_GNU_RELRO` (LNK-03) is
      deferred to compose with bug-187's const/mutable data partition, which
      page-isolates the GOT from the writable arena global (the const/reloc-type
      scaffolding is already in place).

### Phase 3 — validation
- [x] Linker unit suite (55) + `tests/linux_pie_headers.rs` (3) green. Built and
      **ran** on Linux aarch64 (glibc, box 2223), x86_64 (glibc box 2228 + musl
      box 2227), and riscv64 (musl box 2229): simple, rich (imports/strings/math/
      collections/globals), and pure-arithmetic programs all run correctly
      (`readelf`: `Type: DYN`). ASLR proven — the main-image base is randomized
      across runs (`/proc/self/maps`: `aaaacab1c000`, `aaaac4b87000`,
      `aaaab3636000`) where the `ET_EXEC` build was a fixed `0x400000`.

## Validation Plan

- Regression test: `readelf -h` asserts `Type: DYN`; a run-twice check confirms
  the base differs across runs (ASLR active).
- Runtime proof: a representative program runs identically to the pre-change
  build on all three Linux arches.
- Full suite: `scripts/test-accept.sh` + `scripts/artifact-gate.sh`, plus
  hardware/box validation per `.ai/remote_systems.md`.

## Summary

This is the single largest hardening item on the linker surface and the highest
engineering risk (startup self-relocation must be exactly right or every binary
segfaults before `main`). It is isolated to the Linux writer and unlocks the two
adjacent MEDIUM Linux findings (RELRO, GNU_STACK) in the same rework.
