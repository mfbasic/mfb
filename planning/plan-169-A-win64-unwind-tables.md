# plan-169-A: Win64 unwind tables (.pdata/.xdata) and constant-size prologues

Last updated: 2026-10-03
Overall Effort: x-large (1d–3d across A, B, C)
Effort: large (3h–1d)
Depends on: nothing

A Windows x86-64 executable tells the operating system and every debugger how to
walk its stack through **unwind tables**: one `RUNTIME_FUNCTION` entry per
function in `.pdata`, each pointing at an `UNWIND_INFO` record in `.xdata` that
describes the function's prologue (how far it moved `rsp`, which nonvolatile
registers it saved and where). MFBASIC writes none. Without them gdb (and Windows
itself: exception dispatch, Windows Error Reporting stacks, ETW profilers) cannot
find a frame's return address. `mfb man debug backtraces` documents the symptom,
measured on box 206: at a breakpoint gdb names the function and its caller, and
every frame past that is `??` at a random address.

plan-169 makes a Windows backtrace walk all the way to `_main`. This sub-plan
writes the tables for every function that has a frame, and first makes every
prologue describable: today a frame larger than 48 KiB has a prologue too long
for `UNWIND_INFO` to express. The outcome: on box 206, a breakpoint in any
MFBASIC function gives a backtrace that ends at `_main`, and `llvm-objdump
--unwind-info` decodes an entry for every function that moves `rsp`.

Sub-plans B and C remove the two remaining places where `rsp` moves outside a
prologue (mid-body call windows; encoder scratch pushes). Until they land, a stop
*inside* one of those short windows still unwinds wrongly; a stop anywhere else,
and every breakpoint, unwinds correctly after A.

References:

- Microsoft, "x64 exception handling" (`RUNTIME_FUNCTION`, `UNWIND_INFO`,
  `UNWIND_CODE`, the epilogue rules) and "x64 prolog and epilog".
- `src/codegen/engine/util/vreg_frame.rs:finalize_frame`, `push_frame_allocation`:
  the shared prologue/epilogue builder.
- `src/arch/x86_64/backend.rs:Win64Backend` (`stack_probe_page_bytes`).
- `src/arch/encode_plan.rs:encode_plan`, `src/arch/x86_64/encode/emitter.rs`.
- `src/arch/image.rs:EncodedImage`.
- `src/os/windows/link/mod.rs:write_executable`, `src/os/windows/link/pe.rs:write_image`,
  `ImportDirectories`.
- `mfb spec linker windows-x86_64` (PE headers, sections, the probe sentence),
  `mfb man debug backtraces`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Host llvm-objdump decodes COFF unwind info | `objdump --help \| grep -c unwind-info` → ≥ 1 | MET (2026-10-03) |
| Windows box 206 answers and has w64devkit gdb | `ssh test@192.168.1.206 "C:\Users\test\w64devkit\bin\gdb.exe --version"` → `GNU gdb …` | MET (2026-10-03) |
| The `mfb man debug` harness runs on Windows | `bash scripts/man-debug-examples.sh target/release/mfb windows` → `0 failed` | MET (2026-10-03: 8 passed, 0 failed) |

Everything below assumes these hold. (B and C point here for prerequisites.)

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue, and again before you decide to
> stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- Every linked `windows-x86_64` executable carries `.pdata` and `.xdata`
  sections and data directory `[3]` (Exception) pointing at `.pdata`, with one
  `RUNTIME_FUNCTION` per function whose code moves `rsp`, sorted by address,
  whose `UNWIND_INFO` exactly describes that function's prologue.
- Every Win64 prologue fits `UNWIND_INFO`: at most 255 bytes long and at most 255
  unwind-code slots, whatever the frame size.
- On box 206, gdb's backtrace at a breakpoint in any MFBASIC function ends at
  `_main`.

### Non-goals (explicit constraints)

- No change to any non-Windows target's code, `.ncode`, or linked output.
- No PDB, no CodeView debug directory, no change to the COFF symbol table plan-166
  writes.
