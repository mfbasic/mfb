# plan-47-B: Target registration + Windows x64 ABI

Last updated: 2026-07-19
Overall Effort: huge (>3d)
Effort: large (3h–1d) — **over the sub-plan band; split into A1 (ABI realization) / A2 (the 54-method stub wall) / A3 (registration + manifest widening) before starting.**
Depends on: plan-47-A (the exhaustive platform-family match — registration without it gives Windows 20 silent POSIX arms; see the master §3.2)
Supersedes the old `Depends on: nothing` — see the line above.

This sub-plan lands the **codegen half** of the Windows target: a registered
`windows-x86_64` `BuildTarget`, a `src/target/win_x86_64/` backend skeleton, and a
Windows-convention realization of the x86-64 ABI — argument registers
`rcx`/`rdx`/`r8`/`r9`, a mandatory 32-byte shadow space below every outgoing
stack argument, a stack tail past the 4th *external* argument, and a Win64
register model. No PE writer (47-C), no Win32 OS calls (47-D), no `.exe`.

The single checkable behavioral outcome: **for a program lowered through the new
`win_x86_64` codegen platform, a 6-integer-argument external call places its
first four arguments in `rcx`/`rdx`/`r8`/`r9`, its 5th and 6th at `[rsp+32]` and
`[rsp+40]`, and reserves a 16-aligned frame tail of at least 48 bytes — while
`scripts/artifact-gate.sh` reports 0 diffs, i.e. every existing target's emitted
bytes are unchanged.** Both halves are required; the second is not a nicety.

References (read before implementing):

- `planning/plan-47-windows-x86_64.md` §3 item 1 and §4 — the master design this
  expands. Do not contradict it; where this document narrows it (the
  internal/external argument-count split, §4.2 below), that narrowing is
  deliberate and argued.
- `src/target.rs:21` `BuildTarget`, `:102` `NativeBackend`, `:197`
  `NATIVE_BACKENDS`, `:247` `backend_for`, `:93` `BackendCapabilities`.
- `src/target/linux_x86_64/mod.rs:15` `Backend` — the closest precedent (same
  ISA, different OS).
- `src/arch/x86_64/select.rs:67` `CALL_ARGS`, `:68` `SYS_ARGS`, `:75` `RETS`,
  `:80` `map_abi_register`, `:36` `map_scratch_register`, `:107`
  `remap_x86_abi`, `:686` `select_x86`.
- `src/arch/x86_64/regmodel.rs:82` `X86_64RegisterModel`;
  `src/arch/x86_64/backend.rs:23` `impl Backend for X86_64Backend`.
- `src/target/shared/abi.rs:30` `REGISTER_ARGUMENT_COUNT`, `:37`
  `INCOMING_ARGS_BASE`, `:44` `OUTGOING_ARGS_BASE`, `:56`
  `outgoing_stack_arg_store`.
- `src/target/shared/code/codegen_utils.rs:352` `finalize_frame`, `:787`
  `max_outgoing_arg_offset`, `:809` `resolve_stack_arg_sentinels`.
- `src/target/shared/code/mir.rs:533` `trait Backend` (the selection entry point).
- `src/target/shared/code/types.rs:212` `trait CodegenPlatform`.
- `mfb spec memory 06_native-calling-convention`
  (`src/docs/spec/memory/06_native-calling-convention.md`) — the contract this
  plan extends with a Win64 section.
- `AGENTS.md` (the STOP rule on tests/goldens; "a bug you find is a bug you fix")
  and `.ai/compiler.md` (register lifetimes, acceptance obligations).

## Prerequisites

See the master §Prerequisites for the feature-wide gate; and:

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-47-A has landed — registering the target without it gives Windows 29 silent POSIX arms | `rg -n 'enum PlatformFamily' src/` | **NOT MET** |
| Byte-identity goldens exist for every target whose bytes must not change | `find tests -path '*/golden/*' -name '*.ncode*' \| while read f; do b="${f##*/}"; b="${b%.*}"; echo "${b##*.}"; done \| sort -u` | **PARTIAL — `linux-riscv64` has 0** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before you continue and again before you decide to stop. Never act on a
> status you did not just verify. **If you stop, report the status of every row**, not
> only the one that blocked you.

**Row 1 is the one that matters.** This sub-plan's Phase 2 adds `windows-x86_64` to
`NATIVE_BACKENDS`. The instant it does, 29 binary `platform.target()` branches in shared
lowering become reachable and every one resolves to a POSIX arm with no compile error
(master §3.2). Landing 47-A first turns those into 29 compile errors instead.


## 1. Goal

- A `windows-x86_64` `BuildTarget` resolves through `backend_for`
  (`src/target.rs:247`) instead of erroring "native output does not support
  windows-x86_64 yet", backed by `src/target/win_x86_64/mod.rs` advertising
  `executable: false` and an empty `runtime_calls`, so
  `mfb build -target windows-x86_64` fails with the *capability* message
  ("native executable output does not support windows-x86_64 yet",
  `src/target.rs:280`) rather than an unknown-target message.
- A `Win64` variant of the x86 ABI realization, selected only by the new
  backend: call arguments 0–3 in `rcx`, `rdx`, `r8`, `r9`; return in `rax`;
  external arguments ≥ 4 in an outgoing stack tail laid out **above** a 32-byte
  shadow space; a `Win64RegisterModel` whose `external_int_argument_registers()`
  is 4 and whose callee-saved bank is `rbx, rbp, rdi, rsi, r12–r15, xmm6–xmm15`.
- The Win64 realization **rejects** `AbiBoundary::Syscall` with a hard error
  rather than mapping it: Windows has no sanctioned syscall ABI (master §3
  rejected alternative), so a `svc` reaching Win64 selection is a compiler bug
  and must say so, not silently emit a Linux syscall shape.
