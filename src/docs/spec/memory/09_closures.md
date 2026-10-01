# Closures and Function Values

A function value is the runtime representation of a callable bound to a variable,
passed as an argument, or returned. It is a pointer to a **16-byte descriptor** with two
words: a code pointer and an environment pointer. The descriptor is static
for a `FunctionRef` and arena-allocated for a `Closure`.

```text
ClosureObject (CLOSURE_OBJECT_SIZE = 16 bytes, 8-aligned)
  +0   U64  code   ; absolute address of the lifted function body (CLOSURE_OFFSET_CODE = 0)
  +8   U64  env    ; absolute pointer to the capture-slot environment, 0 if none (CLOSURE_OFFSET_ENV = 8)
```

`code` holds the resolved symbol address of the lambda-lifted body; `env` holds
either a pointer to a separate capture environment (a closure with captures) or
the null sentinel `0` (a bare function reference).

**Where the object lives depends on which producer built it.** A capturing
`Closure` is `arena_alloc(16, 8)`-ed from the constructing scope's arena. A
non-capturing `FunctionRef` allocates **nothing**: there is one **static
descriptor per function**, in BSS, populated once at startup, and evaluating the
reference loads its address. [[src/codegen/engine/value/builder_values.rs:NirValue::FunctionRef]] [[src/codegen/error/constants/error_constants.rs:closure_descriptor_symbol]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_OBJECT_SIZE]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_OFFSET_CODE]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_OFFSET_ENV]]

## Function Reference vs Closure

The IR distinguishes two producers of a function value. Only a `Closure` with
non-empty captures allocates an environment:

- **`FunctionRef`** — a bare reference to a named function with **no captures**. It
  loads the address of that function's static BSS descriptor, whose `code` is the
  symbol address and whose `env` is `0`. Nothing is allocated — not the
  environment, and not the 16-byte object either.
- **`Closure`** — a lifted body with an explicit capture list. It allocates the
  capture environment first, populates its slots, then builds the 16-byte object
  with `code = <symbol>` and `env = <env pointer>`. With an empty capture list a
  `Closure` still allocates its 16-byte descriptor but sets its env word to `0`;
  only `FunctionRef` uses the static descriptor. [[src/target/shared/nir/mod.rs:NirValue]] [[src/codegen/engine/value/builder_values.rs:lower_value]]

Both forms share the identical 16-byte object layout, so a call site dispatches the
same way regardless of which producer made the value.

## The Capture Environment

The environment is a **separate arena allocation**, distinct from the 16-byte
closure object, 8-aligned, with
one word per capture at byte offset `index * 8`. Compiler-managed words
follow the capture slots when needed, in this order: the address of the
creator's self-update scratch slot, one capacity-shadow slot address per
shared mutable String capture, and the address of the creator's
`os::appResourcePath` base-cache slot. The allocation size is eight times
the total number of capture and auxiliary words. [[src/codegen/engine/value/builder_values.rs:lower_value]]

```text
Environment (arena-allocated, totalWords * 8 bytes)
  +0      U64  slot[0]
  +8      U64  slot[1]
  ...
  ...         optional auxiliary slot addresses
```

A slot's word holds one of two things, set by the capture's `by_ref` flag:

- **By-value capture (`by_ref = false`).** The slot stores the captured value
  itself — a scalar, or a pointer to a deep-copied flat block. The environment
  outlives the capturing scope, so each by-value capture is materialized through
  `lower_value_owned`, which **deep-copies** an aliasing source so the environment
  independently owns its captured blocks (see `./mfb spec memory arenas`,
  Scope-Drop Frees). The copy's `arena_alloc` clobbers caller-saved scratch
  (including the env register), so the constructor reloads the env pointer from its
  stack slot before each slot store.
- **By-ref capture (`by_ref = true`).** The slot stores a **pointer to the parent
  binding's slot** rather than a value. The capturing body binds a *reference*
  local that dereferences through this pointer on every read and write, so the
  callback observes and mutates the live parent binding (a by-reference `MUT` slot capture). [[src/target/shared/nir/mod.rs:NirValue]] [[src/codegen/engine/value/builder_values.rs:lower_value]]

A `Capture` read inside the body loads the raw slot word from the active
environment at `index * 8`. For a by-value capture that word is the value/block
pointer directly; for a by-ref capture it is the parent-slot pointer that the
reference local derefs. [[src/codegen/engine/value/builder_values.rs:lower_value]]

## The Environment Register

On AArch64, the reserved register **x28**, the realization of
`CLOSURE_ENV_REGISTER`, holds the active closure's environment pointer. Every
`Capture` load reads from `[x28 + index*8]`. [[src/codegen/error/constants/error_constants.rs:CLOSURE_ENV_REGISTER]]

