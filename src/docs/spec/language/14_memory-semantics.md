# 14. Memory Semantics

MFBASIC separates value ownership, resource cleanup, and callable storage:

- **Ordinary data** has value ownership. Each live ordinary value is owned by one
  binding, container slot, temporary, closure environment, thread message, or
  return slot, and is reclaimed when that owner is dropped. A closure environment
  can outlive the lexical scope that created it.
- **Resources** have shared views and one assigned cleanup scope (§15). Dropping
  a view is not itself a close or a transfer of cleanup responsibility.
- **Function values** share callable storage. A capturing closure's object and
  environment normally live until arena teardown; dropping one function-value
  binding does not reclaim them. Proven non-retaining temporary callbacks may
  be reclaimed at the call boundary (§14.4). A non-capturing function reference
  uses a static descriptor instead of an allocated closure.
  [[src/codegen/engine/value/builder_values.rs:lower_value]]
  [[src/codegen/engine/value/builder_values.rs:free_call_closure_temps]]

Lexical scope exit deterministically drops that scope's ordinary owned data and
runs its resource cleanup obligations; it does not reclaim every allocation
created in that scope. There is no tracing GC, no reference counting, and no
user-visible `free`.

The compiler may choose stack storage, inline storage, or heap allocation. A `MUT` self-update (`x = collections::append(x, v)`) is always a destructive update in place, with a cost bound (§12), not an optional optimization. None of these choices can change the ownership behavior described here. [[src/codegen/collection/assign/self_update.rs:try_inplace_self_update]]

## 14.1 Copy, move, and freeze

- **Copy** creates independent ordinary data. Resource slots copy a reference to the same resource (handle, `STATE`, and owner); function slots copy the pointer to the same closure object. These explicit sharing cases are described in §15 and `./mfb spec memory closures`.
- **Move** transfers ownership from one place to another. For resources, the static move check invalidates a source name as described in §14.9; this does not imply that the shared resource identity is closed or that its cleanup obligation has changed scope. A later read of a name marked moved is a compile-time use-after-move error. [[src/rules/table.rs:TYPE_USE_AFTER_MOVE]] [[src/ir/verify/resources.rs:check_resource_moves]]
- **Freeze** converts a mutable collection buffer into an immutable owned collection value. The frozen value may be read and copied or moved according to its element type, but it cannot be mutated through the old mutable buffer.

Primitives, `String`, enums, `Nothing`, records whose fields are copyable, and unions all of whose variant payloads are copyable are copyable. `List`, `Set`, and `Map` are copyable when their element/key/value types are copyable; `RES` list elements and map values are also copyable as non-owning resource references (§15.6). Copying ordinary collection contents copies values, while resource slots copy their references and function slots their pointers. Functions and lambdas are copyable only when their captured environment is copyable. Threads and resource handles are not copyable. [[src/ir/verify/resources.rs:is_copyable]]

The compiler may replace a semantic copy with a move when it proves the source is not used afterward. This is an optimization only; it must not change diagnostics or observable behavior except performance.

## 14.2 Assignment and initialization

`LET name = expr`, `MUT name = expr`, record construction, union construction, collection construction, and return-slot initialization all consume the expression result into the destination.

When the expression is a binding:

- If the value's type is copyable and the binding is used again, assignment copies it.
- If the value's type is copyable and the binding is not used again, the compiler may move it.
- If the value's type is not copyable, assignment moves it and the source binding becomes unusable.

Reassigning a `MUT` first drops the old value in the binding, then initializes the binding with the new value. If evaluating the right-hand side fails, the old value remains live.

## 14.3 Function calls and returns

Function arguments are owned values. Passing an argument follows the same copy-or-move rules as assignment. A call cannot observe or mutate a caller-owned value after the argument has been passed, except through a resource reference as described in §15.

This includes a module-level `MUT` passed to a function that reassigns it, directly or through anything it calls (including a callback a built-in invokes): the parameter keeps the value the global had at the call. Native lowering passes a global argument without copying; it copies one into a statement-scope temporary only when the call can reach a reassignment of that global, and assumes it can when a callee's body is not visible. [[src/codegen/engine/value/operand_snapshot.rs:want_arguments_the_call_can_free]] [[src/codegen/engine/value/store_reach.rs:call_reaches_store]]

