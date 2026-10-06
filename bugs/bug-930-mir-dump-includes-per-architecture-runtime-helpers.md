# bug-930: the `.mir` dump documented as "target-neutral" contains the runtime helpers, which are hand-written per architecture with physical register names

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Other (artifact contract; owner rule recorded)

Status: Open
Regression Test: tests for the chosen contract (e.g. a `-mir` golden that contains no physical register names, or one that labels helper functions)

Split out of bug-917 item 9 at the owner's request. The owner is still deciding the
direction.

`mfb spec architecture artifacts` (`08_artifacts.md`) lists `<name>.mir` as "Target-neutral
MIR dump". The code lowered from MFBASIC source **is** neutral in that dump. The runtime
helper functions included in the same file are not.

## Failing Reproduction

`mfb init mr && cd mr && mfb build --mir` (`/tmp/hardening/coord/mr`, binary
`/tmp/hardening/mfb-head`), then again with `--target linux-x86_64` and `--target linux-riscv64`.
Writes `./mr.mir`. Operand classification of the macOS AArch64 dump, per function:

| function | operands |
| --- | --- |
| `main` (user code) | 6 virtual registers (`%vN`), 13 neutral ABI tokens (`%argMFB0`, `%retMFB0..3`, `%argC0..2`), `sp` only |
| `program.entry` | 174 physical, 18 `arena_base` |
| `runtime.arena_alloc` | 710 physical, 31 `arena_base` |
| `runtime.io.print` | 316 physical, 11 `arena_base` |
| `runtime.parkError` | 169 vreg, 57 physical, ABI tokens |

Most frequent register operands across the whole dump:

- macOS AArch64: `x8` 407, `sp` 284, `x9` 235, `x10` 195, `x11` 184, `x0` 164 …
- x86-64: `r10` 900, `rsp` 618, `r11` 491, `r12` 375, `r14` 297, `rdi` 210 …
- riscv64: `t3` 437, `sp` 321, `s1` 288, `t4` 233, `t5` 203, `t6` 184 …

- Observed: a dump labelled target-neutral that differs per target and names each ISA's registers.
- Expected: whatever the owner decides the `.mir` contract is (below).

## Root Cause

The capture point is neutral: `src/codegen/engine/regalloc/builder_registers.rs` calls
`mir::capture_function(.., mir::lower_to_mir(..))` on "the fully-lowered, pre-allocation
stream … with virtual registers intact", behind the zero-physical-register invariant
(`find_physical_operand`, plan-34-D). The runtime helpers (`runtime.*`, `program.entry`) are
hand-written separately for each backend with physical registers, and they are captured
into the same dump. Physical-register strings reach the stream as `Operand::Raw` (bug-946).

## Root cause, refined (coordinator, 2026-10-05)

The shared runtime helpers are **not** hand-written per architecture. `runtime.arena_alloc`
(`src/codegen/memory/arena/arena.rs`) is built with virtual registers (`vregs.next()`) and
neutral ABI tokens (`abi::c_arg`, `abi::return_register`, `abi::mfb_return`). It shows physical
registers in `.mir` because of **when** it is captured:

- `finalize_vreg_helper` (`src/codegen/engine/util/vreg_frame.rs`) runs the register
  allocator as soon as the helper is built.
- Only functions lowered through the builder are captured pre-allocation
  (`src/codegen/engine/regalloc/builder_registers.rs`, `mir::capture_function`).
- `mir::build_mir_plan` (`src/codegen/engine/mir/mir.rs`) falls back to
  `lower_to_mir(&function.instructions)` for every uncaptured function. That is the final,
  allocated (and on x86-64, selected) stream, so it has physical names.

Only ~10 non-test files under `src/codegen/` spell an AArch64 register literally (mostly the
MIR/operand/regalloc plumbing itself, `grep -rlE '"x[0-9]{1,2}"' src/codegen`). Truly
target-specific code lives under `src/arch/` and `src/target/`.

**Owner rule (2026-10-05):** a helper that is target-specific (in `src/target/` or `src/arch/`,
not `src/codegen/`) stays target-specific. So the fix is:

- Capture shared `src/codegen/` helpers (the `finalize_vreg_helper` family) **before**
  allocation, so they appear in `.mir` with virtual registers and neutral tokens like user code.
- Helpers that come from `src/target/` / `src/arch/` keep their physical registers and are
  labelled per-target in the dump. `08_artifacts.md` says ".mir is target-neutral except
  functions marked target-specific".
- `build_mir_plan`'s silent `lower_to_mir(&function.instructions)` fallback is replaced by
  that explicit split, so an uncaptured shared helper is a bug, not a silent physical dump.

This is not a port of the helpers. They are already neutral at the source.

## Options considered (superseded by the owner rule above)

1. Exclude the runtime helpers from `.mir`, or emit them in a clearly labelled
   per-target section, and say so in `08_artifacts.md`. Small.
2. Port the runtime helpers to neutral MIR (virtual registers + neutral ABI tokens), so the
   whole dump is target-neutral. Huge. Overlaps bug-946.
3. Re-describe `.mir` as "neutral for program code; runtime helpers are per-target" with
   no code change. Smallest.

## Non-goals

- Emitted machine code must not change under any option.