- No change to the MFBASIC internal calling convention or register allocation.
  The tables describe what the code does; they do not restrict it.
- No exception handlers (`UNW_FLAG_EHANDLER`/`UHANDLER`), no chained unwind info,
  no frame-pointer (`UWOP_SET_FPREG`) frames: every MFBASIC frame is
  fixed-size and `rsp`-relative.
- Not this sub-plan: mid-body `rsp` changes (B: call windows, C: encoder pushes).

## 2. Current State

**Prologue and epilogue, one builder.** `finalize_frame` allocates the frame with
`push_frame_allocation`, then saves each callee-saved register with a store at a
fixed `rsp` offset (`save_callee_saved`), inserted at the top of the body. Each
`ret` becomes: restores (reverse order), `add rsp, total`, `ret`. Measured shape on
Windows (objdump of the `examples/debug` and brogue `.exe`):

```
sub    $0x108,%rsp
mov    %r12,0x20(%rsp)
mov    %r14,0x28(%rsp)
movups %xmm6,0x30(%rsp)      (float-heavy functions; 16-aligned slots)
...
mov    0x20(%rsp),%r12       (restores)
add    $0x108,%rsp
ret
```

No `push`, no frame pointer. This maps one-to-one onto `UWOP_ALLOC_SMALL/LARGE`,
`UWOP_SAVE_NONVOL` and `UWOP_SAVE_XMM128`, and the `add rsp, imm32; ret`
epilogue is the form the Windows unwinder recognizes.

**Large frames.** Only the Win64 backend probes (`Win64Backend::stack_probe_page_bytes`
→ `WIN64_STACK_PROBE_PAGE` = 4096; every other backend returns 0).
`push_frame_allocation` emits, per whole page, `sub rsp, 4096` and
`mov qword [rsp], 0`, then `sub rsp, remainder`: 19 bytes per page, measured. A
frame of 13+ pages makes a prologue over 255 bytes, which `UNWIND_INFO`'s u8
`SizeOfProlog` and u8 code offsets cannot express.

**Hand-built frames.** Functions that build their own `CodeFrame` instead of
calling `finalize_frame`, Windows-reachable: `src/target/win_x86_64/app/mod.rs`
(the window procedure and key injection: save all 8 GPR nonvolatiles),
`src/codegen/runtime/canvas/mod.rs` (`_mfb_rt_canvas_graphics_entry`: saves
`r15, rbp`), `src/codegen/runtime/thread/runtime_helpers.rs`
(`_mfb_rt_thread_trampoline`: saves `r15, rbx, r13`),
`src/codegen/engine/function/entry.rs` (`_main`: `sub rsp, 8; sub rsp, 0x2180`),
`src/codegen/builtins/tls/gen_shared.rs`.

**Function extents.** `encode_plan` lays functions out contiguously, one symbol
per function at its start offset, with no padding (plan-166-A §2, verified
property). Linker-appended import thunks (`jmp [rip+iat]`) follow `image.text`;
they move no register and need no entry.

