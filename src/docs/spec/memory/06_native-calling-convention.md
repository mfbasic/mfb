# Native Calling Convention

MFBASIC native functions on AArch64 use a **custom, non-AAPCS64 calling
convention**. The contract in one line: every scalar argument — integer, pointer,
**and** floating/fixed — occupies a single 8-byte positional slot, the first
eight are passed in general-purpose registers `x0..x7` and any beyond that in a
stack tail, an infallible result comes back in `x0`, and a fallible result comes
back in the four-register form. The argument and result boundary uses general-purpose registers even
for floating-point values. Floating-point arithmetic may keep intermediate
values in FP registers. Native C calls use the platform ABI through explicit
marshalling; this internal convention does not define the C boundary.

## Target Register Banks

Shared lowering names `%argMFB0..7` and `%retMFB0..3`; each target realizes
them as follows. An infallible value uses result role 0; the fallible tag,
value, message and source use result roles 0, 1, 2 and 3.

| Target | MFB argument bank (in order) | MFB result bank |
|---|---|---|
| AArch64 | `x0,x1,x2,x3,x4,x5,x6,x7` | `x0,x1,x2,x3` |
| RISC-V 64 | `a0,a1,a2,a3,a4,a5,a6,a7` | `a0,a1,a2,a3` |
| x86-64 SysV | `rdi,rsi,rdx,rcx,r8,r9,rax,rbp` | `rdi,rsi,rdx,rcx` |
| x86-64 Windows | `rcx,rdx,r8,r9,rdi,rsi,rax,rbp` | `rcx,rdx,r8,r9` |

The x86-64 MFB result bank differs from the C return bank (`rax,rdx`).
`%argC`/`%retC` and `%argSys`/`%retSys` explicitly name those other conventions.
The remainder of this topic uses AArch64 instruction/register spellings unless
it gives a target-specific rule.
[[src/target/shared/abi.rs:mfb_arg]] [[src/target/shared/abi.rs:mfb_return]]
[[src/arch/x86_64/select.rs:realize_abi_operand]]
[[src/arch/aarch64/select.rs:select_aarch64]]
[[src/arch/riscv64/select.rs:select_riscv64]]

## Argument Passing

Arguments are assigned to positional slots strictly by position, regardless of
type. The first eight (`REGISTER_ARGUMENT_COUNT`) go in `x0`, `x1`, … `x7`; every
argument at index ≥ 8 goes on the **stack tail** — one 8-byte slot per argument,
in ascending index order, laid out at `[sp+0..]` at the moment of the call
[[src/target/shared/abi.rs:argument_register]] The tail keeps the same one-slot-per-value model as the register
window: a `Float`/`Fixed` stack argument is its raw 8-byte bit pattern, exactly
like an integer or pointer. There is no separate floating-point argument area and
no struct-by-value classification — this is **not** AAPCS64/SysV stack passing.

In the prologue, register parameter *N* (`N < 8`) is read from `x{N}`; a stack
parameter is loaded from the caller's tail (an `sp`-relative load resolved once
the frame size is known) and spilled into its local slot like any register
parameter. The parameter's type plays no part in choosing its slot. [[src/codegen/engine/function/function_lowering/mod.rs:lower_function]] At a call
site, each argument is lowered and spilled to a marshalling slot; the first eight
are then reloaded and moved into the positional `x{index}`, and the rest are
stored into the caller's reserved outgoing tail — again with no type dispatch. [[src/codegen/engine/builder/builder_emit_helpers.rs:emit_prepared_call_args]]

The stack tail is realized entirely at frame finalization: a call passing more
than eight arguments reserves a 16-byte-aligned outgoing area at the very bottom
of the caller frame (below the callee-saved registers), and the callee reads its
incoming arguments from just above its own frame, past the entry return-address
padding (8 bytes on x86-64, 0 on AArch64). Windows additionally reserves its 32-byte shadow area, including for calls
with no stack arguments. [[src/codegen/engine/util/vreg_frame.rs:finalize_frame]]

### Owned-parameter variants

An argument is normally **lent**: the caller keeps its block and the callee only
reads through the pointer. When the caller provably has no further use for the
value, it is **handed over** instead, and the callee's own scope frees it.

A function that any call site hands an argument to is lowered a second time, with
those parameters owned, under an extra internal symbol `<base>$own<mask>` —
`mask` is the hexadecimal bit set of owned parameter indices, so `addOne$own1`
owns parameter 0. It is the same function lowered again, so every source span and
`Error.source` stamp is identical to the base's. Inside the variant an owned
parameter is an ordinary owned local: it carries a scope-drop free, so it is
released on every exit unless it is moved out, and a self-update may mutate its
block. [[src/codegen/engine/function/function_lowering/mod.rs:OwnedVariant]]