The same holds within one expression: an operand read out of a module-level `MUT` is the value the global had when that operand was evaluated, even if a later operand of the same call or operator calls a function that reassigns the global (`g = collections::append(g, f())` extends `g` as it was before `f` ran; `f`'s write is lost). Native lowering reads such an operand in place and copies it into a statement-scope temporary only when a later operand can reach a reassignment of that global, assuming it can when a callee's body is not visible; a global holding a resource, and a by-reference, address-taken or resource local, is copied whenever a later operand can run user code. [[src/codegen/engine/value/operand_snapshot.rs:later_operands_can_free]] [[src/codegen/engine/value/store_reach.rs:values_reach_store]]

Returning a value moves it into the caller's return slot. Returning a local collection is valid because ownership leaves the callee before local scope cleanup. Returning a `MUT` collection freezes the mutable buffer into an immutable owned collection value. Returning a non-copyable local value moves it; the callee does not drop that moved-from binding.

A default argument is evaluated on each call that omits the argument, with its names resolved where the function is declared — never in the caller's scope (`mfb spec language functions`) — and is then passed under the same rules as explicit arguments.

## 14.3.1 Native heap value contract

Native backends use one allocator-agnostic IR contract for heap-backed values. The IR names value operations; native lowering chooses whether a value is inline, static, stack-resident, or arena-backed.

This language specification defines the ownership, aliasing, copy, move, and return behavior of heap-backed values; it does not define a universal per-object header or a byte-for-byte native representation for every value kind. Concrete runtime layouts for strings, records, unions, collections — including the arena allocator, its block headers, the per-package-instance arena model, and drop/reclamation at instance shutdown — are specified by the memory spec (`./mfb spec memory heap-values`, `./mfb spec memory arenas`). Source code only observes the value-model rules above: copies are independent, returns never point into a shorter-lived frame or arena, and a value that crosses an execution context (e.g. a thread boundary) must reach storage valid for the receiver before the receiver observes it.

A failed heap allocation surfaces as an ordinary language-level error — `ErrInvalidArgument` for an invalid request and `ErrOutOfMemory` on exhaustion — and auto-propagates like any other failure.

## 14.4 Closures and first-class functions

Closures capture `LET` bindings by value when the closure is created. Capturing a copyable value copies it into the closure environment unless the compiler can move it without changing later validity.

An ordinary closure cannot capture `MUT` or resource bindings. A lambda supplied directly as the `collections::forEach` action may capture a `MUT` by reference for that synchronous call, but still cannot capture resources. [[src/codegen/builtins/mod.rs:is_nonescaping_callback_arg]]

Function values copy a pointer to one closure object; dropping a binding does not reclaim the object or its environment. They normally survive until arena teardown. Direct temporary callbacks proven non-retaining may be reclaimed after the call, including on error exits. See `./mfb spec memory closures`. [[src/codegen/engine/value/builder_values.rs:is_freeable_flat_value]] [[src/codegen/engine/value/builder_values.rs:free_call_closure_temps]]

## 14.5 Recursive unions and allocation

Recursive concrete unions are represented through compiler-managed owned nodes. A recursive edge is an owned child value, not a shared pointer. The compiler rejects value cycles; a program cannot construct a `List`, `Map`, record, or union value that directly or indirectly owns itself.

Independently of this construction-time check, a record type whose fields cycle back to itself without passing through a `List`, `Map`, or `UNION` is rejected at declaration time with `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION` (see §4.2), because such a type has no base case and can never be constructed.

Because cycles are impossible and each edge has one owner, dropping a recursive value recursively drops its owned children without GC or refcounting. Implementations may use iterative drop internally to avoid stack overflow on deeply nested values.

## 14.6 Containers and aliasing

`List` and `Map` own every stored element, key, and value. Inserting into a container copies or moves the inserted value into the container; it never stores a non-owning alias to an external binding. Removing from a container moves the removed value out when the API returns it, or drops it when the API discards it.

Ordinary containers cannot store a `ThreadWorker`, and cannot store resource handles as a `Map` *key* (handles are not comparable, §4.10). A `List` element or `Map` *value*, however, may hold a **reference** to a resource — a parent `Thread` included (§15.6). Such a reference is never an owner: the resource stays owned by a scope, which closes it exactly once on exit; the collection closes nothing, and copying or dropping a collection only copies or discards references. Containers can store functions only when the function value is copyable or movable under the closure rules above.

No two live mutable bindings may refer to the same collection buffer. A collection buffer may be destructively updated only while exactly one binding owns it and that binding's current value is not read again after the update — a single live `MUT` binding, or any owned binding (including a parameter the caller handed over) at its last use. No program can observe the difference from a copy. Reads produce owned values, not aliases into the buffer.

## 14.7 Drop order

At normal scope exit, `RETURN`, `EXIT FOR`/`EXIT DO`/`EXIT WHILE`,
`CONTINUE FOR`/`CONTINUE DO`/`CONTINUE WHILE`, `FAIL`, `PROPAGATE`, or
auto-propagated errors, live bindings are dropped in reverse declaration order
within each scope. `EXIT PROGRAM` is a stack-wide drop edge that unwinds every
live scope up to the entry point before process termination. Nested scopes drop
before enclosing scopes continue. Record fields drop in declaration order. Union
member values drop according to the active member's record layout. List elements
drop from highest index to lowest. Map entries drop in implementation-defined
storage order; programs must not depend on map drop order.

An ordinary value whose ownership moved out is not dropped by its former owner.
For resources, static invalidation of a name does **not** by itself cancel the
resource's scheduled memory reclamation. An explicit close can invalidate a name
while leaving owner-scope reclamation registered; a successful thread transfer
can leave reclamation of the sender's moved record registered without a close.
See §15 for the close/reclamation split and `./mfb spec memory arenas` for the
native reclamation policy.
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:deactivate_moved_resource_arguments]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:retire_moved_resource_cleanup]]
Frozen buffers are dropped as immutable collection values by their final owner.

