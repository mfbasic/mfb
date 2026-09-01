# bug-479: Windows `Mode.Canvas` — the graphics thread ran 8 bytes out of stack alignment

Last updated: 2026-08-31
Effort: large — Windows-only, needs the box and a minidump
Severity: HIGH
Class: Correctness (Windows canvas runtime)

Status: FIXED
Regression Test: two.

* `codegen::runtime::canvas::tests::every_x86_64_graphics_trampoline_frame_realigns_the_stack`
  — asserts the frame is `8 (mod 16)` on x86-64 for both families. RED-checked by
  restoring the `&& !windows`.
* `scripts/test-winapp.sh` now builds a canvas program, presents a scene and asserts
  the dumped frame's size and three pixels. **This is the one that would have caught
  it**: the unit guard encodes the premise, and the premise was what was wrong.

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

## Resolution — the stack, not the arguments

`emit_graphics_trampoline` skipped the x86-64 `+8` realign on Windows, on the stated
premise that `BaseThreadInitThunk` enters a `CreateThread` start routine already
16-aligned. **That premise is false.** The thunk reaches the start routine through an
ordinary `call`, so it begins at `rsp % 16 == 8` exactly like `_pthread_start`, and a
frame that is a multiple of 16 hands the skew to every call the render loop makes.

Measured on box 2230, one breakpoint on `ntdll!RtlAcquireSRWLockExclusive` printing
`@rsp`, with both threads acquiring the SAME canvas mutex:

```
worker   t=1728  rsp=0x332de68  -> % 16 == 8   correct
graphics t=1a28  rsp=0x3b2fd30  -> % 16 == 0   skewed
```

The worker is correct because its entry pushes a register before its frame; the
trampoline only subtracts, so the frame has to carry the odd 8 itself.

Nothing noticed the skew until a Win32 callee cared. `SleepConditionVariableSRW` cares:
ntdll builds its wait block on the **caller's** stack and tags the pointer in the low 4
bits (`and rdx,0FFFFFFFFFFFFFFF0h` at `RtlSleepConditionVariableSRW+0x13d`). Eight bytes
out, that mask lands mid-block, the wait-list walk below it loads a NULL `Next`, and
`mov [rcx+10h],rax` faults with `rcx=0` — on the very first wait, with **every argument
correct**: CV initialised and empty, lock genuinely held, timeout NULL, flags 0.

Fix: `let realign = usize::from(arch == "x86_64") * 8;` — drop the `&& !windows`.

### Verified

```
2230, no debugger:  rc=0, "presented"
MFB_CANVAS_DUMP  ->  2304000 bytes = 900*640*4 exactly
background           (0,0,0,255)      opaque black
rectangle            (200,40,40,255)  24000 px = 200x120 exactly
circle               (40,200,120,255) 11112 px, 31 distinct colours (antialiased)
```

The first Windows canvas frame this project has produced, and the plan-98-F Phase 3
prerequisite is now clear.

### The four fixed on the way in

Real defects, all of them, and all the plan-85 return-bank family — a Win32 call answers
in `rax` (`c_return(0)`) while the code named `return_register()`, which is `rcx` on
Win64 and the SAME register on AArch64, so each read correctly on the Mac:

1. `emit_env_get` — `os::getEnvOr` returned a **byte count** where a pointer was wanted.
   That is the "payload pointer is `2`" this bug was filed about: `2` was the value's
   length. `4c9e7e16a`.
2. `_mfb_winapp_canvas_blit` read `HeapAlloc`'s block from the wrong bank. `74c034c01`.
3. `WM_GETTEXTLENGTH` masked `rcx` while its own comment said the length arrives in `rax`.
4. `MultiByteToWideChar` (two sites) and `GetDC`. 3 and 4 in `cce4707f9`.

None of them was the fault this bug was named for. All four were shipped bugs found
while looking for it.

## Superseded — the investigation as it stood mid-way

## Current state (2026-08-31, second session) — the render works; the WAIT does not

Four defects have been found and fixed since filing, all of them the plan-85 return-bank
family (a Win32 call answers in `rax` = `c_return(0)`; the code named `return_register()`,
which is `rcx` on Win64 and the SAME register on AArch64 — so every one read correctly on
the Mac and wrongly on Windows):

1. `emit_env_get` answered in the wrong register, so `os::getEnvOr` returned a **byte
   count** where a pointer was wanted. That is the `payload pointer is 2` this bug was
   filed about: `2` was the length of the value string. Fixed in `4c9e7e16a`.