**PE today.** `write_image` writes data directories `[1]` Import, `[2]`
Resource, `[5]` BaseReloc, `[12]` IAT (`ImportDirectories`), every other one zero.
Sections: `.text`, `.rdata`, `.data`, `.idata`, `.rsrc`, `.reloc`, `.mfbnote`,
`.mfbsign`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Windows function instances surveyed | 2645 in 26 fixtures, plus brogue 249, dungeon 555 | `p169-census.sh` (Appendix) (builds every `tests/byte-identity/*` for `--target windows-x86_64 --debug`, objdumps), then `p169-frames.py` (Appendix) |
| Fixtures that do not build for Windows | 2 (`audio`, `link-const-pins`), neither has a Windows golden | same; `ls tests/byte-identity/{audio,link-const-pins}/golden \| grep -c windows` → 1 (audio has one) |
| Prologues over 255 bytes | 2 in the fixtures; 2 in brogue (802 and 745 bytes, 42 and 39 probe steps) | `p169-frames.py` (Appendix) "prologue > 255 bytes" |
| Largest frame seen | 170200 bytes (brogue `buildDungeonFeatureCatalog`) | same, "max alloc" |
| Frameless functions (no `rsp` change in prologue) | brogue 3 of 249; dungeon 23 of 555 | same, "frameless" |
| Save sets seen | `(r12,r14)`, `(r12)`, none; plus the four hand-built sets above; `xmm6`–`xmm10` (`movups`) | same, "save sets" |
| Epilogue shape | 22491 `add rsp, imm` immediately followed by `ret`; no `lea rsp`/`mov rsp` | `awk` over the census objdumps (prev line `addq …, %rsp`, count `retq`) |
| Windows `.ncodesum` goldens | 32 | `find tests -name '*windows-x86_64*.ncodesum' \| wc -l` |
| Fixtures with a frame > 4096 bytes (upper bound, `--debug` build) | 21 of 26 | census "max alloc" > 4096 |

### Verified properties

- **The Windows unwinder recognizes the epilogue.** `add rsp, imm32` followed by
  `ret` is the documented epilogue form; the restores before it are ordinary body
  loads (the slots still hold the saved values). Read `finalize_frame`'s `Ret`
  rewrite; census above.
- **XMM save slots are 16-aligned offsets.** `finalize_frame` aligns a 128-bit
  save slot to 16 (`is_simd128_callee_saved`), and `outgoing_bytes` is 16-aligned,
  so `UWOP_SAVE_XMM128` (offset scaled by 16) can encode it. Census offsets:
  0x30, 0x40, …
- **The probe's stored zero carries no meaning.** `push_frame_allocation` stores
  zero only to touch each page; frame slots that must start zero are zeroed
  explicitly in the body (`movq $0x0, 0x70(%rsp)` in the census). UNVERIFIED that
  no slot is ever read before being written: Phase 1 task.
- **Which registers a prologue scratch may use.** UNVERIFIED that `r10` and `r11`
  carry no incoming argument under MFBASIC's internal convention (`rax`, `rdi`,
  `rsi` do: `_mfb_ifn_canvas_5FpolylineScan` reads `rax` at entry). Phase 1 task:
  read `src/arch/x86_64/regmodel.rs` argument banks.

## 3. Design Overview

1. **Constant-size Win64 probe.** A frame larger than one page is probed by a
   loop that walks a scratch register down a page at a time and touches each
   page, **without moving `rsp`**, then allocates the whole frame with one
   `sub rsp, N`. The prologue becomes a fixed-size probe plus one allocation plus
   the saves: at most ~35 + 8×8 + 10×9 bytes, under 255 for every save set seen.
2. **Prologue annotations.** Every instruction that is part of a prologue
   carries an `unwind` field naming what it does: `alloc=<bytes>`,
   `save=<reg>@<offset>`, `savexmm=<reg>@<offset>`. `finalize_frame` sets them;
   the five hand-built frames set them through the same helpers.
3. **Encoder records.** The x86-64 encoder, encoding a function, records each
   annotated instruction's end offset (from the function start) and its op into
   a new `EncodedImage.unwind: Vec<EncodedUnwind>`. Other encoders leave it empty.
4. **PE emission.** `write_executable` turns the records into `UNWIND_INFO`
   blobs in `.xdata` and sorted `RUNTIME_FUNCTION`s in `.pdata`, sets data
   directory `[3]`, and refuses an image in which a function begins with
   `sub rsp` but has no record (the guard against a hand-built frame nobody
   annotated).

**Correctness risk** concentrates in the unwind-code encoding (reverse order,
the ALLOC_SMALL/LARGE thresholds, slot counts, the even-count padding). A wrong
code makes the unwinder read the wrong return address, silently. Phase 3 tests it
against llvm-objdump's decoder and the real unwinder on box 206.

