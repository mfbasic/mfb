# Native Calling Convention

MFBASIC native functions on AArch64 use a **custom, non-AAPCS64 calling
convention**. The contract in one line: every scalar argument — integer, pointer,
**and** floating/fixed — is passed in a general-purpose register `x0..x7`, there
are no stack arguments, an infallible result comes back in `x0`, and a fallible
result comes back in the four-register form. The deliberate divergences from
AAPCS64 below exist so the whole code plan can treat every value as a single
8-byte slot in a general register; the floating-point register file is touched
only at arithmetic sites.

## Argument Passing

Arguments are assigned to `x0`, `x1`, … `x7` strictly by position, regardless of
type. There are **no stack arguments**: requesting argument index ≥ 8 is a hard
codegen error (`stack arguments are not implemented`). [[src/arch/aarch64/abi.rs:argument_register]] A native function with
more than 8 parameters cannot be lowered.

In the prologue, parameter *N* is simply read from `x{N}`; the parameter's type
plays no part in choosing its register. [[src/target/shared/code/mod.rs:3283]] At a call site, each argument is
lowered, spilled to a stack slot, then reloaded into a scratch register and moved
into the positional `x{index}` — again with no type dispatch. [[src/target/shared/code/builder_misc.rs:emit_prepared_call_args]]

### Float and Fixed arguments go in `x` registers

This is the **critical divergence from AAPCS64**. AAPCS64 passes `double` /
floating arguments in `v0..v7`. MFBASIC does **not**: a `Float` or `Fixed`
argument is passed as its raw 8-byte bit pattern in a general-purpose `x`
register, in the same positional slot as any integer or pointer. `storage_for_type`
classifies both `Float` and `Fixed` as 8-byte / 8-align scalars, and the argument
machinery moves their bits through `x` registers like everything else. [[src/target/shared/plan.rs:storage_for_type]]

The floating-point register file (`d0`, `d1`, …) is used **only at FP-arithmetic
sites**: the operand's bits are moved out of its `x` register with `fmov d, x`
(`fmov_d_from_x`), the `fadd`/`fmul`/etc. runs, and the result is moved back into
an `x` register with `fmov x, d` (`fmov_x_from_d`). [[src/arch/aarch64/abi.rs:float_move_d_from_x]] [[src/target/shared/code/builder_numeric.rs:223]] A `d`-register value never
crosses a call boundary.

## Result Passing

An **infallible** callable returns its single value in `x0` (`RETURN_REGISTER`). [[src/arch/aarch64/abi.rs:return_register]]

A **fallible** callable returns the four-register `{tag, value, message, source}`
outcome in `x0..x3`. That ABI — tags, register roles, the absolute-pointer vs.
block-relative-offset distinction — is owned by `./mfb spec memory
fallible-call-abi` and is not re-tabulated here.

## Storage Classes

Every type is reduced to one `StorageClass` with a fixed size and alignment
before lowering. The taxonomy: [[src/target/shared/plan.rs:StorageClass]] [[src/target/shared/plan.rs:storage_for_type]]

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
user-declared record or union name. [[src/target/shared/plan.rs:is_reference_type]] [[src/target/shared/plan.rs:is_user_type_name]] A resource value (optionally
`RES`-marked, e.g. `RES File`) is also a `Reference` — a pointer to its backing
record — and an unknown type is a hard error. The byte layouts behind a
`Reference` are owned by `./mfb spec memory heap-values`.

`Float` and `Fixed` are distinct classes from `Integer` only so FP-arithmetic
sites know to round-trip through `d` registers; for argument and result passing
all three behave identically (8-byte slot in an `x` register).

## Call Kinds

Each call carries a `CallKind` describing how the target is reached: [[src/target/shared/plan.rs:CallKind]]

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

## Temporary Registers and the No-Spill Limit

Scratch values use a **bump allocator** over a fixed physical map. The builder's
`next_register` counter starts at `8` and increments by one per allocation;
`temporary_register` maps the counter to a physical register: [[src/arch/aarch64/abi.rs:temporary_register]] [[src/target/shared/code/builder_misc.rs:allocate_register]]

```text
allocation  physical     allocation  physical
   8..17     x8..x17        22         x24
   18        x20            23         x25
   19        x21            24         x26
   20        x22            25         x27
   21        x23            26         x28
```

