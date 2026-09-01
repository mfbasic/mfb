# bug-479: Windows `Mode.Canvas` — the WORKER faults inside `canvas::present`, reading a String whose payload pointer is `2`

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

## Corrected: it is the WORKER that faults, not the graphics thread

A debugger changed the picture. `cdb` is on box 2230 after all —
`C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe`; the earlier
"not found" was a quoting mistake, and `scripts/` now has `dbg.bat`-style usage
recorded below. With it:

```
(131c.15e8): Access violation - code c0000005 (!!! second chance !!!)
rax=0000000000000002 rbx=0000000000000002 rcx=00000000001d0020
rdx=000000000001ffff rsi=000000000337fdb8 rdi=0000000000000000
rip=00000001405caad1 rsp=000000000337e280 rbp=0000000000000000
 r8=000000000337d2c0  r9=00000000001b0024 r10=0000000000000002
r15=000000000337ef00
```

**`r15` is the arena-state register on x86-64** (`arch/x86_64/regmodel.rs`), and here it
holds a *stack* address — which is what the arena state looks like on the **worker**,
whose state is carved in its entry frame. The graphics thread's arena is a heap block.
So the faulting thread is the worker.

Statement-level bisect with prints confirms it: a program that prints either side of
each statement gets as far as

```
A: mode set
B: list built
[rc=-1073741819]
```

so it dies **inside `canvas::present`**, and `C: presented` never runs.

It also needs the graphics thread to have run: with the trampoline stubbed to return
immediately, the program *hangs* in `syncFrame` instead of crashing. So the sequence is
graphics-thread-does-something → worker wakes → worker faults.

The faulting instruction, from the same binary under `objdump`:

```
1405caa6c: movl  $0x8, %eax
1405caa71: callq <seam>
1405caa76: movq  0x58(%rsp), %rcx      ; success arm: answer into rcx
1405caa83: movabsq $0x0, %rcx          ; failure arm: 0 into rcx
1405caa8d: addq  $0x60, %rsp           ; the seam's own epilogue
1405caa94: movq  %rax, %r10            ; the CALLER reads rax
...
1405caad1: movzbq (%r10), %r11         ; <- FAULT, r10 = 2
```

An inline seam with a `0x60` frame puts its answer in **`rcx`** — which is
`return_register()` on Win64 — and the surrounding code reads **`rax`**, which is
`c_return(0)`. That is the plan-85 `%retC`-vs-aligned-bank split again, on the *read*
side, and it would be the sixth instance of the class bug-478 catalogued. What lands in
`r10` is then whatever the last C call left in `rax` — here `2` — and the byte-scan that
follows is an MFB `String` length walk over it.

**Not yet identified: which seam, and which reader.** The two must be matched before
this is fixed rather than guessed at; the naming is the whole difficulty, because on
AArch64 the two registers coincide and every candidate looks correct there.

## Ruled out since filing

* **The child arena block's size.** `emit_graphics_spawn` sizes it
  `ENTRY_GLOBALS_OFFSET + arena_global_slots * 8`, the same shape `thread::start` uses
  after bug-369. Instrumented on Windows: the entry and the canvas spawn both report
  `arena_global_slots=44`, so the block is not short and the graphics thread's globals
  do not run off the end of it.

* **The graphics trampoline's frame.** `emit_graphics_trampoline` is hand-built, so it
  gets none of the shadow space `finalize_frame` reserves for allocated functions
  (`outgoing_args_base_offset`), and it took the x86-64 `+8` pthread realign on Windows
  where a `CreateThread` entry is already aligned. Both are real ABI defects and both are
  **fixed** — the frame is now `32 + 32` with the saves above the shadow space, and
  Windows takes no realign. Neither changed the fault. Kept anyway: a hand-built Win64
  frame without shadow space is a callee's licence to overwrite the saves, and it is
  byte-identical everywhere else (`artifact-gate all`, 0 diffs).

* **The shadow-space class is narrower than it first looked.** The Win64 backend already
  answers `shadow_space_bytes` / `outgoing_args_base_offset` as 32, and the shared
  `finalize_frame` honours it — so **allocated** functions have their shadow space and
  need no help. Only *hand-built* frames lack it, which is why `emit_random_bytes` (in
  the hand-managed program entry) needed one and `emit_arena_map` (inside the allocated
  `arena_alloc`) is actively harmed by one: the allocator spills `map_size` around that
  call, and moving `rsp` underneath it invalidates the spill slot. That is the mechanism
  behind the `ErrOutOfMemory` regression recorded below, and it means the audit is
  "which hand-built frames call out", not "which seams call out".

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

Whoever picks this up should start there — but note the correction above: the arena
seam sits in an *allocated* function that already has its shadow space, so the fix is
not to add one. The remaining question is what on the graphics thread produces a String
whose payload pointer is `2`.

The next thing to try is a debugger rather than another minidump. The box is an
administrator and the loader now has a Vulkan driver registered (see below), so a
`cdb`/WinDbg session that breaks on the access violation would name the function in one
step, where a dump only gives an image offset.

## Reproduction

Under a debugger, which is the fastest route and needs no dump:

```
cdb.exe -g -G -c "sxe av; g; r; kb 12; u @rip-20 L14; q" <name>.exe
```

with `MFB_WINAPP_HEADLESS=1` and `MFB_CANVAS_SYNC=1` set. The x64 build is at
`C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe` (there is an arm64
one beside it — take the x64).

Without one:

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
this is the next one. Phase 3's *other* blocker is now cleared: box 2230 has Mesa
26.2.0's `lavapipe` registered as an ICD (`vkCreateInstance=0`, one device), so a
Windows Vulkan render can be verified there as soon as a Windows canvas program runs at
all. **This bug is the only thing left between plan-98-F Phase 3 and its acceptance.**