**Design uncertainty** is in the probe rewrite: whether a loop that touches pages
below `rsp` before allocating is safe on Windows. It is what MSVC's `__chkstk`
does (it probes below `rsp`, then the caller allocates), and Windows commits a
stack page when any access hits its guard page in descending order. Phase 1
proves it by running brogue's 170 KB frame on box 206.

**Byte-identity is NOT this plan's gate.** Expected to change: every Windows
`.exe` (new sections and directory); Windows `.ncode` of every function with a
frame larger than one page (the probe). Expected unchanged: every non-Windows
`.ncode` and linked output, and Windows `.ncode` of functions with frames of one
page or less. A diff outside the expected set is a bug to root-cause (objdump one
fixture), never a reason to abandon the design.

**Rejected alternatives:**

- *Derive unwind info in the linker by pattern-matching prologue bytes.* The
  body stores spill registers to `[rsp+X]` with the same instructions
  (`movq %rdi, 0xb0(%rsp)`), and the internal convention clobbers `rbx`/`rdi`/`rsi`
  unsaved, so the bytes cannot tell a save from a spill. Codegen knows.
- *A `__chkstk`-style call.* Needs a runtime helper and a call in every big
  prologue; the inline loop is the same size and self-contained.
- *Unwind tables only for `--debug` builds.* Windows uses them for exception
  dispatch, crash reports and every stack-walking tool, whatever the build; MSVC
  always emits them. Open Decision 1.
- *A frame pointer (`UWOP_SET_FPREG`).* Would let `rsp` move freely in bodies, but
  `rbp` is an allocatable register in MFBASIC's convention
  (`_mfb_rt_canvas_canvas_vulkanDrawScene` uses it as a temporary); reserving it
  changes register allocation for every Windows function.

## 4. Detailed Design

### 4.1 The probe (`push_frame_allocation`, Win64 only)

For `total > page` (page = 4096):

```
mov   r11, rsp
mov   r10, <total rounded down to whole pages / 4096>   ; page count
probe:
sub   r11, 4096
mov   qword [r11], 0          ; touch (commits the guard page)
dec   r10
jnz   probe
sub   rsp, <total>            ; unwind: alloc=<total>
```

Pages are touched in descending order, exactly as today; only `rsp` stays put
until the single `sub`. `r10`/`r11` must be free at entry (Verified properties,
Phase 1 task). The loop needs labels unique per function; use the existing local
label mechanism. Frames of one page or less keep today's single `sub`.

### 4.2 Annotation

`CodeInstruction` gains nothing structural: the annotation is an ordinary
instruction field `unwind` (string), so `.ncode` dumps show it. `finalize_frame`
annotates the `sub rsp, total` (`alloc=<total>`) and each `save_callee_saved`
store (`save=r12@32`, `savexmm=xmm6@48`). The probe instructions are not
annotated (they do not move `rsp`). A hand-built frame annotates its own
`subtract_stack` and saves with the same helper (`abi::annotate_unwind(…)`).
`_main`'s two `sub`s are two `alloc` ops.

### 4.3 Encoder record

```rust
pub(crate) struct EncodedUnwind {
    pub(crate) function: String,          // the function's text symbol
    pub(crate) ops: Vec<(u8, UnwindOp)>,  // end offset from function start, op
}
pub(crate) enum UnwindOp { Alloc(u32), SaveNonvol { reg: u8, offset: u32 }, SaveXmm128 { reg: u8, offset: u32 } }
```

The encoder errors if an annotated instruction ends past byte 255 of its
function (the prologue-size limit), naming the function.

### 4.4 `UNWIND_INFO` and `.pdata`

Per record: `Version=1`, `Flags=0`, `SizeOfProlog` = the last op's end offset,
`FrameRegister=0`, codes in **descending** offset order:

- `Alloc(n)`: n ≤ 128 → `UWOP_ALLOC_SMALL` (OpInfo = n/8 − 1, 1 slot);
  n ≤ 512 KiB − 8 → `UWOP_ALLOC_LARGE` OpInfo 0, next slot n/8 (2 slots); else
  OpInfo 1, next two slots n (3 slots).