At an approved call site the caller stores 0 into the handed-over local's slot,
after every argument has been lowered and immediately before the branch, and calls
the variant symbol. The ordering matters: an argument evaluated later that fails
leaves the call through the error path *before* the store, so the caller still
owns the block and frees it exactly once. After the store the caller's own
scope-drop reads a null slot and skips it, which is the ordinary treatment of a
moved-from binding. [[src/codegen/engine/builder/builder_emit_helpers.rs:emit_raw_call_handing_over]]

An argument that is a **fresh temporary** rather than a local has no caller slot to
null: it is a value the statement just built. It is instead removed from the
statement's pending-temporary list as that one argument is lowered, which is the
only point at which it is that list's most recent entry. A claim deferred until the
whole argument list is lowered would silently miss any position a later argument's
own temporary shadowed, and the statement's end-of-scope drop would then free a
block the callee owns.
[[src/codegen/engine/builder/builder_emit_helpers.rs:emit_prepared_call_args_with_site]]

Which arguments qualify is decided before any code is emitted, and every condition
is a refusal — an argument that is read again on any path, including by a `TRAP`
handler, is lent. So is anything carrying a resource, and so is a bare `String`: a
`String` binding may hold a pointer to a static literal rather than to an arena
block, and freeing that is not permitted. A `String` *field* of a record is
unaffected, because it travels by value inside the record's own block.
[[src/codegen/engine/analysis/handover.rs:collect_handover_args]]

An argument for a parameter the callee **returns as a borrow** is lent too. A
function whose every `RETURN` is a bare parameter it never reassigns, and which is
not reachable as a `FUNC` value, returns that argument's own pointer uncopied; the
caller reads the result as a borrow of its argument, copies it at an owning store,
and goes on freeing the argument itself. That only holds while the caller still
owns the argument's block, so `idOf(squares(20.0))` with `FUNC idOf OF T(x AS T)
AS T` / `RETURN x` lends the temporary and frees it at the end of the statement.
A parameter such a function does not return is handed over as usual.
[[src/codegen/engine/function/function_lowering/local_analysis.rs:param_borrow_returns]]

**The base symbol retains the lent-parameter contract.** It is lowered first, with every parameter lent,
and keeps its name, arity and lent-parameter ABI. A variant is an additional
internal symbol reached only from a direct call the analysis approved.
Function values, `LINK`/export boundaries, thread entries and `.mfp` packages
use the base contract.

### Destination variants

A function whose parameter `j` has the function's own return type — a flat
record: scalars, `String`s and such records, no pointer, collection or resource —
and which returns `WITH <parameter j> { … }` somewhere, and is never assigned and
never has its address taken, may be lowered once more under `<base>$into<j>` when
a list-element update calls it as `inner := f(…, p.inner, …)` (`p` an
element-bound `get` binding, see `./mfb spec memory collections`). Parameter `j`
is still lent: it points at the bytes of `p.inner` inside the list's data region.
At `RETURN WITH <parameter j> { … }` the variant evaluates every update value
first, then — when the result's exact size equals the parameter's — stores them
into the parameter's bytes and returns the parameter's own address; otherwise it
writes nothing and returns a fresh block, exactly as the base would. Every other
`RETURN` returns a fresh block. [[src/codegen/engine/analysis/into_dest.rs:into_param_eligible]]
[[src/codegen/engine/control/builder_exits.rs:try_into_return]]

The caller compares the result with the destination's address: equal means the
element is already updated and nothing is freed; different means a fresh block,
which the update copies or writes back and then frees. Because the variant
writes during the evaluation of the caller's `WITH`, the caller uses it only
when that is unobservable: every earlier update is a scalar value, every later
update reads neither the destination field, the owner whole, the container nor
any binding that may alias the element, and calls only infallible built-ins; and
if a later update can fail, there is no `TRAP` to observe the half-done update
and the container is an owned local of the calling function.
[[src/codegen/engine/control/builder_control.rs:element_into_call]]
Like an owned variant, a destination variant is an additional internal symbol
reached only from such a direct call; the base contract is unchanged.

### Float and Fixed arguments go in `x` registers

This is the **critical divergence from AAPCS64**. AAPCS64 passes `double` /
floating arguments in `v0..v7`. MFBASIC does **not**: a `Float` or `Fixed`
argument is passed as its raw 8-byte bit pattern in a general-purpose `x`
register, in the same positional slot as any integer or pointer. `storage_for_type`
classifies both `Float` and `Fixed` as 8-byte / 8-align scalars, and the argument
machinery moves their bits through `x` registers like everything else. [[src/target/shared/plan/lower.rs:storage_for_type]]

The floating-point register file (`d0`, `d1`, …) holds `Float` values at
FP-arithmetic sites. An operand's bits move out of its `x` register with `fmov d,
x` (`fmov_d_from_x`), the `fadd`/`fmul`/etc. runs, and the result moves back into
an `x` register with `fmov x, d` (`fmov_x_from_d`) for the finiteness check and
the value model. [[src/target/shared/abi.rs:float_move_d_from_x]] Under the linear-scan allocator, **chained float arithmetic
stays resident in `d`-registers** across operations (the FP register class): a parent float op reads its operand straight from the
`d`-register the child op produced, skipping the GPR round-trip. At every memory
or ABI boundary (storing to a slot, passing an argument, returning) a `Float` is
still its 8-byte little-endian form in an `x` register, as required by the argument/result boundary.

## Result Passing

An **infallible** callable returns its single value in `x0` (`RETURN_REGISTER`). [[src/target/shared/abi.rs:return_register]]

A **fallible** callable returns the four-register `{tag, value, message, source}`
outcome in `x0..x3`. That ABI — tags, register roles, the absolute-pointer vs.
block-relative-offset distinction — is owned by `./mfb spec memory
fallible-call-abi` and is not re-tabulated here.

## Storage Classes

Every type is reduced to one `StorageClass` with a fixed size and alignment
before lowering. The taxonomy: [[src/target/shared/plan/mod.rs:StorageClass]] [[src/target/shared/plan/lower.rs:storage_for_type]]

```text
StorageClass   source types                size  align
  Void         Nothing                        0    1
  Boolean      Boolean                        1    1
  Byte         Byte                           1    1
  Integer      Integer                        8    8
  Float        Float                          8    8
  Fixed        Fixed                          8    8
  Money        Money                          8    8
  Scalar       Scalar                         4    4
  Reference    heap/composite/resource        8    8
