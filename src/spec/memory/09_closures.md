# Closures and Function Values

A function value is the runtime representation of a callable bound to a variable,
passed as an argument, or returned. It is always a **16-byte heap object** with two
words: a code pointer and an environment pointer.

```text
ClosureObject (CLOSURE_OBJECT_SIZE = 16 bytes, arena-allocated, 8-aligned)
  +0   U64  code   ; absolute address of the lifted function body (CLOSURE_OFFSET_CODE = 0)
  +8   U64  env    ; absolute pointer to the capture-slot environment, 0 if none (CLOSURE_OFFSET_ENV = 8)
```

The object is allocated with `arena_alloc(16, 8)` from the constructing scope's
arena. `code` holds the resolved symbol address of the lambda-lifted body; `env`
holds either a pointer to a separate capture environment (a closure with captures)
or the null sentinel `0` (a bare function reference). [[src/target/shared/code/mod.rs:CLOSURE_OBJECT_SIZE]] [[src/target/shared/code/mod.rs:CLOSURE_OFFSET_CODE]] [[src/target/shared/code/mod.rs:CLOSURE_OFFSET_ENV]]

## Function Reference vs Closure

The IR distinguishes two producers of a function value, and the distinction is the
sole determinant of whether an env allocation happens:

- **`FunctionRef`** — a bare reference to a named function with **no captures**. It
  builds the 16-byte object with `code = <symbol>` and `env = 0` (the `x31`/zero
  register is stored into the env word). No separate environment is allocated.
- **`Closure`** — a lifted body **with one or more captures**. It allocates the
  capture environment first, populates its slots, then builds the 16-byte object
  with `code = <symbol>` and `env = <env pointer>`. With an empty capture list a
  `Closure` degrades to the `FunctionRef` shape (env word set to `0`), so an env
  object is produced *only* when there is at least one capture. [[src/target/shared/nir.rs:NirValue]] [[src/target/shared/code/builder_values.rs:268]]

Both forms share the identical 16-byte object layout, so a call site dispatches the
same way regardless of which producer made the value.

## The Capture Environment

The environment is a **separate arena allocation**, distinct from the 16-byte
closure object, sized `captures.len() * 8` bytes and 8-aligned. Each capture
occupies one word at byte offset `index * 8`. [[src/target/shared/code/builder_values.rs:268]]

```text
Environment (arena-allocated, captures.len() * 8 bytes)
  +0      U64  slot[0]
  +8      U64  slot[1]
  ...
  +N*8    U64  slot[N]
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
  callback observes and mutates the live parent binding (a `MUT` slot-borrow). [[src/target/shared/nir.rs:NirValue]] [[src/target/shared/code/builder_values.rs:399]]

A `Capture` read inside the body loads the raw slot word from the active
environment at `index * 8`. For a by-value capture that word is the value/block
pointer directly; for a by-ref capture it is the parent-slot pointer that the
reference local derefs. [[src/target/shared/code/builder_values.rs:399]]

## The Environment Register (x28)

During codegen of a closure body, the reserved register **x28 =
`CLOSURE_ENV_REGISTER`** holds the active closure's environment pointer. Every
`Capture` load reads from `[x28 + index*8]`. [[src/target/shared/code/mod.rs:CLOSURE_ENV_REGISTER]]

x28 is established by the **caller** at the call site, not by the callee prologue.
`emit_function_value_call` loads `code` from `[obj+0]` and `env` from `[obj+8]`,
moves `env` into x28, then `blr code`. Because x28 is reserved and a call may
itself be made from inside an enclosing closure body, the caller **saves its own
x28 to a stack slot before the call and restores it afterward**, so the enclosing
closure's environment survives the nested call. [[src/target/shared/code/builder_misc.rs:emit_function_value_call]]

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

## Closures Across Threads

When a closure is dispatched as a worker thread entry, the thread trampoline
restores the closure environment the same way a normal call does: it loads the
entry closure object from the control block, reads `env` from `[obj + 8]` into x28
and `code` from `[obj + 0]`, and branches to the body — having saved the caller's
x28 (and arena-state register) into the trampoline frame first. The per-worker
arena handling is owned by `./mfb spec threading thread-runtime-helpers`. [[src/target/shared/code/mod.rs:CLOSURE_ENV_REGISTER]]

## See Also

* ./mfb spec memory native-calling-convention — the reserved-register model (x19, x28) and call ABI
* ./mfb spec memory arenas — `arena_alloc`/`arena_free`, deep-copy on capture, scope-drop frees
* ./mfb spec memory heap-values — byte layouts of the String/Record/Error/Result/Union values a closure may capture
* ./mfb spec architecture ir — AST-to-IR lambda lifting and the FunctionRef/Closure/Capture distinction
* ./mfb spec language functions — source-level lambda and closure semantics
* ./mfb spec threading thread-runtime-helpers — closure dispatch on worker threads and per-worker arenas