- `SaveNonvol`: offset/8 ≤ 0xFFFF → `UWOP_SAVE_NONVOL` (2 slots), else
  `UWOP_SAVE_NONVOL_FAR` (3 slots).
- `SaveXmm128`: offset/16 ≤ 0xFFFF → `UWOP_SAVE_XMM128` (2 slots), else `_FAR`.
- `CountOfCodes` = slots used; pad the code array to an even slot count.

Each `UNWIND_INFO` is 4-byte aligned in `.xdata`. `.pdata` holds 12-byte
`RUNTIME_FUNCTION { BeginAddress, EndAddress, UnwindInfoAddress }` (RVAs), one
per record, sorted by `BeginAddress`; `EndAddress` is the next function's start
(or `image.text.len()`). Functions with no record get no entry (leaf rule).
Sections: `.xdata` and `.pdata`, both `INITIALIZED_DATA | READ`, placed after
`.reloc` and before `.mfbnote` (Open Decision 2). Data directory `[3]` =
(`.pdata` RVA, size).

## Compatibility / Format Impact

- **Windows `.exe`**: two new sections, data directory `[3]` set, `SizeOfImage`
  and later sections' RVAs move. `mfb info` finds `.mfbnote`/`.mfbsign` by name,
  so it is unaffected (Phase 3 checks).
- **Windows `.ncode`**: large-frame prologues change shape; every prologue
  instruction gains an `unwind` field.
- **Everything else**: unchanged.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partial work, and mark moot tasks
> `- [x] ~~text~~ (moot: <evidence>)`. Fill each `Commit:` line when the phase
> lands. **An unticked box means NOT DONE.**

### Phase 1: Constant-size probe (uncertainty first)

- [ ] Verify `r10`/`r11` carry no argument at function entry under the Win64
      MFBASIC convention: read `src/arch/x86_64/regmodel.rs` argument banks and
      `WIN64_MODEL`; record the evidence here. If either can, pick a register
      that cannot, and say which.
- [ ] Verify no frame slot is read before it is written (the probe's stored zero
      is not relied on): grep `finalize_frame` callers for zero-initialized slot
      assumptions; record the evidence.
- [ ] `src/codegen/engine/util/vreg_frame.rs:push_frame_allocation`: emit the §4.1
      loop when `page != 0 && total > page`.
- [ ] Tests: a frame-shape unit test in `vreg_frame.rs` (Win64, total = 3 pages +
      100: one `sub rsp, total`, the loop, no `rsp` change before it).

Acceptance: brogue's 170 KB frame runs on Windows, and every Windows prologue is
at most 255 bytes.
  Check: `p169-frames.py` (Appendix) (re-created from this plan if `/tmp` was cleared)
  over a brogue Windows objdump → `max prologue bytes` ≤ 255 (est. 2 min).
  Check: copy brogue's `.exe` to box 206, `brogue.exe dig 1 1` → normal output,
  exit 0 (the deepest-frame path). (est. 2 min)
  Check: `bash scripts/artifact-gate.sh target/release/mfb all` → diffs only in
  `*.windows-x86_64.ncodesum` goldens of fixtures with a frame > 4096; for each,
  objdump one function and confirm the only change is the probe (est. 5 min). Re-pin
  only those sums, with the proof in the commit (AGENTS.md golden rule).
Commit: —

### Phase 2: Annotations and encoder records

- [ ] `src/target/shared/abi.rs`: an `unwind` field helper.
      `vreg_frame.rs:finalize_frame`: annotate the allocation and every save.
- [ ] The five hand-built Windows frames (§2) annotate their allocation and saves.
- [ ] `src/arch/image.rs`: `EncodedUnwind`, `UnwindOp`,
      `EncodedImage.unwind`; `encode_plan.rs` sets it empty; every test
      literal gets `unwind: Vec::new()` (count them with
      `grep -rn "debug_symbols: false" src | wc -l`, the field added last time).
