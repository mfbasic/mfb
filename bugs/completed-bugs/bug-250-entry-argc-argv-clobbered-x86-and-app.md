# bug-250: every arg-accepting program SIGSEGVs on linux-x86_64, and in app mode on every Linux arch

Last updated: 2026-07-16
Effort: small (<1h)
Severity: HIGH
Class: correctness (crash on valid source; native codegen)

Status: FIXED (2026-07-16).

Found while runtime-verifying bug-240's argv plumbing on the Debian x86-64 GTK
box (VM 2228). Two independent root causes, both in the shared program entry,
both making an arg-accepting entry (`FUNC main(args AS List OF String)`) read a
bogus `argv` and fault while walking it.

## Failing Reproduction

```mfb
IMPORT io
FUNC main(args AS List OF String) AS Integer
  io::print("argc=" & toString(len(args)))
  FOR EACH a IN args
    io::print("arg: " & a)
  NEXT
  RETURN 0
END FUNC
```

`mfb build -target=linux-x86_64 <project>` then run with any arguments:

- Observed: `Segmentation fault` (exit 139), no output at all.
- Expected: `argc=3` / `arg: <exe>` / `arg: alpha` / `arg: beta`.

Measured before the fix (identical program, one build per target):

| Target | VM | Result |
| --- | --- | --- |
| linux-x86_64 glibc | 2228 Debian | SIGSEGV ✗ |
| linux-x86_64 musl | 2227 Alpine | SIGSEGV ✗ |
| linux-aarch64 glibc | 2223 Kali | works ✓ |
| linux-riscv64 musl | 2229 Alpine | works ✓ |
| macos-aarch64 | host | works ✓ |

## Root Cause A — `SCRATCH[1]` and `ARG[1]` are the same register on x86-64

`lower_program_entry` materializes argc/argv into `ARG[0]`/`ARG[1]`, then zeroes
the arena state with a loop whose end pointer is `SCRATCH[1]`
(`entry_and_arena.rs`). The comment there asserted the sequence was safe:

> `x9`/`x10` are free scratch here; `x0`/`x1` (argc/argv) are live.

True on AArch64, where `SCRATCH[1]` = x10 and `ARG[1]` = x1. False on x86-64,
where the two neutral tokens realize to the **same physical register**:

- `SCRATCH[1]` is x10 → `map_scratch_register(10)` → `(10-9) % 11` = 1 → **rsi**
- `ARG[1]` → `CALL_ARGS[1]` → **rsi**

So the zero loop destroyed argv two instructions after it was loaded, and the
entry then walked the arena-state address as a `char**`. Emitted x86-64 `_main`
before the fix:

```
[1] ldr_u64 dst=rdi base=rsp offset=0     ; argc -> ARG[0]
[2] add_imm dst=rsi src=rsp imm=8         ; argv -> ARG[1] = rsi
[6] add_imm dst=rsi src=r15 imm=3768      ; SCRATCH[1] = rsi  <-- destroys argv
```

The park into callee-saved `SCRATCH[17]`/`SCRATCH[18]` (x27/x28 → r12/r13) that
would have saved them ran ~20 instructions later, long after the clobber.

Same family as bug-85 (a neutral ABI token aliasing differently per ISA). Fix:
park argc/argv immediately, before anything can touch them. Gated on
`language_entry_accepts_args`, so non-arg entries stay byte-identical.

## Root Cause B — an app-mode entry read args off the worker stack

`entry_args_in_registers()` is false for every Linux backend, meaning "the raw
ELF entry is jumped to with argc at `[sp]` and argv at `sp+8`". That is right for
a console build, where the entry IS the process entry point — but in app mode the
toolkit bootstrap owns `_main` and the entry body runs under
`MACAPP_PROGRAM_SYMBOL`, **called as an ordinary function from the worker
thread**. A worker stack carries no kernel argv layout, so the entry loaded
garbage over whatever the caller passed in registers.

This broke app-mode args on every Linux arch, not just x86-64, and would have
silently defeated bug-240's plumbing even after Root Cause A was fixed.

Fix: `ProgramEntrySpec::entry_called_as_function` (true only in app mode). The
entry now uses `platform.entry_args_in_registers() || entry_called_as_function`
to decide, so a called entry takes args from registers on every platform.

## Why it survived this long

No test exercised an arg-accepting entry's native codegen or runtime. The only
two arg-accepting-`main` fixtures —
`tests/rt-error/project/project-entry-{func,sub}-args-*` — run `-ast -ir` only
and `FAIL` on their first statement, so they never reach codegen, let alone
argv. Moving two instructions in every arg-accepting entry churned **zero**
goldens across all 949 acceptance tests, which is precisely the coverage hole.

## Blast Radius

- `shared/code/entry_and_arena.rs` — both fixes. Non-arg entries unchanged
  (both are gated on `language_entry_accepts_args` / app mode).
- `shared/code/types.rs`, `shared/code/mod.rs`, and the four backend
  `emit_program_entry` shims — thread the new flag through.
- Console builds on aarch64/riscv64/macOS: behavior unchanged (they already
  worked); their arg-entry instruction ORDER changes (park moves earlier).

## Validation

- `tests/entry_args.rs` (new): the park invariant on x86-64 console, x86-64 app,
  and aarch64 console; that an app entry never reads args off the worker stack;
  and a host end-to-end run asserting a program receives `argc=3`/`alpha`/`beta`.
  With the pre-fix ordering restored, the three park tests fail naming the exact
  clobbering instruction.
- Runtime, after the fix — the same program, one build per target:

| Target | VM | Result |
| --- | --- | --- |
| linux-x86_64 glibc | 2228 | `argc=3` + args ✓ |
| linux-x86_64 musl | 2227 | `argc=3` + args ✓ |
| linux-aarch64 glibc | 2223 | `argc=3` + args ✓ |
| linux-riscv64 musl | 2229 | `argc=3` + args ✓ |
| macos-aarch64 | host | `argc=3` + args ✓ |

- App mode (GTK, VM 2228, headless `gtk4-broadwayd`): the worker delivered the
  real vector — `argc=4|/tmp/gtkargs.out|alpha|beta|gamma`.
- Full acceptance: 949/949, zero golden churn.

## Notes / follow-ups

- A GTK app under the **broadway** backend segfaults during teardown *after* the
  program body completes (an arg-less `SUB main` writes its file, then crashes).
  Pre-existing and unrelated to this fix or bug-240 — reproduced identically on
  a pre-change baseline binary — and absent on a real X display (c1e76921
  verified GTK app mode interactively on this box). Not chased here; only the
  headless broadway path is affected.
