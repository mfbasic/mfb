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
reference loads its address. [[src/codegen/engine/value/builder_values/lower_inner.rs:NirValue::FunctionRef]] [[src/codegen/error/constants/error_constants.rs:closure_descriptor_symbol]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_OBJECT_SIZE]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_OFFSET_CODE]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_OFFSET_ENV]]

## Function Reference vs Closure

The IR distinguishes two producers of a function value. Only a `Closure` with
non-empty captures allocates an environment:

- **`FunctionRef`** — a bare reference to a named function with **no captures**. It
  loads the address of that function's static BSS descriptor, whose `code` is the
  symbol address and whose `env` is `0`. Nothing is allocated — not the
  environment, and not the 16-byte object either.
- **`Closure`** — a lifted body with an explicit capture list. It allocates the
  capture environment first, populates its slots, then builds the 16-byte object
  with `code = <symbol>` and `env = <env pointer>`. A `Closure` with an empty
  capture list (the front end never builds one; it lowers a capture-less lambda to
  a `FunctionRef`) is lowered exactly as a `FunctionRef`, to the static descriptor,
  so `env = 0` marks exactly the function values that own no memory. [[src/target/shared/nir/mod.rs:NirValue]] [[src/codegen/engine/value/builder_values/pending_temps.rs:lower_value]] [[src/codegen/engine/analysis/module_analysis/mod.rs:collect_function_value_refs]]

Both forms share the identical 16-byte object layout, so a call site dispatches the
same way regardless of which producer made the value.

## The Capture Environment

The environment is a **separate arena allocation**, distinct from the 16-byte
closure object, 8-aligned. Its first word is a **header**: the address of the
environment routine of the `LAMBDA` that created it (below). One word per capture
follows, capture `index` at byte offset `(index + 1) * 8`. Compiler-managed words
follow the capture slots when needed, in this order: the address of the
creator's self-update scratch slot, one capacity-shadow slot address per
shared mutable String capture, and the address of the creator's
`os::appResourcePath` base-cache slot. The allocation size is eight times
one more than the total number of capture and auxiliary words. [[src/codegen/engine/value/builder_values/pending_temps.rs:lower_value]] [[src/codegen/error/constants/error_constants.rs:closure_env_word_offset]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_ENV_OFFSET_OPS]]