- [ ] `src/arch/x86_64/encode/`: record ops per function (§4.3); error past
      byte 255.
- [ ] Tests: encoder tests that a `finalize_frame` function yields
      `[(7, Alloc), (15, Save r12 @32), …]` with the real byte offsets, and that a
      frameless function yields none.

Acceptance: every function of brogue that moves `rsp` in its prologue has a
record whose ops match its disassembly.
  Check: a test in `src/arch/x86_64/encode/tests.rs` that encodes a
  multi-function plan (small frame, xmm saves, 3-page frame) and asserts each
  record (est. 3 min).
Commit: —

### Phase 3: `.pdata` / `.xdata`

- [ ] `src/os/windows/link/pe.rs`: `ImportDirectories.exception`; data directory
      `[3]`; `UNWIND_INFO` and `RUNTIME_FUNCTION` encoders (§4.4).
- [ ] `src/os/windows/link/mod.rs:write_executable`: build both sections from
      `image.unwind`, place them (§4.4), refuse a `sub rsp`-starting function
      with no record.
- [ ] Tests in `src/os/windows/link/mod.rs`: decode the emitted `.pdata`/`.xdata`
      and check sort order, extents, every slot of an ALLOC_SMALL, ALLOC_LARGE
      (both forms), SAVE_NONVOL, SAVE_XMM128 record, even padding, and that the
      missing-record guard fires. `src/cli/info/tests.rs` PE cases still pass.

Acceptance: llvm's decoder agrees with the records.
  Check: `objdump --unwind-info examples/debug/build/debug.exe` (Windows build) →
  one entry per framed function, each with the ops its disassembly shows (est. 2 min).
Commit: —

### Phase 4: Proof on Windows, docs

- [ ] Box 206: `gdb -batch -ex 'break _mfb_fn_isPrime' -ex run -ex bt` on the
      `--debug -O0` example → frames `isPrime`, `countPrimes`, `runRound`,
      `runRounds`, `main`, `_main`, nothing after. Record the transcript.
- [ ] Box 206: the same at a breakpoint in a runtime helper reached through a
      hand-built frame (`_mfb_rt_thread_trampoline` via `debug 1 1000 1`).
- [ ] `src/docs/spec/linker/10_windows-x86_64.md`: data directory `[3]`, the
      `.pdata`/`.xdata` sections and their order, the probe sentence (§4.1).

`mfb man debug backtraces` is not changed here: its Windows caveat stays true
for stops inside mid-body windows until C, which rewrites the page.

Acceptance: the breakpoint backtrace on Windows ends at `_main`.
  Check: the 206 transcript above (est. 3 min).
Commit: —

## Validation Plan

- Tests: Phase 1 frame test, Phase 2 encoder tests, Phase 3 PE decode tests.
- Runtime proof: Phase 1 brogue run and Phase 4 gdb transcripts on box 206.
- Byte-identity: non-Windows `.ncode` and linked outputs unchanged (artifact gate
  diffs confined to Windows goldens, each root-caused).
- Doc sync: Phase 4.
- Final gate: in plan-169-C.

## Open Decisions

1. **Always emit unwind tables** *(recommended)*, or only for `--debug` builds.
   Windows' own exception dispatch, crash reports and profilers read them in
   every build, and they carry no names. Cost: every Windows `.exe` grows by
   12 bytes per framed function plus its `UNWIND_INFO`.
2. **Section placement**: after `.reloc`, before `.mfbnote` *(recommended)*, or
   folded into `.rdata`. Separate sections keep `.rdata`'s layout (and data symbol
   RVAs) unchanged.

## Corrections

(Filled in during execution.)

## Summary

The tables are standard and the prologue shape already matches them; the risk is
in encoding the unwind codes exactly, which Phase 3 checks against two
independent decoders. The one codegen change is the Win64 large-frame probe,
needed because today's probe sequence cannot be described.

## Appendix: census probes