`CLOSURE_ENV_REGISTER` is the neutral `%closure_env` role token,
realized per ISA at selection — AArch64 `x28`, x86-64 `r13`, riscv64 `s10`. Each
`RegisterModel` returns that register from `closure_env()` and **excludes it from
`INT_ALLOCATABLE`** (the mirror of `arena_base`'s exclusion), so the linear-scan
allocator can never color a body virtual register onto it. Without that exclusion
the allocator could place a closure call's freshly-loaded `code` pointer into the
env register, and the `mov %closure_env, <env>` below would overwrite the code
pointer before the indirect call jumped through it.
[[src/target/shared/regmodel.rs:closure_env]]

x28 is established by the **caller** at the call site, not by the callee prologue.
`emit_function_value_call` loads `code` from `[obj+0]` and `env` from `[obj+8]`,
moves `env` into x28, then `blr code`. Because x28 is reserved and a call may
itself be made from inside an enclosing closure body, the caller **saves its own
x28 to a stack slot before the call and restores it afterward**, so the enclosing
closure's environment survives the nested call. [[src/codegen/engine/builder/builder_emit_helpers.rs:emit_function_value_call]]

```text
function-value call (caller side)
  save  x28 -> [sp + saved_env_slot]   ; preserve enclosing closure env
  ldr   code <- [obj + CLOSURE_OFFSET_CODE]
  ldr   env  <- [obj + CLOSURE_OFFSET_ENV]
  mov   x28 <- env                     ; install callee env
  blr   code
  ldr   x28 <- [sp + saved_env_slot]   ; restore enclosing closure env
```

The reserved-register model that pins x28 (alongside x19 for arena state) is owned
by `./mfb spec memory native-calling-convention`.

## Ownership and Reference Semantics

A function value has **reference (pointer) semantics**, not value semantics: there
is one closure object, and every binding, argument, return, collection element, or
map value that holds the function value holds a **copy of the 8-byte pointer** to
that one object. A function value is never deep-copied — assigning it, passing it,
returning it, or storing it into a collection copies the pointer only. This is the
same discipline a resource handle follows (a non-owning pointer, `./mfb spec memory
arenas`), and it is why `lower_value_owned`'s copy-insertion and the
`is_freeable_flat_value` owned-value drop **exclude** function types: there is no
per-value copy on a store and no per-value `arena_free` on scope drop. [[src/codegen/engine/value/builder_values.rs:is_freeable_flat_value]] [[src/codegen/engine/value/builder_values.rs:lower_value_owned]]

The consequence for lifetime is the **arena-lifetime closure rule**: a closure
object (and its capture environment, if any) is owned by the constructing scope's
arena and normally remains until that arena is torn down. The temporary-callback
exception below permits earlier reclamation.
Nothing frees a closure object on a binding's scope exit, so a function value that
has escaped into a longer-lived collection, binding, or returned value stays valid;
correspondingly, a collection of function values frees only its own packed-pointer
block on scope drop and never the closure objects it references. This makes storage
sound (no dangling pointer, no double free) at the cost of not reclaiming an
individual closure before arena teardown.

One exception reclaims a closure individually: a capturing `LAMBDA` written
directly as an argument in a callback position the callee provably never retains
(a non-retaining `FUNC` parameter of a user function, or the
callback of a native `collections` member) is freed — object, environment and the
environment's owned capture copies — by the call it was built for, once that call
has returned. Forwarding is accepted when every destination is itself
non-retaining, with a bounded forwarding-analysis depth of four; an unproven or deeper
forward is not reclaimed by this rule. [[src/codegen/engine/value/builder_values.rs:argument_position_is_nonretaining_callback]] [[src/codegen/engine/value/builder_values.rs:free_call_closure_temps]]
The same closure is freed on every error exit taken after it was built and before
that drain — the callee's failure or a later argument's — since such an exit never
reaches the drain. [[src/codegen/engine/value/builder_values.rs:emit_pending_closure_frees_on_error]]
An inline `TRAP` on the call joins the success path first, so the drain covers it.

Because each *evaluation* of a capturing `Closure` allocates a fresh 16-byte object
plus its environment, repeatedly **creating** capturing closures inside a loop
retains arena memory until teardown unless the temporary-callback rule applies.
A non-capturing `FunctionRef`
does **not**: it resolves to the address of a static descriptor, so evaluating one
in a loop allocates nothing at all.
Building and discarding a *collection* of already-constructed function values in a
loop does **not** grow memory: the collection's own block is reclaimed each
iteration and no new closure objects are produced. Storing existing function
values in a collection does not allocate new
closure objects; newly evaluated capturing lambdas follow the lifetime rules above.

## Closures Across Threads

When a closure is dispatched as a worker thread entry, the thread trampoline
restores the closure environment the same way a normal call does: it loads the
entry closure object from the control block, reads `env` from `[obj + 8]` into x28
and `code` from `[obj + 0]`, and branches to the body — having saved the caller's
x28 (and arena-state register) into the trampoline frame first. The per-worker
arena handling is owned by `./mfb spec threading thread-runtime-helpers`. [[src/codegen/error/constants/error_constants.rs:CLOSURE_ENV_REGISTER]]

## See Also

* ./mfb spec memory native-calling-convention — the reserved-register model (x19, x28) and call ABI
* ./mfb spec memory arenas — `arena_alloc`/`arena_free`, deep-copy on capture, scope-drop frees
* ./mfb spec memory heap-values — byte layouts of the String/Record/Error/Result/Union values a closure may capture
* ./mfb spec architecture ir — AST-to-IR lambda lifting and the FunctionRef/Closure/Capture distinction
* ./mfb spec language functions — source-level lambda and closure semantics
* ./mfb spec threading thread-runtime-helpers — closure dispatch on worker threads and per-worker arenas