```text
Environment (arena-allocated, (1 + totalWords) * 8 bytes)
  +0      U64  routine   ; address of _mfb_rt_closure_env_<lambda symbol>
  +8      U64  slot[0]
  +16     U64  slot[1]
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
  callback observes and mutates the live parent binding (a by-reference `MUT` slot capture). [[src/target/shared/nir/mod.rs:NirValue]] [[src/codegen/engine/value/builder_values/pending_temps.rs:lower_value]]

A `Capture` read inside the body loads the raw slot word from the active
environment at `(index + 1) * 8`. For a by-value capture that word is the value/block
pointer directly; for a by-ref capture it is the parent-slot pointer that the
reference local derefs. [[src/codegen/engine/value/builder_values/pending_temps.rs:lower_value]]

## The Environment Register

On AArch64, the reserved register **x28**, the realization of
`CLOSURE_ENV_REGISTER`, holds the active closure's environment pointer. Every
`Capture` load reads from `[x28 + (index+1)*8]`. [[src/codegen/error/constants/error_constants.rs:CLOSURE_ENV_REGISTER]]

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

## Ownership

A function value has **single-owner value semantics**, like every ordinary value
(`./mfb spec language memory-semantics` §14.1, §14.4): every binding, argument,
return, record or union field, `Result` payload, collection element or map value
that holds a function value owns its own closure object and environment. No two
holders share one, so no reference counting is needed and none is done.

- **Copy.** An owning store of a function value that is still read afterwards
  copies it: `_mfb_rt_closure_copy` allocates a new 16-byte object with the same
  `code` over a deep copy of the environment. A store that is its source's last
  read moves it instead (`./mfb spec memory arenas`), and a fresh closure (a
  `LAMBDA` just evaluated, a call's result) is taken over by its first owner. A
  value that holds function values — a record or union with a `FUNC` field, a
  collection of them, a `Result` of one — is copied by the same deep-copy walker as
  a value whose type reaches a cycle, which copies each function value it meets. [[src/codegen/memory/arena/closure_ops.rs:lower_closure_copy]] [[src/codegen/engine/value/builder_values/ownership.rs:needs_graph_copy]] [[src/codegen/memory/arena/builder_arena_transfer/transfer.rs:emit_thread_copy_real]]
- **Drop.** Dropping a holder frees its closure: `_mfb_rt_closure_drop` runs the
  environment's routine to drop it, then frees the 16-byte object. A binding drops
  at its scope's exits, a reassignment drops the old value, and a record, union,
  `Result` or collection drops the function values it holds with itself. [[src/codegen/memory/arena/closure_ops.rs:lower_closure_drop]] [[src/codegen/memory/arena/graph_drop.rs:emit_graph_value_drop]]
- **Arguments are lent.** A callee frees an argument only when its caller hands
  it over (an owned-parameter variant, `./mfb spec memory native-calling-convention`);
  a callee that keeps a lent function argument (stores it in a record, returns it,
  appends it) stores its own copy. So a closure written as an argument is its
  statement's temporary, freed when the statement ends — on every error exit too —
  at every call position, and no caller has to know whether a callee retains it. [[src/codegen/engine/value/builder_values/pending_temps.rs:pending_temp_is_freeable]]
- **Static descriptors are shared.** A `FunctionRef` (`env = 0`) owns no memory:
  copying it returns the same pointer, dropping it does nothing, and it is never a
  statement temporary. A null function value (the default an inline `TRAP` binds
  before its `RECOVER` value, never called) is treated the same way. [[src/codegen/memory/value/builder_value_semantics/default_value.rs:lower_default_value]]

Because a function value owns its closure, a `FUNC` field makes its record (and
anything that holds that record) non-flat: its slot is a pointer edge, and the
record is copied by the walker rather than by one `memcpy`
(`./mfb spec memory heap-values`). [[src/codegen/collection/layout/builder_collection_layout/type_predicates.rs:type_contains_func]] [[src/codegen/collection/layout/builder_collection_layout/type_predicates.rs:record_field_is_pointer]]

**Why copying is sound.** A created closure cannot change: by-value captures are
`LET` values copied into the environment at creation, and the only by-reference
capture (a `MUT` captured by a direct `collections::forEach` action) belongs to a
closure that is never stored, returned or copied. Functions are not comparable, so
no program can tell a copy from the original.

## The Environment Routine

A `FUNC(Float) AS Float` type does not say what a closure captured, so neither the
copy nor the drop can be generated from the holder's type. Each capturing `LAMBDA`
instead gets one routine, `_mfb_rt_closure_env_<lambda symbol>(op, env[, a, b]) -> env`,
emitted once per module from the capture types its creating function recorded,
and the environment's header word points at it: [[src/codegen/memory/arena/closure_ops.rs:lower_closure_env_ops]] [[src/codegen/error/constants/error_constants.rs:closure_env_ops_symbol]]

- `op = 0` (**drop**) drops every owned capture exactly as that capture type's
  owner would (`String`, flat composite, or graph-owning value — a captured
  function value included), then frees the environment block.
- `op = 1` (**copy**) allocates a block of the same size, copies the header and
  every plain word verbatim, and deep-copies every owned capture, returning the new
  block.

A capture word is **owned** when its value is a block the environment holds (a
by-value `String`, flat composite, or graph-owning value) and **plain** otherwise:
a scalar, a resource or thread handle, the parent-slot address of a by-reference
capture, and every auxiliary word (the addresses of slots in the creating frame).
A captured **resource union** is a third kind: the union value is a 16-byte
`{tag@0, record@8}` box that its owning binding or statement frees (`./mfb spec
memory heap-values`, "Union"), so the
environment holds its own copy of the box — copied at capture and with the
environment, freed with it, and never closed through. A `LAMBDA` created at
several sites must have one layout at all of them; the compiler reports an
internal error otherwise. [[src/codegen/engine/value/builder_values/ownership.rs:closure_env_word]] [[src/codegen/engine/value/builder_values/ownership.rs:record_closure_env_layout]] [[src/codegen/engine/builder/mod.rs:ClosureEnvWord]]

### Resource operations

A captured resource is a reference the closure carries (`./mfb spec language
resource-management` §15.6): its cleanup scope follows the closure the way it
follows a `List`. Inside one function that routing is static. Across a function
boundary a `List` is walked by its type, and a `FUNC` type names nothing, so in a
module where some `LAMBDA` captures a value that reaches a resource every
environment routine also answers four resource operations, each walking the
captures that reach a resource and running, per resource, the step the type walk
runs for a `List`. A captured function value forwards the same operation to its
own routine. [[src/codegen/resource/closure_carry.rs:emit_closure_env_resource_op]] [[src/codegen/error/constants/error_constants.rs:CLOSURE_ENV_OP_RELEASE]]

| `op` | Arguments 2, 3 | Per resource |
| --- | --- | --- |
| `2` release | the returning frame's token base | release it to the caller when a scope of that frame owns it (a `RETURN`) |
| `3` close unowned | — | close it when nobody owns it (a discarded call result) |
| `4` module float | — | raise it to the module instance (a store into a module-level binding) |
| `5` adopt | the address of a dynamic owned-list's head word, a scope token | float it to that scope, and list it when the scope takes it (a binding of a call result) |

A binding that holds function values — a `FUNC`, a `List OF FUNC`, a record with a
`FUNC` field — can come to own resources of any kind a closure in the module
carries, which its type cannot name. It gets one **dynamic owned-list** whose
32-byte nodes record their own kind: `{record@0, next@8, kind@16, tag@24}`, the
kind indexing the module's closure-carried resource kinds and the tag naming a
resource-union leaf's active variant. Its drain closes each node's record when the
scope still owns it and reclaims the record's buffers and `STATE`, never the
record itself. A module none of whose `LAMBDA`s captures a resource-reaching value
emits none of this: its routines keep the two operations above. [[src/codegen/resource/closure_carry.rs:emit_dynamic_owned_list_drain]] [[src/codegen/resource/closure_carry.rs:DYNAMIC_NODE_SIZE]]

Each *evaluation* of a capturing `Closure` allocates a fresh object and
environment, and its owner frees both, so creating capturing closures in a loop
does not grow memory. A `FunctionRef` allocates nothing at all.

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