One-off probes used for the measured populations of all three sub-plans. Save
them under `/tmp` to re-run; they are not repository scripts.

### `p169-census.sh`

```sh
#!/bin/bash
# plan-169 census: build every tests/byte-identity fixture for windows-x86_64 with
# --debug (for symbols) and run the frame and body-rsp censuses over each.
repo=/Users/justinzaun/Development/mfb
mfb=$repo/target/release/mfb
out=/tmp/p169-census; rm -rf $out; mkdir -p $out
for d in $repo/tests/byte-identity/*/; do
  n=$(basename $d)
  w=$out/$n; cp -R $d $w; rm -rf $w/golden $w/build
  (cd $w && $mfb build -q --debug --target windows-x86_64 . >/dev/null 2>&1) || { echo "BUILD FAILED $n"; continue; }
  exe=$(ls $w/build/*.exe 2>/dev/null | head -1)
  [ -n "$exe" ] && objdump -d --no-show-raw-insn "$exe" > $out/$n.dis
done
python3 /tmp/p169-frames.py $out/*.dis > $out/frames.txt
python3 /tmp/p169-bodyrsp.py $out/*.dis > $out/body.txt
```

### `p169-frames.py`

```python
"""plan-169 census: per function in a windows-x86_64 objdump, the prologue's
total rsp allocation, its byte length (through the last `sub rsp` / probe /
nonvolatile save), the number of `sub rsp` steps, and the registers saved.
One-off probe. Usage: python3 p169-frames.py <objdump -d output>..."""
import re, sys, collections

NONVOL = {"rbx", "rbp", "rdi", "rsi", "r12", "r13", "r14", "r15"}
for path in sys.argv[1:]:
    funcs = []
    cur = None
    for line in open(path):
        m = re.match(r"^([0-9a-f]+) <([^>]+)>:", line)
        if m:
            cur = {"name": m.group(2), "start": int(m.group(1), 16), "ins": []}
            funcs.append(cur)
            continue
        m = re.match(r"^\s*([0-9a-f]+):\s+(\S+)\s*(.*)$", line)
        if m and cur is not None:
            cur["ins"].append((int(m.group(1), 16), m.group(2), m.group(3).split("#")[0].strip()))
    stats = []
    for f in funcs:
        alloc = 0; steps = 0; end = f["start"]; saves = []; xmm = 0
        for i, (addr, op, args) in enumerate(f["ins"]):
            nxt = f["ins"][i + 1][0] if i + 1 < len(f["ins"]) else addr
            if op == "subq" and args.endswith("%rsp"):
                alloc += int(re.search(r"\$0x([0-9a-f]+)", args).group(1), 16); steps += 1; end = nxt
            elif op == "movq" and args.startswith("$0x0, (%rsp)"):
                end = nxt
            elif op == "movq" and re.match(r"%(\w+), 0x[0-9a-f]+\(%rsp\)", args) and re.match(r"%(\w+)", args).group(1) in NONVOL:
                saves.append(re.match(r"%(\w+)", args).group(1)); end = nxt
            elif op in ("movdqu", "movaps", "movups") and "%rsp" in args and re.search(r"%xmm(\d+)", args) and int(re.search(r"%xmm(\d+)", args).group(1)) >= 6:
                xmm += 1; end = nxt
            else:
                break
        stats.append((f["name"], alloc, end - f["start"], steps, tuple(saves), xmm))
    frameless = [s for s in stats if s[1] == 0]
    over = [s for s in stats if s[2] > 255]
    print(f"{path}: {len(stats)} functions; frameless {len(frameless)}; "
          f"max alloc {max(s[1] for s in stats)}; max prologue bytes {max(s[2] for s in stats)}; "
          f"prologue > 255 bytes: {len(over)}; xmm6+ saves: {sum(s[5] for s in stats)}")
    print("   save sets:", collections.Counter(s[4] for s in stats).most_common(5))
    for s in sorted(stats, key=lambda s: -s[2])[:3]:
        print("   largest:", s[0], "alloc", s[1], "prologue bytes", s[2], "steps", s[3])
```