Allocation `8` is `x8`; allocation `17` is `x17`; allocations `18..26` skip the
reserved `x18`/`x19` and map onto the callee-saved `x20..x28`. **There is no
spilling.** Requesting an allocation past `26` (i.e. past `x28`) is a hard error
(`exhausted physical registers`). [[src/arch/aarch64/abi.rs:temporary_register]] This is why a sufficiently deep nested
expression must be broken into separate `LET` bindings — each statement resets the
counter back to `8` (`reset_temporary_registers`), so the limit is per-expression,
not per-function. [[src/target/shared/code/builder_misc.rs:reset_temporary_registers]]

When the allocator hands out a callee-saved register (`x20..x28`), it records it
so the frame finalizer saves and restores it (`mark_register_used`). [[src/target/shared/code/builder_misc.rs:mark_register_used]]

## Reserved Registers

Two registers carry pinned roles in the convention:

* **`x19` — arena-state** (`ARENA_STATE_REGISTER`). Pins the current package
  instance's arena-state for the life of the call chain; never handed out by the
  temporary allocator (the bump map skips `x18`/`x19`). Owned by `./mfb spec
  memory arenas`. [[src/target/shared/code/mod.rs:ARENA_STATE_REGISTER]]
* **`x28` — closure environment** (`CLOSURE_ENV_REGISTER`). Holds the captured
  environment pointer for an `Indirect` (closure) call; owned by `./mfb spec
  memory closures`. [[src/target/shared/code/mod.rs:CLOSURE_ENV_REGISTER]]

Unlike `x19`, `x28` is **not** excluded from the temporary map: it is the highest
register the bump allocator can reach (allocation `26`), so `x28` serves double
duty as both the closure-environment register and the final scratch slot.

## Stack Frame, Prologue, and Epilogue

There is **no `x29` frame-pointer chain**. `finalize_frame` builds the frame once
the body is lowered: [[src/target/shared/code/mod.rs:finalize_frame]]

1. If the body contains **any** `bl`/`blr` and `x30` (the link register) is not
   already in the callee-saved set, `x30` is added to it automatically. [[src/arch/aarch64/abi.rs:link_register]]
2. `save_size = callee_saved.len() * 8`; the total frame is
   `align(save_size + local_stack_size, 16)`, rounded up to **16 bytes**. A
   zero-size frame emits no prologue/epilogue.
3. Layout places **callee-saved registers at the bottom** of the frame
   (`sp+0`, `sp+8`, …) and **local slots above** them — every local slot offset is
   shifted up by `save_size`.

```text
frame layout (higher addresses up)
  sp + total          <- caller's sp
  ...                     local slots (shifted up by save_size)
  sp + save_size      <- first local slot
  sp + (n-1)*8           callee_saved[n-1]
  ...
  sp + 8                 callee_saved[1]
  sp + 0                 callee_saved[0]   <- sp after prologue
```

The prologue is `sub sp, sp, #total` followed by one `str` per callee-saved
register at `sp + index*8`. **Every `ret`** is rewritten to first reload all
callee-saved registers (in reverse), then `add sp, sp, #total`, then return — so
the save/restore is repeated at each return site rather than via a single shared
epilogue. [[src/target/shared/code/mod.rs:finalize_frame]]

The callee-saved set the convention preserves is **`x19..x28`**
(`is_callee_saved`); `x0..x17` and `x30` are caller-saved (`x30` only auto-added
to a function's save set when it makes calls, per step 1). [[src/arch/aarch64/abi.rs:is_callee_saved]]

## See Also

* ./mfb spec memory fallible-call-abi — the four-register `{tag,value,message,source}` result form
* ./mfb spec memory arenas — the `x19` arena-state register and arena mechanics
* ./mfb spec memory closures — the `x28` closure-env register and closure object layout
* ./mfb spec memory heap-values — byte layouts behind a `Reference` pointer
* ./mfb spec memory scalar-storage — scalar value representations
* ./mfb spec memory runtime-helper-abi — `CallKind::Runtime` helper signatures
* ./mfb spec threading thread-runtime-helpers — cross-thread value transfer helpers
* ./mfb spec architecture native — native codegen overview