- Proven by selection/encoder unit tests only. No linker, no executable, no
  runtime.

### Non-goals (explicit constraints)

- **No existing target's emitted bytes may change. Not one.** macos-aarch64,
  linux-aarch64, linux-x86_64 and linux-riscv64 stay byte-identical.
  This is achieved *by construction*, not by inspection: every new seam is a
  `Backend`/`RegisterModel` method with a default that reproduces today's value
  (`shadow_space_bytes() -> 0`, `outgoing_args_base_offset() -> 0`,
  `x86_abi() -> X86Abi::SysV`), and the SysV constants are not edited. The gate
  is `scripts/artifact-gate.sh` reporting 0 diffs, plus a `.ncode` diff for the
  non-host targets (see Validation).
- **No PE/COFF writer, no `write_executable` implementation, no `.exe`.** 47-C.
- **No `CodegenPlatform` OS methods** (`emit_write`, `emit_arena_map`, entry
  path, imports). 47-D. This sub-plan supplies the platform only as far as
  `backend()` and the fields the skeleton needs to compile.
- **No language, IR, NIR, native-plan, MIR-schema, layout, or value-semantics
  change.** The MIR stream this backend consumes is the same OS-neutral stream
  linux-x86_64 consumes.
- **No `syscall` instruction for the Windows target**, ever.
- **No app mode** — `supports_app_mode()` stays `false`.
- **No widening of `REGISTER_ARGUMENT_COUNT`'s meaning for existing targets.**
  It stays the const 8 (`src/target/shared/abi.rs:30`); the Win64 divergence is
  expressed at the *external* boundary only (§4.2).

## 2. Current State

**Target registry.** `BuildTarget { os, arch }` (`src/target.rs:21`) is parsed
from an open `os-arch` string (`:79`) — `"windows-x86_64"` already *parses*; it
fails later at `backend_for` (`:247`), which linear-searches `NATIVE_BACKENDS`
(`:197`, four entries) and errors `"native output does not support {} yet"`. The
CLI parses `-target` at `src/cli/build.rs:165`/`:174` and defaults to the host at
`:226`. A backend advertising `executable: false` is rejected earlier, in
`target::write_executable` (`:280`), with a distinct message — so a registered
but non-executable backend is already a supported, clean state.

**The registry is a derived vocabulary, and widening it is observable.**
`registered_target_oses()` / `registered_target_arches()` (`src/target.rs:211`,
`:225`) feed manifest validation (`src/manifest/mod.rs:647`), and
`registered_targets()` feeds `supported_target_slots()`
(`src/manifest/libraries.rs:253`), which drives a per-slot coverage warning
`NATIVE_LIBRARY_TARGET_UNCOVERED` (`:388`). Registering `windows-x86_64` adds an
8th slot, so **every project with a `libraries` section that does not name
`windows` gains a new warning** — `bindings/sqlite3`, `bindings/libsnd`, and any
fixture whose `build.log` golden captures manifest findings. The spec states the
count literally: "currently **7 slots**"
(`src/docs/spec/package/10_native-bindings.md:55`), and the manifest spec's `os`
row says "`macos` or `linux`"
(`src/docs/spec/tooling/01_project-manifest.md:167`). Three
`supported_target_slots()` tests (`src/manifest/libraries.rs:609`, `:656`,
`:671`) enumerate the matrix. None of this is a reason not to register; it is a
reason registration is its own phase with its own acceptance.

**The x86 ISA layer is OS-neutral and SysV-hardcoded.** `select_x86`
(`src/arch/x86_64/select.rs:686`) selects neutral MIR, then calls `remap_x86_abi`
(`:107`, invoked at `:777`) to resolve the residual AArch64-spelled ABI registers
to SysV homes by control-flow role inference. The homes are three module
constants: `CALL_ARGS = ["rdi","rsi","rdx","rcx","r8","r9","rax","rbp"]` (`:67`),
`SYS_ARGS` (`:68`), `RETS = ["rax","rdx","rcx","rsi"]` (`:75`), consumed by
`map_abi_register` (`:80`). `CALL_ARGS` is already **eight** entries: SysV passes
six, and `rax`/`rbp` are documented at `:59`–`:66` as an *internal-only*
extension for arguments 7 and 8 — sound for the compiler's own calls, wrong for
an external C callee. `map_scratch_register` (`:36`) maps residual AArch64
scratch `xN (N ≥ 9)` through an 11-entry pool ordered so that x19/x20/x27/x28
land on x86 callee-saved `rbp`/`rbx`/`r12`/`r13` (`:48`–`:53`).

**The internal/external argument-count split already exists and is enforced.**
`RegisterModel::external_int_argument_registers()` defaults to
`REGISTER_ARGUMENT_COUNT` (`src/target/shared/regmodel.rs:148`) and
`X86_64RegisterModel` overrides it to 6 (`src/arch/x86_64/regmodel.rs:159`)
precisely because of bug-296. Its one consumer is the LINK thunk
(`src/target/shared/code/link_thunk.rs:661`), which **refuses** a native call
whose integer slot count exceeds that number with an explicit
"stack arguments are not yet staged for native calls" error (`:667`). So today
the external stack tail genuinely does not exist — the master is right about
that — but the *internal* one does, and the machinery is shared.