### `p169-bodyrsp.py`

```python
"""plan-169 census: functions that change rsp outside their prologue (the leading
sub/probe run) and their epilogues (`add rsp` immediately followed by `ret`).
Windows x64 unwinding assumes rsp is fixed in the body unless a frame register
is established. One-off probe. Usage: python3 p169-bodyrsp.py <objdump -d>..."""
import re, sys
for path in sys.argv[1:]:
    funcs = []; cur = None
    for line in open(path):
        m = re.match(r"^([0-9a-f]+) <([^>]+)>:", line)
        if m:
            cur = [m.group(2), []]; funcs.append(cur); continue
        m = re.match(r"^\s*[0-9a-f]+:\s+(\S+)\s*(.*)$", line)
        if m and cur:
            cur[1].append((m.group(1), m.group(2).split("#")[0].strip()))
    bad = []; pushpop = 0
    for name, ins in funcs:
        i = 0
        while i < len(ins) and (ins[i][0] == "subq" and ins[i][1].endswith("%rsp") or ins[i][1].startswith("$0x0, (%rsp)")):
            i += 1
        body = ins[i:]
        n = 0
        for j, (op, args) in enumerate(body):
            if op in ("subq", "addq") and args.endswith(", %rsp"):
                if op == "addq" and j + 1 < len(body) and body[j + 1][0] == "retq":
                    continue
                n += 1
            if op in ("pushq", "popq"):
                pushpop += 1
        if n:
            bad.append((name, n))
    print(f"{path}: {len(funcs)} functions, {len(bad)} with rsp changes in the body ({sum(n for _, n in bad)} instructions); push/pop: {pushpop}")
    for name, n in sorted(bad, key=lambda b: -b[1])[:6]:
        print("   ", name, n)
```

### `p169-windows.py`

```python
"""plan-169: classify mid-body rsp-changing windows (sub rsp .. add rsp, and
push .. pop) in windows-x86_64 objdumps by whether a call happens inside the
window. One-off probe. Usage: python3 p169-windows.py <objdump -d>..."""
import re, sys, glob, collections
stats = collections.Counter(); names_call = set(); names_nocall = set()
for path in sys.argv[1:]:
    funcs = []; cur = None
    for line in open(path):
        m = re.match(r"^([0-9a-f]+) <([^>]+)>:", line)
        if m:
            cur = [m.group(2), []]; funcs.append(cur); continue
        m = re.match(r"^\s*[0-9a-f]+:\s+(\S+)\s*(.*)$", line)
        if m and cur:
            cur[1].append((m.group(1), m.group(2).split("#")[0].strip()))
    for name, ins in funcs:
        i = 0
        while i < len(ins) and ((ins[i][0] == "subq" and ins[i][1].endswith("%rsp")) or ins[i][1].startswith("$0x0, (%rsp)")):
            i += 1
        depth = 0; called = False; start_kind = None
        for j in range(i, len(ins)):
            op, a = ins[j]
            opens = (op == "subq" and a.endswith(", %rsp")) or op == "pushq"
            closes = (op == "addq" and a.endswith(", %rsp") and not (j + 1 < len(ins) and ins[j + 1][0] == "retq")) or op == "popq"
            if opens:
                if depth == 0:
                    called = False; start_kind = "push" if op == "pushq" else "sub"
                depth += 1
            elif closes and depth > 0:
                depth -= 1
                if depth == 0:
                    key = (start_kind, "call" if called else "no-call")
                    stats[key] += 1
                    (names_call if called else names_nocall).add(re.sub(r"\$own\d+", "", name))
            elif op.startswith("call") and depth > 0:
                called = True
for k, v in sorted(stats.items()):
    print(k, v)
print("distinct functions with a window spanning a call:", len(names_call))
print("distinct functions with only call-free windows:", len(names_nocall - names_call))
print("examples (call):", sorted(names_call)[:12])
```

