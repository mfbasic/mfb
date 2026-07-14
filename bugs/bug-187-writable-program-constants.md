# bug-187: emitted programs place constant data in a writable segment (no read-only rodata)

Last updated: 2026-07-14
Effort: large (3h–1d)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: tests/rt-behavior/rodata_readonly_segment (readelf/otool check, to be added)

All immutable program data an emitted binary carries — string literals, constant
tables, dispatch/jump constants — is placed in a read+write segment on both
Linux and macOS. There is no read-only `.rodata`/`__const` region. An attacker
who obtains an arbitrary-write primitive in a running program can therefore
corrupt constants they should not be able to touch (rewrite a format string, a
constant used in a security check, or a dispatch table), amplifying an otherwise
contained bug. The single correct behavior a fix produces: genuinely-constant
program data lives in a read-only segment/section, with only the arena global and
truly-mutable globals remaining writable.

This is the new finding **LNK-08**, observed on freshly built binaries. See
`planning/audit-2-linker-hardening.md`.

References:

- `planning/audit-2-linker-hardening.md` (LNK-08)
- Linux: constant data sits in the single R+W `PT_LOAD` (`p_flags=6`) —
  `src/os/linux/link/elf.rs:54-67`; observed RW `PT_LOAD` at `0x403000` holds all
  non-text bytes.
- macOS: program constants populate the writable `__DATA` segment
  (`src/os/macos/link/macho.rs:152-158`, `src/os/macos/link/commands.rs:103-132`,
  `initprot/maxprot = 0x3`); only the GOT/init-pointers get read-only
  `__DATA_CONST` (with `SG_READ_ONLY`, `commands.rs:65`).

## Failing Reproduction

```
mfb build /tmp/proj            # any program with a string literal
# Linux:
readelf -l /tmp/proj/target/*/proj   # observe: only R E and RW PT_LOADs; the
                                      # RW load contains the literal bytes
# macOS:
otool -l /tmp/proj/target/*/proj | grep -A5 '__DATA '  # literals in RW __DATA
```

- Observed: constant literal bytes reside in a segment whose runtime protection
  is R+W.
- Expected: constant literal bytes reside in an R-only segment/section; a runtime
  write to them faults.

## Root Cause

The linker segregates only the GOT/init-pointers into a read-only region (macOS
`__DATA_CONST`); it has no separate read-only load for program constant data.
The design reason is that the zero-initialized main-arena global is co-located
with constants in `image.data` and must be writable — but constants and the
arena global are never separated, so the whole blob is mapped writable.

## Goal

- Immutable program data (string literals, constant tables) is placed in a
  read-only segment (Linux: a separate R-only `PT_LOAD` or folded into RELRO;
  macOS: a read-only `__DATA_CONST`/`__TEXT,__const` section), while the arena
  global and any genuinely-mutable globals stay R+W. A runtime write to a
  constant faults.

### Non-goals (must NOT change)

- The arena model / arena global's writability.
- Program observable behavior or golden outputs (other than the header/section
  layout the new test asserts).
- The `.mfp`/ABI formats.

## Blast Radius

- `src/os/linux/link/elf.rs` segment layout — split constants out of the R+W
  `PT_LOAD` into an R-only load (pairs naturally with the LNK-01/RELRO rework,
  bug-186).
- `src/os/macos/link/{macho.rs,commands.rs}` — route constant bytes to a
  read-only section instead of `__DATA`.
- The codegen data-emission layer (`image.data` construction) — must tag each
  datum as constant vs mutable so the linker can partition it.

## Fix Design

Introduce a constant/mutable partition of `image.data` at codegen time (constants
vs the arena global + mutable globals). Emit constants into a read-only
segment/section on each platform; keep the mutable partition in R+W. On Linux
this composes with the PIE/RELRO rework (bug-186) — a separate R-only `PT_LOAD`;
on macOS, a read-only `__DATA_CONST`/`__const` section (and, per LNK-10, lower
its `maxprot` to R for distributed builds). Rejected alternative: `mprotect`-ing
the region read-only at startup — leaves a writable window and a re-enable path.

## Phases

### Phase 1 — failing test + audit
- [ ] Add a build-and-inspect test asserting constant literal bytes land in an
      R-only region; confirm it fails.
- [ ] Audit `image.data` construction to classify each datum constant vs mutable.

### Phase 2 — the fix
- [ ] Partition `image.data`; emit the constant partition read-only on Linux and
      macOS; keep the arena global R+W.

### Phase 3 — validation
- [ ] Full acceptance + artifact gate green; a runtime write to a constant faults
      (proving the mapping); programs run unchanged otherwise.

## Validation Plan

- Regression test: header/section inspection asserting the R-only constant region
  on both OSes.
- Runtime proof: a crafted write to a literal address faults with SIGSEGV.
- Full suite: `scripts/test-accept.sh` + `scripts/artifact-gate.sh`.

## Summary

The real work is the codegen-side constant/mutable partition; once data is tagged,
the per-platform segment emission is mechanical. Best sequenced with the Linux
PIE/RELRO rework (bug-186) since both touch the segment layout.