**The outgoing/incoming stack tail is already ISA-neutral and already works on
x86.** Shared lowering emits `outgoing_stack_arg_store` /
`incoming_stack_arg_load` sentinels (`src/target/shared/abi.rs:56`, `:49`) with
symbolic bases (`"outgoing_args"`, `"incoming_args"`) past
`REGISTER_ARGUMENT_COUNT` (`src/target/shared/code/builder_emit_helpers.rs:81`,
`:87`; `src/target/shared/code/function_lowering.rs:661`, `:672`).
`finalize_frame` (`src/target/shared/code/codegen_utils.rs:352`) reserves
`outgoing_bytes = align(max_offset + 8, 16)` at the frame bottom (`:407`), adds
`frame_call_padding()` (`:395`, 8 on x86-64 —
`src/arch/x86_64/backend.rs:32`), and resolves the sentinels at `:433` via
`resolve_stack_arg_sentinels` (`:809`): outgoing keeps its frame-bottom offset
`[sp + k*8]`, incoming becomes `[sp + frame_size + entry_padding + k*8]`. The
neutral `sp` is rewritten to `rsp` by the x86 remap. **Nothing about this
mechanism is AArch64-specific.** This materially reduces 47-B's work versus the
master's estimate: what is missing for Win64 is not the tail, it is (a) the
32-byte shadow space beneath it and (b) starting the tail at external argument
index 4.

**Register model.** `X86_64RegisterModel` (`src/arch/x86_64/regmodel.rs:82`):
allocatable ints are the tight four `["r10","r11","r12","r14"]` (`:52`) —
`rbx` is `%thread` (`:170`), `r13` is `%closure_env` (`:163`), `r15` is
`arena_base` (`:59`); callee-saved is `["rbx","rbp","r12","r13","r14","r15"]`
(`:70`); every xmm is caller-saved (`:77`, `:103`) because SysV has no
callee-saved FP bank.

**Backend dispatch.** `mir::Backend` (`src/target/shared/code/mir.rs:533`) has
exactly three interesting methods — `select`, `register_model`,
`frame_call_padding` — installed per lowering thread by `set_backend` (`:566`)
from `CodegenPlatform::backend()` (`src/target/shared/code/types.rs:221`), and
read back by `active_backend()` (`:573`) inside `finalize_frame`. **This is the
seam.** A second x86 backend singleton is additive by design: "adding an ISA
needs only a new `impl mir::Backend` plus a platform that returns it — no
shared-code edit at the selection sites" (`types.rs:216`–`:220`).

**Windows is presently only a negative.** `src/os/linux/mod.rs:143` is a unit
test that sets an object plan's `target` to `"windows"` to assert ELF lowering
rejects it. That test belongs to 47-C and this sub-plan does not touch it.

## 3. Design Overview

Four pieces, layered bottom-up. Only the first two touch shared code, and both do
so through defaulted trait methods.

1. **Three defaulted seams on `mir::Backend`** (`mir.rs:533`):
   `x86_abi()` is not needed there — the ABI choice is baked into the *backend
   singleton*, so `select` dispatches it — but `shadow_space_bytes() -> usize`
   (default 0) and `outgoing_args_base_offset() -> usize` (default 0) are, because
   `finalize_frame` is shared. Existing backends inherit 0 and produce today's
   bytes; the Win64 backend returns 32 for both.
2. **An `X86Abi { SysV, Win64 }` parameter inside `src/arch/x86_64/`**, threaded
   from `select_x86(instructions, abi)` into `map_abi_register` and
   `remap_x86_abi`. `SysV` reads today's constants unchanged. A second zero-sized
   backend singleton `Win64Backend` (`src/arch/x86_64/backend.rs`) passes
   `X86Abi::Win64` and returns `Win64RegisterModel`.
3. **`Win64RegisterModel`** — a sibling struct in `src/arch/x86_64/regmodel.rs`,
   not an edit to `X86_64RegisterModel`.
4. **`src/target/win_x86_64/{mod,code}.rs`** — the `NativeBackend` registration
   plus the minimal `CodegenPlatform` returning `Win64Backend`.

**Where the correctness risk concentrates: the shadow space (§4.3).** Everything
else in this sub-plan is a table swap that a unit test pins exactly. The shadow
space is different in kind, because it is a *silent* contract: a Win64 callee is
entitled to spill its four register arguments into `[rsp+0..32]` **of the
caller's frame**, whether or not it has four arguments and whether or not the
caller knows. Omit the reservation and the callee scribbles over whatever the
caller put at the bottom of its frame — on this codebase, the outgoing stack
arguments themselves and the callee-saved save area (`finalize_frame:443`). That
is a corruption with no crash at the point of the bug and no diagnostic. It is
also invisible until 47-D makes a real Win32 call, which is exactly why 47-B must
pin it in a unit test rather than wait for a runtime.

**Rejected alternatives.**

- *Make `REGISTER_ARGUMENT_COUNT` backend-dependent (4 on Win64).* Rejected. It
  is a `const` read by shared lowering at three sites
  (`builder_emit_helpers.rs:81`, `function_lowering.rs:577`, `:661`) that decide
  MIR *before* a backend is consulted, and it defines the compiler's own
  convention — which is internal on every target and already diverges from the
  platform C ABI on x86 (bug-296). Turning it into a query would put a new branch
  in the hottest shared path and make SysV byte-identity a property to be
  verified rather than constructed. The internal convention keeps 8 register
  homes on Windows; only external calls are capped at 4 (§4.2).
- *One backend singleton with a mutable ABI mode.* Rejected: `Backend` is `Sync`
  and every implementor is a zero-sized `static` (`mir.rs:559`–`:561`); a mode
  flag would be thread-shared mutable state on the lowering path.
- *A separate `select_win64` module duplicating selection.* Rejected (master §
  Open Decisions agrees): ~90% of `select_x86` is ISA, not ABI. A duplicate would
  drift, and every future x86 encoder fix would need landing twice.
- *Editing `X86_64RegisterModel` in place with `if win64` branches.* Rejected:
  the model is read on every allocation decision, and an in-place branch makes
  "SysV is unchanged" a claim rather than a fact. Two structs, one shared trait.