```

Declared enums use `Integer` storage (8 bytes, alignment 8).
[[src/target/shared/plan/lower.rs:type_storage]]

`Reference` is a single 8-byte **pointer** used by heap, resource, and
composite types: `String`, `Error`, every `List OF …`, `Map OF …`, `Set OF …`,
`MapEntry OF …`, `#Result OF …`, `Thread OF …`/`ThreadWorker OF …`, `FUNC(…)` /
`ISOLATED FUNC(…)` closure types, the file/dir resource handles, and any
user-declared record or union name. [[src/target/shared/plan/lower.rs:is_reference_type]] [[src/target/shared/plan/lower.rs:is_user_type_name]] A resource value (optionally
`RES`-marked, e.g. `RES fs::File`) is also a `Reference` — a pointer to its backing
record — and an unknown type is a hard error. The byte layouts behind a
`Reference` are owned by `./mfb spec memory heap-values`.

Storage classes distinguish the scalar representations for lowering. Every
scalar still occupies one 8-byte positional argument slot; `Fixed` and `Money`
are scaled-integer representations, while `Float` arithmetic uses FP registers.

## Call Kinds

Each call carries a `CallKind` describing how the target is reached: [[src/target/shared/plan/mod.rs:CallKind]]

```text
CallKind    target
  Local     a function compiled in this object (bl <local symbol>)
  Import    a platform/imported symbol (bl <import symbol>)
  Runtime   a generated runtime helper (bl <runtime symbol>)
  Indirect  a computed target — closure/function value (blr <register>)
```

A closure value is a 16-byte object `{code@0, env@8}`; an `Indirect` call loads
the code pointer and branches with `blr`, having placed the captured environment
in `x28` (see Reserved Registers).

## Temporary Registers and Register Allocation

Lowerings do not name physical temporary registers. `allocate_register` mints a
**virtual register** carried in the instruction stream; after a function is fully
lowered, a coloring pass assigns each virtual register a physical register (or a
spill slot). [[src/codegen/engine/regalloc/builder_registers.rs:allocate_register]] [[src/codegen/engine/regalloc/mod.rs:allocate]]
A register operand is a typed `abi::Reg` — a minted virtual register, a
convention-explicit ABI role, or a neutral role token — and has no conversion from text, so a lowering cannot name a register
by hand. [[src/target/shared/abi.rs:Reg]]