2. `_mfb_winapp_canvas_blit` read `HeapAlloc`'s block from `return_register()`. `74c034c01`.
3. `WM_GETTEXTLENGTH` masked `rcx` while its own comment said the length arrives in `rax`.
4. `MultiByteToWideChar` (two sites) and `GetDC` — same shape. 3 and 4 in `cce4707f9`.

**The canvas now renders on Windows.** `canvas::present` completes, `"presented"` prints,
the program runs its whole body and its `os::sleep` expires. The original symptom is gone.

What remains is a different fault, at the other end of the program:

```
ntdll!RtlSleepConditionVariableSRW+0x148:
00007ff9`cce7fbd8 48894110  mov qword ptr [rcx+10h],rax   ds:0000000000000010=????
rcx=0  rax=0000000003aefc90 (a wait block on the faulting thread's own stack)
```

`~*k` puts it on the **graphics thread** (thread 5 of 6; thread 0 is in
`WaitForSingleObjectEx`, threads 1-3 are the ntdll thread pool, thread 4 is the worker).

### What the breakpoint trace establishes

Run `scripts/`-free, with a cdb script file (`-cf`) — note **not** `-g`, or cdb runs to
the fault before it ever reads the script and no breakpoint is ever armed:

```
bp ntdll!RtlAcquireSRWLockExclusive   ".printf \"ACQ  t=%x lock=%p\\n\", @$tid, @rcx; gc"
bp ntdll!RtlSleepConditionVariableSRW ".printf \"WAIT t=%x cv=%p lock=%p lockval=%p\\n\", @$tid, @rcx, @rdx, poi(@rdx); gc"
```

yields, immediately before the fault:

```
ACQ  t=175c lock=00000001405dc018
WAIT t=175c cv=00000001405dc048 lock=00000001405dc018 lockval=0000000000000001
```

and the emitted call site disassembles exactly as `emit_wait_for_redraw` writes it:

```
mov rcx,r12 ; add rcx,40h    ; cv   = base + GRAPHICS_OFFSET_COND  (64)
mov rdx,r12 ; add rdx,10h    ; lock = base + GRAPHICS_OFFSET_MUTEX (16)
mov r8,0 ; sub r8,1          ; dwMilliseconds = INFINITE
mov r9,0                     ; Flags = 0 (exclusive)
call SleepConditionVariableSRW
```

So the arguments are right, the lock **is** held (`lockval=1`), and this is the **first**
wait in the process — there is exactly one `WAIT` line in the whole trace.

### Ruled out, each with its measurement

| hypothesis | how it was ruled out |
|---|---|
| the worker, not the graphics thread, faults | `~*k 6` — the worker is thread 4, in our code; the fault is thread 5 |
| the 1 MiB `CreateThread` default stack overflows | gave it 8 MiB reserved (`STACK_SIZE_PARAM_IS_A_RESERVATION`); byte-for-byte the same fault |
| wrong argument registers (the bug-478 family again) | disassembled the call site, above |
| the SRW lock is not held at the wait | `lockval=1` in the trace; the `0` seen in a post-fault `dq` is ntdll having already released it |
| corruption accumulated over frames | it is the first wait; only one `WAIT` line exists |
| cv/lock offsets collide in the state block | `MUTEX=16`, `COND=64`, `ARENA=112` — 48 bytes apart, and a Win32 `CONDITION_VARIABLE` is 8 |

### The one number still missing

What the `CONDITION_VARIABLE` at `base+0x40` contains **at the instant of the call**. A
zeroed CV is the valid initial state and would work; a non-zero one would make ntdll walk
a garbage queue and produce precisely this `mov [rcx+10h],rax` with `rcx=0`. Print it with
`poi(@rcx)` in the `WAIT` breakpoint (staged as `/tmp/p98f-win/trace2.cdb`), together with
a breakpoint on `ntdll!RtlInitializeConditionVariable` to confirm the CV is ever
initialised at that address and on no other.

The next suspect after that is the missing Win64 **shadow space** on this call: the
`pthread_cond_wait` arm of `src/codegen/runtime/thread/runtime_helpers.rs` reserves none,
while the `pthread_cond_timedwait` arm right below it explicitly subtracts `0x50` with the
comment *"0x00..0x20 is the Win64 shadow space every call below needs"*. Missing shadow
space is the exact defect bug-478 found in `emit_random_bytes`.

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