- *Deferring target registration to 47-C.* Rejected: without it there is no
  `CodegenPlatform` to hang the backend on and no way to exercise Win64 lowering
  end-to-end from a test, so the ABI work would land unreachable. Registration is
  cheap and its observable consequences (§2, the 8th coverage slot) are better
  absorbed in isolation than mixed into the PE writer's diff.

## 4. Detailed Design

### 4.1 Argument and return homes

| role | SysV (today) | Win64 (new) |
| --- | --- | --- |
| int args 0–3 | `rdi, rsi, rdx, rcx` | `rcx, rdx, r8, r9` |
| int args 4–7 | `r8, r9, rax, rbp` (internal past 6) | internal extension, §4.2 |
| int return | `rax` | `rax` |
| result bank `RETS` | `rax, rdx, rcx, rsi` | `rax, rdx, r8, r9` |
| FP args | `xmm0–7` (`FP_SCRATCH`) | `xmm0–3`, positionally aliased |
| syscall args | `SYS_ARGS` | **rejected — hard error** |

`RETS` must change on Win64: today's `rcx`/`rsi` slots for the 4-register
fallible-result convention (`select.rs:69`–`:74`) collide with Win64 argument
register `rcx`, which would alias the error message onto argument 0 at the next
call. `r8`/`r9` are the natural Win64 replacements — caller-saved, not otherwise
pinned, and the error/TRAP path consumes the bank immediately with no
intervening call (`select.rs:71`–`:73`), so no callee-saved home is needed.

Pinned registers are checked for collision, not assumed clear: `arena_base` =
`r15`, `%thread` = `rbx`, `%closure_env` = `r13`, frame `rbp`. None is a Win64
argument register, and Win64 preserves `rbx`, `rbp`, `r15` (and `r13`, `r12`,
`r14`), so all four survive a `call` into kernel32 for free.

### 4.2 The internal / external argument split

The compiler's own calls keep **8 register homes** on Windows, exactly as they do
on SysV, so no shared-lowering site changes and `REGISTER_ARGUMENT_COUNT` stays
8. Win64 binds only where a real Windows callee is on the other side: the LINK
thunk (`link_thunk.rs:661`) and 47-D's `emit_libc_call` IAT calls.

`Win64RegisterModel::external_int_argument_registers()` returns **4**, mirroring
`X86_64RegisterModel`'s 6 (`regmodel.rs:159`) and the bug-296 reasoning verbatim.
The internal extension for argument indices 4–7 draws from registers that are
neither Win64 argument registers nor pinned. Recommended:
`CALL_ARGS_WIN64 = [rcx, rdx, r8, r9, rdi, rsi, rax, r10]`, with `rdi`/`rsi`
newly excluded from `Win64` allocatable ints (they are Win64 callee-saved, so an
internal callee clobbering them is sound for internal calls but must not be
colored by the allocator when they carry arguments) and `r10` likewise.

That leaves `Win64` allocatable ints as `["r11", "r12", "r14"]` — three, against
SysV's four (`regmodel.rs:52`). This is a real, Windows-only spill-pressure
regression and it is accepted here: correctness first, and the pool is a tuning
knob later (freeing `arena_base` from `r15`, the refinement plan-00-H declined,
would return a fourth). It is recorded as an Open Decision, not buried.

**Consequence for 47-D, stated here because it constrains 47-B's design and must
not be rediscovered later:** the machine floor's `WriteFile(hFile, lpBuffer,
nNumberOfBytesToWrite, lpNumberOfBytesWritten, lpOverlapped)` takes **five**
integer arguments. With `external_int_argument_registers() == 4` and no external
stack tail, `link_thunk`-style staging would put argument 5 in `rdi` and
`WriteFile` would read garbage from `[rsp+32]`. So the external stack tail
(§4.3) is not optional polish deferred to a later letter — the very first Windows
`print` needs it. 47-B delivers it.

### 4.3 Shadow space and the outgoing stack tail

Win64 frame layout at the moment of a `call`, low address first:

```
rsp+0   ┌──────────────────────────────┐
        │ shadow / home space, 32 bytes│  callee may spill rcx,rdx,r8,r9 here
rsp+32  ├──────────────────────────────┤
        │ outgoing stack arg 0  (arg 4)│  [rsp+32]
rsp+40  │ outgoing stack arg 1  (arg 5)│  [rsp+40]
        │ …                            │
        ├──────────────────────────────┤  16-aligned
        │ callee-saved save area       │
        │ locals / spills              │
        └──────────────────────────────┘
