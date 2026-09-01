# bug-479: Windows `Mode.Canvas` faults on the graphics thread — a UTF-8 marshal dereferences a pointer of `2`

Last updated: 2026-08-31
Effort: large — Windows-only, needs the box and a minidump
Severity: HIGH
Class: Correctness (Windows canvas runtime)

Status: Open
Regression Test: none yet. `scripts/test-winapp.sh` exists now (bug-478) and covers
app mode, `fs` and the worker; it does **not** cover `Mode.Canvas`. Extending it is
part of the fix — the canvas assertions belong beside the ones already there.

## What happens

A `--app` build for `windows-x86_64` that enters `Mode.Canvas` and presents **any**
scene faults with `0xC0000005`. The scene does not matter: `canvas::present([])`
faults identically.

```
> set MFB_WINAPP_HEADLESS=1
> set MFB_CANVAS_SYNC=1
> wc6.exe
mode set
[rc=-1073741819]        (0xC0000005)
```

`app::setMode(Mode.Canvas)` **on its own** is fine — the program prints on both sides
of it and exits 0. It is the first `present` that dies, which is the first thing that
starts the graphics thread.

## Where

From a `LocalDumps` minidump (`DumpType=1`) of the faulting run:

```
faulting thread : the graphics thread (not the worker, which printed "mode set")
exception code  : 0xc0000005
exception addr  : <image>+0x5cc373     <- OUR code, not a system DLL
params          : ['0x0', '0x2']       <- a READ of address 0x2
rsp             : 16-byte aligned      <- so NOT an alignment fault
```

Disassembling the image at that offset:

```
1405cc356: movq  0x80(%rsp), %r11      ; a pointer, out of a stack slot
1405cc35e: movq  %r11, %r10
1405cc361: movabsq $0x0, %r11          ; index 0
1405cc36b: movq  %r11, 0x90(%rsp)
1405cc373: movzbq (%r10), %r11         ; <- FAULT: byte load through r10
```

and the captured stack has `[rsp+0x80] = 0x0000000000000002`, so `r10` is literally
`2`. The surrounding code is unmistakable — `movabsq $0xfde9` (65001 = `CP_UTF8`),
`$0x8000`/`$0x20000` buffer sizes, six-argument calls staging `[rsp+0x20]`/`[rsp+0x28]`
— it is the UTF-8 → UTF-16 marshalling helper (`emit_utf8_slot_to_wide`).

So: **something on the graphics thread hands a Win32 seam a String whose payload
pointer is 2.** The nearest caller frame is an arena allocation of 32 bytes
(`callq <alloc>` with `rcx=0x20, rdx=0x8`), whose result is checked and then used.

## What it is not

* **Not alignment.** `rsp % 16 == 0` at the fault, and gating Windows out of the
  graphics trampoline's x86-64 `+8` realign (`emit_graphics_trampoline`) does not
  change the outcome — tested.
* **Not the `MFB_CANVAS_DUMP` / `MFB_CANVAS_STATS` path.** It faults with neither set.
* **Not `os::getEnvOr` itself.** The same call on the **worker** thread returns
  correctly (`got:unset`), so the seam works; it is the thread it runs on that differs.
* **Not bug-478.** That is fixed and verified; app mode, `fs` and the worker are green
  (`scripts/test-winapp.sh`).

## The leading hypothesis, and why the obvious fix is wrong

`emit_arena_map` (`src/target/win_x86_64/code.rs`) calls `VirtualAlloc` and
`emit_arena_unmap` calls `VirtualFree` **without reserving the callee's shadow space**
— the exact defect bug-478 found in `emit_random_bytes`. On Win64 those 32 bytes are
the caller's job and land *above* its `rsp`, in its own frame, so the callee is free to
spill four registers over 32 bytes of the enclosing body's locals. A corrupted pointer
slot on the graphics thread is precisely the observed symptom.

**But adding the frame there regresses app mode**: with
`instructions.push(abi::subtract_stack(0x20))` around each call, the canvas fault
becomes a clean `ErrOutOfMemory` (`7-701-0001`) on the program's *first* allocation,
while the console path stays green. So `emit_arena_map`'s caller is not an ordinary
body with an aligned, sp-relative frame, and a bare `sub rsp` inside it is not the fix.
Measured both ways; the change is reverted, with the reason recorded at the seam.

Whoever picks this up should start there: find what the arena bootstrap assumes about
`rsp` across that call, and give the seam its shadow space in a way that respects it.

## Reproduction

```
mfb build --app --target windows-x86_64 <project>   # setMode(Mode.Canvas); present([])
scp the .exe to box 2230, then:
  set MFB_WINAPP_HEADLESS=1
  set MFB_CANVAS_SYNC=1
  <name>.exe            -> rc = -1073741819
```

For the minidump (the box user is an administrator), `LocalDumps` is already
configured to `C:\mfbvk\dumps` with `DumpType=1`. `/tmp/parsedump.py` in the session
that filed this parsed the exception, module and thread streams directly; the layout
notes that cost time are in bug-478 (and one more: `MINIDUMP_EXCEPTION` is
`code(0) flags(4) nested(8) address(16) nparams(24) unused(28) info[15](32..152)`,
with the stream's `ThreadContext` location at **+152** — reading `nparams` at +20 as
bug-478's parser did yields a nonsense `params` list).

## Found while

plan-98-F Phase 3 needs a Windows canvas frame to compare a Vulkan render against, so
it needs canvas mode to run. bug-478 was the first blocker on that path and is fixed;
this is the next one. Note that Phase 3 has a **second**, independent blocker: box 2230
has the Vulkan loader (`C:\Windows\System32\vulkan-1.dll`) but **no ICD**
(`HKLM\SOFTWARE\Khronos\Vulkan\Drivers` does not exist), so a Windows Vulkan render
cannot be verified there at all without provisioning a software driver.
