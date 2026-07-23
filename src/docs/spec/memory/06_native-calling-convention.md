# Native Calling Convention

MFBASIC native functions on AArch64 use a **custom, non-AAPCS64 calling
convention**. The contract in one line: every scalar argument — integer, pointer,
**and** floating/fixed — occupies a single 8-byte positional slot, the first
eight are passed in general-purpose registers `x0..x7` and any beyond that in a
stack tail, an infallible result comes back in `x0`, and a fallible result comes
back in the four-register form. The deliberate divergences from AAPCS64 below
exist so the whole code plan can treat every value as a single 8-byte slot in a
general register; the floating-point register file is touched only at arithmetic
sites.

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
parameter. The parameter's type plays no part in choosing its slot. [[src/target/shared/code/function_lowering.rs:lower_function]] At a call
site, each argument is lowered and spilled to a marshalling slot; the first eight
are then reloaded and moved into the positional `x{index}`, and the rest are
stored into the caller's reserved outgoing tail — again with no type dispatch. [[src/target/shared/code/builder_emit_helpers.rs:emit_prepared_call_args]]

The stack tail is realized entirely at frame finalization: a call passing more
than eight arguments reserves a 16-byte-aligned outgoing area at the very bottom
of the caller frame (below the callee-saved registers), and the callee reads its
incoming arguments from just above its own frame, past the entry return-address
padding (8 bytes on x86-64, 0 on AArch64). The register-only path — every call of
eight or fewer arguments — is byte-for-byte unchanged. [[src/target/shared/code/codegen_utils.rs:finalize_frame]]

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
stays resident in `d`-registers** across operations (the FP register class,
the FP register class): a parent float op reads its operand straight from the
`d`-register the child op produced, skipping the GPR round-trip. At every memory
or ABI boundary (storing to a slot, passing an argument, returning) a `Float` is
still its 8-byte little-endian form in an `x` register, so its value
representation and the call ABI are unchanged.

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
  Reference    everything else                8    8