```

Two shared-code changes, both defaulted to today's value:

1. `finalize_frame` (`codegen_utils.rs:407`) computes
   `outgoing_bytes = align(max_offset + 8, 16)`; it becomes
   `align(shadow + max_offset + 8, 16)` where
   `shadow = if has_calls { active_backend().shadow_space_bytes() } else { 0 }`,
   and when there is no stack tail at all but the function has calls,
   `outgoing_bytes = shadow` (not 0) — a leaf-calling Win64 frame still owes the
   32 bytes. `shadow_space_bytes()` defaults to 0, so every existing backend
   takes the current arithmetic on the current inputs and emits the current
   bytes.
2. `resolve_stack_arg_sentinels` (`:826`–`:830`) resolves an outgoing sentinel to
   `offset_of(instruction)`; it becomes
   `active_backend().outgoing_args_base_offset() + offset_of(instruction)`,
   default 0. Incoming resolution is unchanged (`frame_size + entry_padding +
   k*8`): our own functions are called by our own code under the internal
   8-register convention, so their incoming tail keeps today's shape. A function
   called *by Windows* — the entry stub and the 47-H thread trampoline — is a
   different contract and is explicitly out of scope here.

`body_shift = outgoing_bytes + save_size` (`:421`) then carries the shadow space
automatically, so locals and spills sit above it with no further change. The
16-alignment invariant the comment at `:403`–`:405` relies on holds: 32 is a
multiple of 16.

**`frame_call_padding()` stays 8 for Win64.** The `call` still pushes an 8-byte
return address and Win64 requires `rsp` 16-aligned at the call site, identically
to SysV; the existing x86 justification (`backend.rs:33`–`:35`) transfers
unchanged.

### 4.4 Register model divergence, and why the clobber masks do *not* change

Win64 callee-saved: `rbx, rbp, rdi, rsi, r12, r13, r14, r15, xmm6–xmm15`.
SysV callee-saved: `rbx, rbp, r12, r13, r14, r15`, no xmm.

**Win64's preserved set is a strict superset of SysV's.** Therefore the existing
caller-saved clobber model — which the allocator derives from
`RegisterModel::caller_saved` (bug-350) — is *conservatively correct* for calls
out to Windows code: it assumes destroyed everything Win64 destroys, plus
`rdi`/`rsi`/`xmm6–15` which Win64 actually preserves. Nothing is under-saved. The
cost is a few unnecessary spills around calls; the benefit is that 47-B does not
have to get a widened clobber mask right to be safe.

The direction that is *not* automatically safe is the reverse — our code being
called **by** Windows, where we must preserve Win64's set. That happens in
exactly two places, both later letters: the program entry (47-D) and the thread
trampoline callback (47-H). This document states the obligation so those letters
inherit it rather than discover it.

`Win64RegisterModel` therefore differs from `X86_64RegisterModel` in four
methods: `external_int_argument_registers()` → 4, `allocatable(Int)` → the
narrowed pool (§4.2), `is_callee_saved` → the Win64 bank, and `caller_saved(Fp)`
→ `xmm0–xmm5` plus the reserved-scratch rules (declaring the true Win64 volatile
FP set; narrowing it is the safe direction). `arena_base`, `closure_env`,
`current_thread`, spill widths and mnemonics are identical.

### 4.5 The scratch-pool audit

`map_scratch_register` (`select.rs:36`) hands residual AArch64 scratch to a fixed
pool whose ordering encodes an assumption stated at `:44`–`:50`: values parked in
`x19`–`x28` must survive an intervening `call`, so those indices land on x86
callee-saved `rbp`/`rbx`/`r12`/`r13`. Those four are callee-saved under Win64
too, so the assumption holds. But the low-scratch remainder includes `rcx`, `r8`,
`r9` — **argument registers under Win64 that were not argument registers 0–2
under SysV**. A machine-floor helper that stages a value into low scratch and
then stages call arguments over it would be corrupted only on Windows.

Phase 1 therefore carries an explicit audit task with a written finding, not an
assumption. Per `AGENTS.md`, if the audit finds a live hazard it is fixed in this
change, not filed.

## Compatibility / Format Impact

**New (all additive):**

- A `windows-x86_64` `BuildTarget`, resolvable by `backend_for`.
- An 8th `TargetSlot` (`windows/x86_64`, no libc axis) in
  `supported_target_slots()` (`src/manifest/libraries.rs:253`), and `windows` in
  the manifest `os` vocabulary (`src/manifest/mod.rs:647`).
- Two defaulted `mir::Backend` methods; one `X86Abi` enum internal to
  `src/arch/x86_64/`; one `Win64RegisterModel`; one `Win64Backend` singleton.

**Changed, observably:** projects with a `libraries` section that name no
`windows` locator gain one `NATIVE_LIBRARY_TARGET_UNCOVERED` warning per library
(`src/manifest/libraries.rs:388`). This is the documented, intended behavior of a
registry-derived matrix ("registering a backend widens the matrix for free",
`:251`), and it is a *correct* new warning — those libraries genuinely have no
Windows locator. Any `build.log` golden that captures it is updated with that
justification recorded in the commit, per the AGENTS.md STOP rule; goldens are
not re-baselined wholesale.

**Unchanged:** every existing target's emitted bytes; `REGISTER_ARGUMENT_COUNT`;
`X86_64RegisterModel`; the SysV `CALL_ARGS`/`SYS_ARGS`/`RETS` constants; the
NIR/native-plan/MIR/object-plan schemas; `EncodedImage`; the `CodegenPlatform`
and `NativePlanPlatform` trait surfaces (no new required methods); the language,
resolver, builtins and IR.

## Phases

### Phase 1 — Audit and the defaulted seams (no behavior change anywhere)

Establishes every hook Win64 needs while every caller still takes today's path,
so this phase is provably byte-neutral on its own.

- [x] Audit `map_scratch_register` (`src/arch/x86_64/select.rs:36`) against the
      Win64 argument bank: for each machine-floor helper and trampoline that
      parks a value in low scratch across a call
      (`src/target/shared/code/runtime_helpers.rs`,
      `runtime_helpers_thread.rs`, `entry_and_arena.rs`), record whether the
      chosen scratch would be clobbered by `rcx`/`rdx`/`r8`/`r9` argument
      staging. Write the finding as a comment at `select.rs:36`. If a live hazard
      exists, fix it in this phase (AGENTS.md: a bug you find is a bug you fix).
- [x] Add `shadow_space_bytes(&self) -> usize { 0 }` and
      `outgoing_args_base_offset(&self) -> usize { 0 }` to `mir::Backend`
      (`src/target/shared/code/mir.rs:533`), each documented with why the default
      reproduces the SysV/AAPCS64 frame exactly.
- [x] Consume them in `finalize_frame`
      (`src/target/shared/code/codegen_utils.rs:407` and the `outgoing_bytes ==
      0 && has_calls` case) and `resolve_stack_arg_sentinels` (`:826`), per §4.3.
- [x] Add `enum X86Abi { SysV, Win64 }` to `src/arch/x86_64/select.rs` and thread
      it through `select_x86`, `remap_x86_abi` and `map_abi_register` as a
      parameter; every existing caller passes `X86Abi::SysV` and every `SysV`
      arm reads the untouched constants.
- [x] Tests: `src/target/shared/code/codegen_utils.rs` unit tests asserting a
      frame with `shadow_space_bytes() == 0` computes the pre-change
      `outgoing_bytes` and sentinel offsets for both the tail and no-tail cases;
      `src/arch/x86_64/select.rs` tests asserting `map_abi_register(n,
      X86Abi::SysV, …)` equals the pre-change values for every `n` and role.

Acceptance: `scripts/artifact-gate.sh target/debug/mfb` reports **0 diffs**;
`cargo test` green; the scratch audit finding is written down (and any hazard it
found is fixed with its own regression test).
Commit: —

### Phase 2 — Target registration and the backend skeleton

Makes `windows-x86_64` a real, resolvable, deliberately non-executable target.
Separately valuable and separately reviewable, with a user-visible outcome.

- [x] New `src/target/win_x86_64/mod.rs`: `pub(crate) static BACKEND: Backend`
      implementing `NativeBackend` (`src/target.rs:102`) with `target()` =
      `{os:"windows", arch:"x86_64"}`, `capabilities()` = all `false` /
      `runtime_calls: &[]`, `supports_app_mode()` = `false`, and every
      `write_*` method returning an explicit
      "not yet supported on windows-x86_64" error naming 47-C/47-D. Mirror the
      shape of `src/target/linux_x86_64/mod.rs:15`.
- [x] Register it in `NATIVE_BACKENDS` (`src/target.rs:197`) and declare the
      module.
- [x] Update the three `supported_target_slots()` tests
      (`src/manifest/libraries.rs:609`, `:656`, `:671`) for the 8th slot, and the
      `parse_accepts_every_registered_target` /
      `backend_for_resolves_every_registered_target` tests (`src/target.rs:480`,
      `:530`) which iterate the registry and need no edit if written generically
      — verify, don't assume.
- [x] Doc sync: `src/docs/spec/package/10_native-bindings.md:55` (7 → 8 slots);
      `src/docs/spec/tooling/01_project-manifest.md:167` (`os` value set gains
      `windows`); any target enumeration in
      `src/docs/spec/tooling/07_cli-reference.md`,
      `src/docs/spec/architecture/01_commands.md` and `.ai/compiler.md`.
- [x] Tests: a `src/target.rs` test asserting
      `BuildTarget::parse("windows-x86_64")` resolves through `backend_for` and
      that `target::write_executable` for it returns the *capability* error
      ("native executable output does not support windows-x86_64 yet"), not the
      unknown-target error.

Acceptance: `mfb build -target windows-x86_64 <any project>` fails with
"native executable output does not support windows-x86_64 yet"; `cargo test`
green; `scripts/artifact-gate.sh` 0 diffs; any `build.log` golden churn is
limited to the new `NATIVE_LIBRARY_TARGET_UNCOVERED` line and justified in the
commit message.
Commit: —

### Phase 3 — `Win64RegisterModel`

A pure data structure with no callers yet, so it lands behind unit tests alone.

- [x] Add `Win64RegisterModel` to `src/arch/x86_64/regmodel.rs` as a sibling of
      `X86_64RegisterModel` (`:82`) — do not edit the SysV model. Diverge in
      `allocatable(Int)` (§4.2), `is_callee_saved` (the Win64 bank incl.
      `xmm6`–`xmm15`), `caller_saved(Fp)`, and
      `external_int_argument_registers()` → 4. Keep `arena_base` = `r15`,
      `current_thread` = `rbx`, `closure_env` = `r13`, `spill_slot_bytes` = 16,
      and the spill/reload/move emitters identical.
- [x] Document on the struct why the *clobber* masks are unchanged (§4.4:
      Win64's preserved set is a superset of SysV's), and the reverse obligation
      inherited by 47-D/47-H.
- [x] Tests: in `src/arch/x86_64/regmodel.rs` — no allocatable register is a
      Win64 argument register or a pinned register; `rdi`/`rsi`/`xmm6`–`xmm15`
      are callee-saved; `external_int_argument_registers() == 4`; and a test
      asserting `X86_64RegisterModel`'s every answer is byte-for-byte what it was
      (the SysV model is untouched).

Acceptance: the new model's tests pass; `scripts/artifact-gate.sh` 0 diffs (the
model has no caller yet, so it must be trivially so).
Commit: —

### Phase 4 — Win64 ABI realization in selection

The table swap. Behind Phase 1's parameter, so SysV cannot be affected.

- [x] Add `CALL_ARGS_WIN64` and `RETS_WIN64` to `src/arch/x86_64/select.rs`
      (§4.1/§4.2), each with a comment naming which entries are the Win64 ABI and
      which are the internal extension (mirroring the `:59`–`:66` precedent).
- [x] Implement the `X86Abi::Win64` arms of `map_abi_register` (`:80`) and
      `remap_x86_abi` (`:107`), including a hard `Err`/`panic!` with a clear
      message on `AbiBoundary::Syscall` — Windows has no syscall ABI, so reaching
      that arm is a compiler bug (§1).
- [x] Add `Win64Backend` to `src/arch/x86_64/backend.rs` mirroring
      `X86_64Backend` (`:17`): `select` → `select_x86(neutral, X86Abi::Win64)`,
      `register_model` → `Win64RegisterModel`, `frame_call_padding` → 8,
      `shadow_space_bytes` → 32, `outgoing_args_base_offset` → 32.
- [x] New `src/target/win_x86_64/code.rs`: a minimal `CodegenPlatform`
      (`src/target/shared/code/types.rs:212`) returning `Win64Backend` from
      `backend()`, `target()` = `"windows-x86_64"`, `arch()` = `"x86_64"` — only
      enough to make Win64 lowering reachable from a test. Every OS-call method
      is 47-D's; leave the trait's own defaults or an explicit
      "47-D" error, never a silent stub.
- [x] Tests: `src/arch/x86_64/select.rs` — a call staging `%arg0`–`%arg3`
      realizes `rcx`/`rdx`/`r8`/`r9`; a result bank realizes `rax`/`rdx`/`r8`/`r9`
      with no `rcx` aliasing; a `svc` under `X86Abi::Win64` errors; and a
      paired assertion that the same input under `X86Abi::SysV` still realizes
      `rdi`/`rsi`/`rdx`/`rcx` and `rax`/`rdx`/`rcx`/`rsi`.

Acceptance: the Win64 selection tests pass, the paired SysV assertions pass, and
`scripts/artifact-gate.sh` reports 0 diffs.
Commit: —

### Phase 5 — Shadow space and the external stack tail (highest-risk work last)

The silent-corruption surface (§4.3), landed last and pinned by an exact-offset
test rather than a shape assertion.

- [x] Verify end-to-end through the Win64 platform that a function containing any
      call reserves ≥ 32 bytes at its frame bottom, that outgoing stack argument
      `k` resolves to `[rsp + 32 + k*8]`, and that `body_shift`
      (`codegen_utils.rs:421`) carries locals and the callee-saved area above it.
- [x] Raise the LINK-thunk external-argument cap for Win64 by staging arguments
      ≥ 4 into the outgoing tail in `src/target/shared/code/link_thunk.rs:661`,
      or — if that staging is larger than this sub-plan — leave the existing
      explicit refusal (`:667`) in place for the Windows target and record it as
      47-D's precondition. Decide against §4.2's `WriteFile` finding, and state
      the decision in the commit; do not leave it implicit.
- [x] Tests: a lowering test through the Win64 platform asserting the exact
      resolved offsets for a 6-integer-argument external call — `rcx`, `rdx`,
      `r8`, `r9`, `[rsp+32]`, `[rsp+40]` — and a total frame ≥ 48 bytes,
      16-aligned; a leaf-calling function (no stack args) still reserving 32; and
      the SysV twin of both asserting `[rsp+0]`/`[rsp+8]` and no shadow
      reservation.

Acceptance: the goal statement holds — a 6-int-argument external call under the
Win64 platform places args at `rcx`/`rdx`/`r8`/`r9`/`[rsp+32]`/`[rsp+40]` with a
≥48-byte 16-aligned tail — **and** `scripts/artifact-gate.sh` reports 0 diffs.
Commit: —

## Validation Plan

- **Tests.** Unit tests only; this sub-plan produces no runnable artifact.
  `src/arch/x86_64/select.rs` (argument/return/scratch realization, the syscall
  rejection, the paired SysV twins), `src/arch/x86_64/regmodel.rs` (both models),
  `src/arch/x86_64/backend.rs` (the two singletons' seam values),
  `src/target/shared/code/codegen_utils.rs` (frame arithmetic with shadow 0 and
  32), `src/target.rs` and `src/manifest/libraries.rs` (registration and the
  widened matrix). Negative cases are first-class: an `svc` under Win64 must
  error; `mfb build -target windows-x86_64` must fail with the capability
  message; a LINK function exceeding the external register count must keep
  failing with an explanatory error, never emit a wrong call.
- **Regression guard — the load-bearing one.** `scripts/artifact-gate.sh
  target/release/mfb` (execution-free, ~5 min, `.ai/compiler.md`'s fast codegen
  gate) after **every** phase, expecting `0 diff(s)`.

  **Corrected 2026-07-20.** The 2026-07-19 draft said the gate "diffs goldens for the
  **host** target only (`:9`–`:10` derive one `TGT`)" and prescribed a manual per-target
  `-ncode -nobj` `cmp` workaround. That is **false**: `scripts/artifact-gate.sh:7`–`:12`
  states in its own header that it is MULTI-TARGET — a `$pkg.linux-aarch64.ncode` golden
  is regenerated with `-target linux-aarch64` even from a macOS host (`ff163ddeb`,
  2026-07-20). The workaround is redundant work built on a false premise; delete it and
  rely on the gate. What the gate genuinely does **not** cover is `linux-riscv64`, which
  has **zero** native goldens (master §Prerequisites row 3) — seed them before any phase
  that edits shared frame code.
- **Runtime proof.** None is possible in 47-B and none is claimed — there is no
  executable until 47-C/47-D. Per `.ai/compiler.md`, compiler plumbing and golden
  output are not proof of runtime support; the Windows runtime claim is made in
  47-D, not here. The behavioral outcome this sub-plan *does* prove is the exact
  register/offset realization asserted in Phase 5.
- **Doc sync.** `src/docs/spec/memory/06_native-calling-convention.md` — add the
  Win64 divergence beside the existing stack-tail section (`:14`–`:38` already
  documents `REGISTER_ARGUMENT_COUNT`, the tail, and the 8-vs-0 entry padding);
  state that the internal convention is unchanged on Windows and that the
  external boundary is 4 registers + 32-byte shadow.
  `src/docs/spec/package/10_native-bindings.md:55` (7 → 8 slots).
  `src/docs/spec/tooling/01_project-manifest.md:167` (`os` gains `windows`). Any
  target list in `src/docs/spec/tooling/07_cli-reference.md`,
  `src/docs/spec/architecture/01_commands.md`, and `.ai/compiler.md`. Keep the
  `[[path:symbol]]` anchors resolving (`.ai/specifications.md`).
- **Acceptance.** `cargo fmt` (a second pass in `repository/`, which is not a
  workspace member), `cargo clippy --all-targets` clean — add any needed
  `#![allow]` *before* running `--fix`, which degrades the fdlibm double-double
  constants — `cargo check --all-targets` clean with no blanket
  `#![allow(dead_code)]`, `cargo test` green, and
  `scripts/test-accept.sh target/debug/mfb target/accept-actual` green (~15 min;
  poll the output file, never rebuild during the run).