## 14.8 Diagnostics

The compiler must diagnose:

- Use after move.
- Copy attempts for non-copyable types.
- Cyclic value construction.
- Capturing `MUT` bindings outside the direct `collections::forEach` callback exception.
- Capturing resource handles in ordinary closures.
- Capturing other non-copyable values in ordinary closures.
- Storing a `ThreadWorker` in an ordinary collection, or using a resource handle as a `Map` key.
- Any control-flow path that could drop the same resource or owned value more than once.

`.mfp` packages must preserve enough ownership metadata for import-time checking and Binary Representation semantic verification (see `./mfb spec package verifier-rules`).
At minimum, exported type shape metadata must remain sufficient to reconstruct copyability, resource/thread containment, and drop-sensitive ownership checks when imported packages participate in move analysis.

## 14.9 Move tracking

Use-after-move (§14.1, §14.8) is detected on the **typed IR** by the IR semantic
verifier's resource-move pass — the same checker that runs on decoded `.mfp`
packages, so a crafted package cannot smuggle a double-free past it. The check is
a static binding-validity check: close, return, and direct-name resource rebinds
can mark a source name moved. A move in this check is not necessarily a transfer
of cleanup responsibility: close ends the resource's open state, while a direct
`RES new = old` can copy its reference and invalidate `old` without relocating its
cleanup. The initializer table and cleanup model are specified in §15. Copying
ordinary copyable data is never a resource move. [[src/ir/verify/resources.rs:check_resource_moves]]

State is a single per-block set of moved binding names threaded through the op
list (moved-or-not, not a three-value lattice). Reading a name already in the
set is a use-after-move; the consuming op reads the name too, but it is inserted
into the set only *after* that op, so the consume itself is fine and a *second*
consume (a double close) is what trips. Ordinary moved-name reporting is under `TYPE_USE_AFTER_MOVE`
with the message — `"Binding \`name\` was moved and cannot be used again."`
— regardless of whether the earlier move was definite or path-dependent.

At a branch — each arm of `IF`/`ELSE`, every `MATCH` case, and loop/`TRAP`
bodies — the arm runs against a *clone* of the entering set, and the new moves
it accumulates are unioned back into the outer set only if the arm *falls
through*. An arm that diverges (a top-level `RETURN`/`FAIL`/`EXIT PROGRAM`)
cannot reach the code after the branch, so its moves are dropped from the merge
— close-then-return in one branch does not taint the join. Only moves of
bindings the outer scope knows propagate; branch-local resources die with the
branch. Because "moved on *some* fall-through path" unions into the set, a
subsequent use is rejected exactly as a definite move would be. [[src/ir/verify/resources.rs:check_resource_moves]]

A resource's cleanup scope follows the production, container-routing and
escape rules in §15; merely reading or passing a pointer does not change it.
Any holder — a resource parameter, a `FOR EACH`
element binding, a `RES` binding whose ownership floated to a collection's scope —
may close, `RETURN`, or transfer it. Close and return of a tracked name participate
in the static move check; the owning scope closes an unescaped resource if nobody
already did.

**Transfer is currently enforced dynamically, not by this moved-name set.**
`consumed_resource` deliberately excludes `thread::transfer` (both lowered
resource-plane directions). It does not distinguish a transfer's success and
failure edges, and marking its `CallResult` unconditionally moved would reject
legal reuse in an inline failure handler. Consequently even a direct read after
an untrapped successful transfer can pass verification; a subsequent guarded
resource operation raises `ErrResourceMoved`. This is a static-checking gap, not
permission to use a successfully transferred resource. §15 specifies the runtime
effects and the distinction between inline recovery and function-level cleanup.
[[src/ir/verify/link.rs:consumed_resource]]
[[src/codegen/resource/mod.rs:closed_or_moved_guard]]

Because two names can now denote one resource, a consume through either marks
both: a binding produced by a call returning that same resource type is recorded
as a possible alias of the resource-typed arguments, and closing one name marks
the whole alias set. Nothing is reported at the aliasing point itself — only a
later *use* of a consumed name is. [[src/ir/verify/resources.rs:check_resource_moves]]

## See Also

* ./mfb spec memory heap-values — concrete runtime value layouts
* ./mfb spec memory arenas — arena allocator and reclamation
* ./mfb spec language resource-management — resource ownership and lexical drop
* ./mfb spec architecture native — how these semantics are realized in codegen