```

`Reference` is a single 8-byte **pointer**, and **all** heap, user, resource, and
composite types collapse to it: `String`, `Error`, every `List OF …`, `Map OF …`,
`MapEntry OF …`, `Result OF …`, `Thread OF …`/`ThreadWorker OF …`, `FUNC(…)` /
`ISOLATED FUNC(…)` closure types, the file/dir resource handles, and any
user-declared record or union name. [[src/target/shared/plan/lower.rs:is_reference_type]] [[src/target/shared/plan/lower.rs:is_user_type_name]] A resource value (optionally
`RES`-marked, e.g. `RES File`) is also a `Reference` — a pointer to its backing
record — and an unknown type is a hard error. The byte layouts behind a
`Reference` are owned by `./mfb spec memory heap-values`.

`Float` and `Fixed` are distinct classes from `Integer` only so FP-arithmetic
sites know to round-trip through `d` registers; for argument and result passing
all three behave identically (8-byte slot in an `x` register).

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
spill slot). The method is a pluggable strategy selected by `--regalloc <name>`
(see `./mfb spec architecture native`). [[src/target/shared/code/builder_registers.rs:allocate_register]] [[src/target/shared/code/regalloc/mod.rs:allocate]]

The default strategy, **`linear-scan`**, computes liveness over the lowered
stream and colors the integer class by live interval, reusing a register as soon
as its previous occupant dies and **spilling to a stack slot under pressure**.
A value whose live range crosses a call is spilled, since no register survives an
internal runtime helper (e.g. `_mfb_arena_alloc` clobbers callee-saved
`x20`–`x28`). Because pressure spills rather than failing, there is **no
"break a deep expression into `LET` bindings" limit** — an arbitrarily nested
expression compiles. [[src/target/shared/code/regalloc/linear_scan.rs:run]]

The reference strategy, **`bump`**, replays the fixed numbering — the
`next_register` counter starts at `8` and `temporary_register` maps it to a
physical register (`8..17` → `x8..x17`; `18..26` → the callee-saved `x20..x28`,
skipping the reserved `x18`/`x19`); allocation past `26` is a hard error. It is
byte-identical to the pre-allocator backend and kept as the differential oracle
(`--regalloc bump`). [[src/target/shared/abi.rs:temporary_register]] [[src/target/shared/code/regalloc/mod.rs:BumpAndReset]]

When the coloring uses a callee-saved register (`x20..x28`), it is recorded so
the frame finalizer saves and restores it. [[src/target/shared/code/builder_registers.rs:mark_register_used]]

## Reserved Registers

Two registers carry pinned roles in the convention:

* **`x19` — arena-state** (`ARENA_STATE_REGISTER`). Pins the current package
  instance's arena-state for the life of the call chain; never handed out by the
  temporary allocator (the bump map skips `x18`/`x19`). Owned by `./mfb spec
  memory arenas`. [[src/target/shared/code/error_constants.rs:ARENA_STATE_REGISTER]]
* **`x28` — closure environment** (`CLOSURE_ENV_REGISTER`). Holds the captured
  environment pointer for an `Indirect` (closure) call; owned by `./mfb spec
  memory closures`. [[src/target/shared/code/error_constants.rs:CLOSURE_ENV_REGISTER]]

Unlike `x19`, `x28` is **not** excluded from the temporary map: it is the highest
register the bump allocator can reach (allocation `26`), so `x28` serves double
duty as both the closure-environment register and the final scratch slot.

### Neutral names in shared lowering

The register names above are the concrete AArch64 realizations. The three registers
whose role is a program- or frame-wide **invariant** are never spelled by their
AArch64 number in shared lowering; each is named by one
neutral token, realized per ISA at selection: [[src/target/shared/code/]]

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
the body is lowered: [[src/target/shared/code/codegen_utils.rs:finalize_frame]]

1. If the body contains **any** `bl`/`blr` and `x30` (the link register) is not
   already in the callee-saved set, `x30` is added to it automatically. [[src/target/shared/abi.rs:link_register]]
2. `save_size = callee_saved.len() * 8`; the frame reserves an **outgoing
   stack-argument tail** of `outgoing_bytes` at its very bottom (the widest call
   that passes more than eight arguments, 16-aligned; zero when no such call
   exists), and the total frame is
   `outgoing_bytes + align(save_size + local_stack_size, 16)`, rounded up to
   **16 bytes** (plus an 8-byte return-address pad on x86-64). A zero-size frame
   emits no prologue/epilogue.
3. Layout places the **outgoing argument tail at the bottom** (`sp+0`, `sp+8`, …),
   the **callee-saved registers above it**, and **local slots above those** —
   every callee-save and local offset is shifted up by
   `outgoing_bytes + save_size`.

```text
frame layout (higher addresses up)
  sp + total          <- caller's sp; incoming stack args live here and up
  ...                     local slots (shifted up by outgoing_bytes + save_size)
  sp + off + save_size <- first local slot   (off = outgoing_bytes)
  sp + off + (n-1)*8      callee_saved[n-1]
  ...
  sp + off + 0           callee_saved[0]
  sp + (m-1)*8           outgoing arg m-1  (this frame's calls write here)
  ...
  sp + 0                 outgoing arg 0    <- sp after prologue
```

The prologue is `sub sp, sp, #total` followed by one `str` per callee-saved
register at `sp + outgoing_bytes + index*8`. **Every `ret`** is rewritten to first
reload all callee-saved registers (in reverse), then `add sp, sp, #total`, then
return — so the save/restore is repeated at each return site rather than via a
single shared epilogue. A callee reads incoming stack argument `k` from
`sp + total + entry_padding + k*8`, where `entry_padding` is the entry
return-address slot (8 on x86-64, 0 on AArch64). [[src/target/shared/code/codegen_utils.rs:finalize_frame]]

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