## Open Decisions

- **The Win64 internal argument extension and the narrowed allocatable pool.**
  Recommend `CALL_ARGS_WIN64 = [rcx, rdx, r8, r9, rdi, rsi, rax, r10]` with
  allocatable ints `["r11","r12","r14"]` (3, vs SysV's 4), accepting the
  Windows-only spill-pressure cost. Alternative: keep `rdi`/`rsi` allocatable and
  cap the *internal* convention at 6 on Windows, which would require a
  backend-dependent `REGISTER_ARGUMENT_COUNT` — rejected in §3. (§4.2)
- **Where the external stack tail lands.** Recommend implementing it in 47-B
  Phase 5, because `WriteFile`'s five arguments make it a hard precondition of
  47-D's very first `print`. Alternative: leave `link_thunk`'s refusal in place
  and let 47-D do it — acceptable only if Phase 5's frame reservation still
  lands here, since that is the silent-corruption half. (§4.2, Phase 5)
- **`RETS_WIN64` = `[rax, rdx, r8, r9]`** vs. reusing `[rax, rdx, r10, r11]`.
  Recommend `r8`/`r9`: they are caller-saved, unpinned, and the bank is consumed
  with no intervening call, so reusing the argument registers is safe and keeps
  `r10`/`r11` for allocation. Revisit if the error path ever grows a call between
  production and consumption. (§4.1)
- **Whether `Win64Backend` should assert its own non-reachability of `svc` at
  selection time (`panic!`) or thread a `Result`.** Recommend `panic!` with an
  explicit message: `select` returns `Vec<CodeInstruction>`
  (`mir.rs:535`) and threading a `Result` through it is a shared-signature change
  for a case that is unreachable by construction. (§1, Phase 4)

## Summary

The engineering risk here is one thing, and it is not the register table: the
**32-byte shadow space**. Swapping `rdi,rsi,rdx,rcx` for `rcx,rdx,r8,r9` is a
constant a unit test pins exactly and a mistake in it fails loudly. Forgetting
that a Win64 callee may write to the caller's `[rsp+0..32]` fails silently, in
someone else's frame, months later — so it is reserved unconditionally for any
calling frame and asserted at an exact offset in Phase 5.

The second-order finding that shapes this plan: the outgoing/incoming stack-arg
tail is **already ISA-neutral and already functional on x86**
(`codegen_utils.rs:352`–`:436`), so Win64 needs only a shadow offset and a
different external split point, not a new mechanism. And because Win64's
callee-saved set is a strict superset of SysV's, the existing clobber masks are
conservatively correct for calls out to Windows — the reverse obligation (our
code called *by* Windows) is deferred, in writing, to 47-D's entry and 47-H's
thread trampoline.

Untouched: every existing target's emitted bytes, `REGISTER_ARGUMENT_COUNT`, the
SysV register model and constants, the x86-64 encoder, the NIR/plan/MIR
pipeline, and the entire language layer.


## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The `scripts/artifact-gate.sh` claim was false and load-bearing.** This
  document said the gate "diffs goldens for the **host** target only" and prescribed a
  manual per-target `-ncode -nobj` `cmp` workaround. The gate's own header
  (`:7`–`:12`) says MULTI-TARGET in capitals; `ff163ddeb` (2026-07-20) made it so. The
  workaround was redundant work built on a false premise and has been deleted. The real
  gap is `linux-riscv64`, which has **zero** native goldens.
- 2026-07-20 — **"a minimal `CodegenPlatform` … leave the trait's own defaults" does not
  compile.** Only **11 of 65** methods have defaults; **54 are required** (master §2.1).
  This sub-plan must author ~51 stubs, and the 8 `termios_*` plus the offset/constant
  accessors return plain `usize`/`u64` — they cannot carry an "unimplemented" error, so
  they must return fabricated values. 47-D §Phase 3 specifies those as **poison** values
  that crash on use rather than plausible zeros. Give the stub wall its own phase (A2).
- 2026-07-20 — **Effort `large` is over the sub-plan band**; split into A1 (ABI
  realization) / A2 (the stub wall) / A3 (registration + manifest widening) before
  starting.
- 2026-07-20 — **This sub-plan now depends on 47-A.** Registration is what makes 29
  silent POSIX arms reachable; 47-A converts them to compile errors first.
- 2026-07-20 — **`mir::Backend` has 4 methods, not "exactly three interesting" ones.**
  `is_aarch64()` (`mir.rs:541`) is an existing ISA-dispatch hook this document never
  mentions, and it is exactly the kind of seam a new backend must answer.
- 2026-07-20 — **`REGISTER_ARGUMENT_COUNT` is read at 7 shared-lowering sites, not 3**
  (`builder_emit_helpers.rs:81,87,91`; `function_lowering.rs:577,580,661,674`). The
  argument for not making it backend-dependent understates the blast radius it is about.
- 2026-07-20 — **"~90% of `select_x86` is ISA, not ABI" is likely inverted.**
  `select.rs` non-test is 780 lines, of which `remap_x86_abi` (`:107-621`) alone is 515 —
  roughly **71% ABI-specific**. Re-derive before using it to argue the Win64 delta is
  small.
- 2026-07-20 — **The `supported_target_slots()` test enumeration is incomplete.** This
  document names 3 tests (`libraries.rs:609`, `:656`, `:671`); there are **4 distinct
  test fns across 5 call sites** — `:688`
  (`a_wildcard_arch_locator_pinned_to_one_libc_covers_three_slots`) is missed, and `:644`
  /`:656` are the same test. The Phase 2 checklist would skip one.
