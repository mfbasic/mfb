# bug-478: a Windows `--app` worker thread faults inside `BCryptGenRandom` — every app-mode program dies with `0xC0000005`, whatever it does

Last updated: 2026-08-31
Effort: large (half a day+) — Windows-only, needs the box and a minidump
Severity: HIGH
Class: Correctness (Windows app-mode runtime)

Status: Fixed — 2026-08-31
Regression Test: `scripts/test-winapp.sh` (the box script that did not exist, and whose
absence is why this shipped), plus two codegen guards that run in `cargo test`:
`every_calling_win32_seam_reserves_the_callees_shadow_space` in
`src/target/win_x86_64/code.rs` and `the_worker_frame_keeps_the_stack_16_byte_aligned`
in `src/target/win_x86_64/app/mod.rs`. Both RED-checked against the original defects.

## What happens

A `--app` build for `windows-x86_64` runs its worker thread, reaches the program
entry, and faults before the first statement of `main` executes. The program body
is irrelevant — an **empty** `SUB main() END SUB` faults identically:

```
> set MFB_WINAPP_HEADLESS=1
> winapp2.exe
[rc=-1073741819]        (0xC0000005, ACCESS_VIOLATION)
```

The same source built **without** `--app` runs clean (`rc=0`), so this is app mode,
not the program.

## Where

From a `LocalDumps` minidump (`DumpType=1`), parsed for the exception record and a
stack scan of the faulting thread:

```
faulting thread : 288            (the CreateThread worker, not the main thread)
exception code  : 0xc0000005
exception addr  : ntdll.dll+0x70f32
access violation: read at 0xffffffffffffffff

return addresses on the captured stack, innermost first:
    ntdll.dll+0x70f32          <- fault
    ntdll.dll+0x40aa0 / +0x178ce0 / +0x17ad30 / +0x134117 / +0x70c66 / +0x706ce
    KERNELBASE.dll+0x214cf
    bcrypt.dll+0x2e60 ... +0x905c        (~14 frames)
    winapp2.exe+0x10b4 / +0x106f / +0x10fb
    winapp2.exe+0x1e55                   <- _mfb_winapp_worker
    kernel32.dll+0x2ccb7                 <- BaseThreadInitThunk
```

`ntdll+0x70f32` sits between the exports `RtlDosApplyFileIsolationRedirection_Ustr`
(+0x192) and `RtlFindActivationContextSectionString` (−0x295e) — the SxS /
activation-context machinery the loader uses to resolve a module or a path. The
faulting read is of `0xFFFFFFFFFFFFFFFF`, i.e. something held −1 where a pointer
belonged.

So: the worker calls the program entry, the entry seeds its RNG through
`emit_random_bytes` → `BCryptGenRandom(NULL, buf, len, BCRYPT_USE_SYSTEM_PREFERRED_RNG)`,
and bcrypt's first-use initialization dies in ntdll.

## Why it is not simply "bcrypt is broken"

The **console** build calls exactly the same seam on its main thread and is fine.
Both PEs import `bcrypt.dll` / `BCryptGenRandom` (checked by grepping the two
binaries). The difference is the thread: app mode runs the entry on a
`CreateThread` worker.

Two candidate mechanisms, neither yet proven:

1. **Missing shadow space.** `emit_random_bytes` is "append shape" — it emits no
   frame of its own and relies on the `abi_function` finalizer for one. If the
   finalizer's frame does not reserve Win64's mandatory 32-byte shadow area at the
   point of this call, `BCryptGenRandom` spills over the caller's locals. That
   would corrupt state that only fails later, which matches a fault in an unrelated
   ntdll routine.
2. **Per-thread loader state.** The read of −1 in the activation-context path looks
   like `TEB->ActivationContextStackPointer`. Nothing in `src/target/win_x86_64/`
   touches the TEB (grepped: no `gs:`, no `Tls*`), so if this is it, it is
   corruption from elsewhere rather than a deliberate write.

## Reproduction

```
mfb build --app --target windows-x86_64 <project>     # SUB main() END SUB
scp the .exe to box 2230, then:
  set MFB_WINAPP_HEADLESS=1
  winapp2.exe          -> rc = -1073741819
```

For the minidump (the box user is an administrator):

```
HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps
  DumpFolder = C:\mfbvk\dumps   DumpCount = 5   DumpType = 1
```

