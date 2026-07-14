# bug-192: x86-64 `adrp` emits `lea` for GOT imports instead of a mov-from-GOT → os::environ off by one indirection

Last updated: 2026-07-14
Effort: medium (1h–2h)
Severity: HIGH
Class: correctness (platform: linux-x86_64)

Status: Open
Regression Test: tests/rt-behavior/ (linux-x86_64 os::environ / getEnv returns correct value)

On linux-x86_64 the neutral `adrp` op is always encoded as `lea dst,[rip+disp32]`
(opcode `0x8D`), even when the target is an imported (GOT) symbol that requires a
dereferencing `mov dst,[rip+disp32]` (REX.W `0x8B`) GOT-load. The imported data
address therefore comes out one dereference short.

The only imported-data symbol today is `os.environ`
(`src/target/linux_x86_64/plan.rs:121`, `code.rs:454`). `record_reloc` routes it
to `got_pc32`/external (`emitter.rs:147`) and the linker points the disp32 at the
GOT slot (`link/mod.rs:219-229`). With `lea`, `dst = &GOTslot`; the helper's
single following `load_u64` (`code.rs:465`) yields `*(GOTslot) = &environ`, i.e.
a `char***` where a `char**` is required → wrong data or SIGSEGV. The spec
(`src/arch/x86_64/reloc.rs:14`, `docs/spec/linker/08_linux-x86_64.md:78`) states
GotLoad must be `mov reg,[rip] GOT`, but the encoder has no such path.

## Failing Reproduction

Build a linux-x86_64 program that reads the environment
(`os::environ`/`os::getEnv`) and run it. Observed: garbage / crash (pointer one
indirection too high). Expected: correct environment access, as on aarch64/riscv64.
Latent because the x86 phase-1 focus was integer programs, so this path is not
yet runtime-exercised.

## Root Cause

`src/arch/x86_64/encode/emitter.rs:661-679` — the `"adrp"` arm unconditionally
emits `lea` (0x8D) and has no import/GOT-aware `mov`-from-GOT (0x8B) form.

## Non-goals

- Do not change the non-import `adrp` (PC-relative address) behavior — that must
  stay `lea`.
- Do not alter aarch64/riscv64, which already dereference the GOT correctly.

## Blast Radius

- Only imported-data addresses on x86-64 (today: `os.environ`). All other x86
  `adrp` uses are non-import and correctly `lea`.

## Fix Design

In the `adrp` arm, when the target resolves as an import (GOT), emit
`mov dst,[rip+disp32]` (REX.W 0x8B) instead of `lea`; or split the neutral op so
`GotLoad` selects a mov-from-GOT form. Reconcile the reloc.rs/spec comments with
the emitted bytes. Add a linux-x86_64 runtime test exercising `os::environ`.