The allocator, **linear scan**, computes liveness over the lowered
stream and colors the integer class by live interval, reusing a register as soon
as its previous occupant dies and **spilling to a stack slot under pressure**.
A value must avoid every register a call clobbers if it is live across that
call: an ordinary MFB call preserves the callee-saved registers, so such a value
can stay in one, while the allocator conservatively treats internal `_mfb_*` helpers other
than `_mfb_fn_*` and `_mfb_ifn_*` as clobbering every allocatable integer
register, so an integer value live across one is spilled. The exceptions are the
arena allocator's `_mfb_arena_alloc` and `_mfb_arena_free` and the owned-collection
drop `_mfb_rt_drop_owned_collection`: each is audited, on every target, to save every
callee-saved register it uses, so across them a value keeps a callee-saved register
as it does across an MFB call. FP clobbers use the target's caller-saved set. This is
a compiler model, not a claim that each helper physically overwrites every register.
[[src/codegen/engine/regalloc/analysis.rs:call_clobber_mask]]
[[src/codegen/engine/regalloc/analysis.rs:PCS_CLEAN_RUNTIME_HELPERS]] At `-O1` and above only the
calls a value is **live across** (still needed after the call, by the dataflow)
constrain it; a call inside its interval at which it is dead — the inline raise
path of a checked operation, which calls the error builders and returns — does
not. At `-O0` every call inside the interval counts. [[src/codegen/engine/regalloc/analysis.rs:Liveness]]
Because pressure spills rather than failing, there is **no
"break a deep expression into `LET` bindings" limit** — an arbitrarily nested
expression is not limited by a fixed temporary-register counter. Linear scan
is the allocator. [[src/codegen/engine/regalloc/linear_scan.rs:run]]