and parse the exception + thread + module streams directly — the header is
`MDMP`, the directory is `(StreamType, DataSize, Rva)` triples, exception is stream
6, thread list 3, module list 4. `MINIDUMP_MODULE.ModuleNameRva` is at **+20**, not
+16; `MINIDUMP_THREAD` is 48 bytes with the stack descriptor at +24 and the
`CONTEXT` location at +40; in an AMD64 `CONTEXT`, `Rip` is at 0xF8 and `Rsp` at 0x98.

## Found while

plan-98-F Phase 3 needs a Windows canvas frame to compare a Vulkan render against,
so it needs app mode to run. Four Windows defects were fixed on the way to this one
and are **not** part of this bug (they are fixed):

* `emit_open_file` never moved `CreateFileW`'s handle out of `rax`, so **every
  `fs` write on Windows failed** with a 0-byte file left behind;
* ~20 sites in `win_x86_64/app/mod.rs` read a Win32 result from
  `abi::return_register()` (the aligned MFB bank, `rcx`) instead of
  `abi::c_return(0)` — including `CreateThread`, which made `WaitForSingleObject`
  return instantly so app mode exited before its worker ran;
* the `WNDPROC` return value was written to `rcx` rather than `rax`;
* `GetEnvironmentVariableW`'s result was read the same wrong way in three places,
  so the `MFB_WINAPP_HEADLESS` test **always** took the headless branch and the GUI
  path was unreachable.

All four share one root: plan-85 split `%retC` from the aligned MFB bank, and
`win_x86_64/app/mod.rs` was never audited for it. `src/target/win_x86_64/code.rs`
was — its comments cite plan-85 explicitly — which is why the console path works
and app mode did not.

## Fix

**Two defects, and the second was hiding behind the first.**

**1. `emit_random_bytes` reserved no shadow space.** Win64 makes the *caller* leave the
callee 32 bytes, and those bytes are **above** `rsp` — in the caller's own frame. So a
seam that calls without reserving them hands the callee 32 bytes of its own locals to
spill into. `emit_random_bytes` emitted no frame at all; every other external-call
emitter in `win_x86_64/code.rs` reserves one. That is hypothesis 1 from this report,
now proven: reserving the frame took the empty `SUB main() END SUB` from
`0xC0000005` to `rc=0`.

**2. The worker's frame was an odd multiple of 8.** `emit_worker` reserved `0x28`, which
is the right shape for an ordinary prologue — a function reached by `call` arrives with
`rsp % 16 == 8`. A **thread start routine does not**: `BaseThreadInitThunk` enters it
already aligned. So `0x28` left the call site 8 bytes out, and because the program body's
alignment simply *is* its call site's, every Win32 call the program would ever make
inherited the skew.

That is the one that made this so hard to see. The console path was green throughout —
it comes from the PE loader, whose 8-byte skew `entry_stack_misaligned_on_entry` already
shaves — so the two paths disagreed by exactly 8 bytes, and a fix measured on one broke
the other. Measured both ways: with the worker at `0x28`, the shadow-space frame had to
be `0x28` for app mode and `0x20` for console; at `0x20` both want `0x20`, which is what
every other emitter in the file already assumed.

Neither hypothesis about the TEB was needed. The read of `0xFFFFFFFFFFFFFFFF` in ntdll's
activation-context path was downstream of a misaligned stack, not a cause.

## Evidence

Box 2230, `scripts/test-winapp.sh`:

```
rc=0
worker reached main
readback:written by the worker
ok: the app-mode program exited cleanly
ok: the worker thread reached the program's first statement
ok: a file written by the worker reads back with its contents
```

The console path re-checked at the same commit (empty `SUB main() END SUB`,
`--target windows-x86_64` without `--app`): `rc=0`. Against the merge-base binary
(`git archive 739ee1434 | tar -x -C /tmp/base98` + `cargo build --release`) the same
source also gives `rc=0`, which is how the intermediate console regression was caught
rather than shipped: an early version of this fix used `0x28` and broke it.

## What it cost to find, and what would have caught it sooner

Five Windows defects in one sitting, all of them shipped, all of them invisible to a
green `cargo test` on a macOS host: `fs` writes, `CreateThread`'s handle, the WNDPROC
return, `GetEnvironmentVariableW` in three places, and this. The first four share a root
(plan-85's `%retC` split, never audited in `win_x86_64/app/`); this one does not — it is
older, and it is a plain ABI mistake.

What they share is the *absence of a Windows test*. `scripts/test-winapp.sh` now runs an
app-mode program on the box and checks three things the four fixed defects each broke.
It is three assertions and it would have caught four of the five on the day they landed.