The lowering gives every local, parameter and operand temporary its own 8-byte
frame object and stores and reloads the value through it. At `-O1` and above,
**stack slot promotion** runs on the lowered stream just before instruction
selection: a one-word frame object whose every access is a whole-word load or
store at its start, whose address is never taken except by an owned-value drop
helper (which reads and zeroes only that word), which the prologue does not
zero after allocation, and which no path reads before writing, becomes a
virtual register — the stores and loads become register copies, and the
allocator decides whether the value lives in a register or a spill slot. Any
other use of the stack pointer in a function turns promotion off for that
whole function. Register coalescing (a copy whose source dies at it takes the
source's register, and the copy is deleted) also runs from `-O1`. Neither
changes which instructions execute or in what order, only where values live.
[[src/optimizer/opt2/promote.rs:promote]] [[src/codegen/engine/regalloc/builder_registers.rs:run_register_allocation]]

When coloring uses an allocatable callee-saved register, it is recorded so
the frame finalizer saves and restores it. [[src/codegen/engine/regalloc/builder_registers.rs:mark_register_used]]

The save set also covers a callee-saved FP register the body writes by FIXED
name rather than through the allocator. The math and SIMD kernels and the
`Fixed`/`Money`→`Float` conversion stage values in the `FP_SCRATCH`/`VEC_SCRATCH`
tokens, whose indices 6 and 7 realize to `xmm6`/`xmm7` on x86-64. Win64 makes
`xmm6`–`xmm15` callee-saved and the allocator keeps a caller's `Float` in
`xmm6`–`xmm14` across a call, so a Win64 frame saves each of them its stream
writes (16-byte slots). On SysV, AArch64 and RISC-V those realizations are
caller-saved and nothing is added. `xmm15`, the encoder's SSE scratch, is never
an allocator home and is not saved. [[src/codegen/engine/regalloc/linear_scan.rs:fixed_callee_saved_writes]]

## Reserved Registers

Three registers carry pinned roles in the convention:

* **`x19` — arena-state** (`ARENA_STATE_REGISTER`). Pins the current package
  instance's arena-state for the life of the call chain; never handed out by the
  temporary allocator. Owned by `./mfb spec
  memory arenas`. [[src/codegen/error/constants/error_constants.rs:ARENA_STATE_REGISTER]]
* **`x28` — closure environment** (`CLOSURE_ENV_REGISTER`). Holds the captured
  environment pointer for an `Indirect` (closure) call; owned by `./mfb spec
  memory closures`. [[src/codegen/error/constants/error_constants.rs:CLOSURE_ENV_REGISTER]]

* **`x20` — current worker thread** (`abi::CURRENT_THREAD`, `%thread`).
  The worker trampoline pins its control-block pointer here. The register
  allocator excludes it; the main-thread entry may use the corresponding
  scratch role, so code that must distinguish main from worker reads
  `ArenaState.workerThread`. The other targets realize `%thread` as `rbx`
  (x86-64) or `s2` (RISC-V).
  [[src/target/shared/regmodel.rs:current_thread]]
  [[src/arch/aarch64/regmodel.rs:current_thread]]
  [[src/arch/x86_64/regmodel.rs:current_thread]]
  [[src/arch/riscv64/regmodel.rs:current_thread]]

`x28` also realizes the `%closure_env` role token, and the allocator never
hands it out as a temporary — the mirror of `x19`'s arena-base exclusion.

### Neutral names in shared lowering

The register names above are the concrete AArch64 realizations. The three registers
whose role is a program- or frame-wide **invariant** are never spelled by their
AArch64 number in shared lowering; each is named by one
neutral token, realized per ISA at selection: [[src/codegen/]]

| role | token | AArch64 | RISC-V | x86-64 |
|---|---|---|---|---|
| zero register | `abi::ZERO` (`xzr`) | `xzr` (`x31`) | `zero` | none — pins `r14`, or a "no register" sentinel |
| link register | `abi::LR` (`lr`) | `x30` | `ra` | none — `call` pushes the return address |
| arena base | `abi::ARENA` (`arena_base`) | `x19` | `s11` | `r15` |

The per-platform backends and the encoders' input
language still accept the bare AArch64 spellings; only shared lowering routes
through the tokens. [[src/target/]]

## Stack Frame, Prologue, and Epilogue

There is **no `x29` frame-pointer chain**. `finalize_frame` builds the frame once
the body is lowered: [[src/codegen/engine/util/vreg_frame.rs:finalize_frame]]

1. A body containing a direct or indirect call adds the link-register role
   to its save set if it is absent.
2. The save area uses 8-byte slots for integer registers and 64-bit FP
   registers, but 16-byte, 16-aligned slots for SIMD128 callee saves. Its
   total `save_size` is rounded up to 16.
3. A calling Windows frame reserves 32 bytes of shadow space. With an outgoing
   stack argument at maximum byte offset `k`, `outgoing_bytes` is
   `align(shadow + k + 8, 16)`; without stack arguments it is `shadow`.
   Other targets have zero shadow space.
4. `total = outgoing_bytes + align(save_size + local_stack_size, 16)
   + call_padding`. `call_padding` is 8 for a calling x86-64 function and 0
   otherwise. A zero-size frame needs no allocation or restore.
5. Outgoing arguments occupy the bottom region (after Windows shadow space),
   followed by the aligned save area, then locals and spills. Existing local
   offsets are shifted by `outgoing_bytes + save_size`. Incoming argument `k`
   is addressed above the whole frame, the target's entry return-address
   padding, and any shadow space.

```text
frame layout (higher addresses up)
  incoming stack arguments
  caller shadow space (Windows)
  entry return address (x86-64)
  call-alignment padding, if required
  locals and spills
  callee-save area (per-register width/alignment; total rounded to 16)
  outgoing stack arguments
  outgoing shadow space (Windows)       <- sp after prologue
```

The prologue allocates the frame and saves each register at its computed
save-area offset. Windows frames larger than a stack-probe page allocate and
touch successive pages before allocating the remaining bytes. Every return
restores the saved registers in reverse order, releases the frame, and returns.
[[src/codegen/engine/util/vreg_frame.rs:finalize_frame]]
[[src/codegen/engine/util/vreg_frame.rs:push_frame_allocation]]

The callee-saved set the convention preserves is **`x19..x28`**
(`is_callee_saved`); `x0..x17` and `x30` are caller-saved (`x30` only auto-added
to a function's save set when it makes calls, per step 1). [[src/target/shared/abi.rs:is_callee_saved]]

## See Also

* ./mfb spec memory fallible-call-abi — the four-register `{tag,value,message,source}` result form
* ./mfb spec memory arenas — the `x19` arena-state register and arena mechanics
* ./mfb spec memory closures — the `x28` closure-env register and closure object layout
* ./mfb spec memory heap-values — byte layouts behind a `Reference` pointer
* ./mfb spec memory scalar-storage — scalar value representations
* ./mfb spec memory runtime-helper-abi — `CallKind::Runtime` helper signatures
* ./mfb spec threading thread-runtime-helpers — cross-thread value transfer helpers
* ./mfb spec architecture native — native codegen overview
