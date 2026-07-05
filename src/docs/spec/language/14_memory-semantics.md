# 14. Memory Semantics

MFBASIC values have lexical ownership. Each live value is owned by exactly one binding, container slot, temporary, closure environment, thread message, or return slot. Values are reclaimed by deterministic drop at the end of the owning scope. There is no tracing GC, no reference counting, and no user-visible `free`.

The compiler may choose stack storage, inline storage, heap allocation, or destructive update, but those choices cannot change the ownership behavior described here.

## 14.1 Copy, move, and freeze

- **Copy** creates an independent value with no shared mutable state. Mutating the destination cannot affect the source.
- **Move** transfers ownership from one place to another. After a move, the source binding is uninitialized and any later read, write, capture, comparison, print, return, or drop of that binding is a compile-time use-after-move error, enforced on the IR by `ir::verify`'s `check_resource_moves`. [[src/rules/table.rs:576]] [[src/ir/verify/mod.rs:check_resource_moves]]
- **Freeze** converts a mutable collection buffer into an immutable owned collection value. The frozen value may be read and copied or moved according to its element type, but it cannot be mutated through the old mutable buffer.

Primitives, `String`, enums, `Nothing`, records whose fields are copyable, and unions whose active payload is copyable are copyable. `List` and `Map` are copyable only when their element/key/value types are copyable; copying a collection copies its contents. Functions and lambdas are copyable only when their captured environment is copyable. Threads and resource handles are not copyable. [[src/syntaxcheck/resources.rs:is_copyable_type]]

The compiler may replace a semantic copy with a move when it proves the source is not used afterward. This is an optimization only; it must not change diagnostics or observable behavior except performance.

## 14.2 Assignment and initialization

`LET name = expr`, `MUT name = expr`, record construction, union construction, collection construction, and return-slot initialization all consume the expression result into the destination.

When the expression is a binding:

- If the value's type is copyable and the binding is used again, assignment copies it.
- If the value's type is copyable and the binding is not used again, the compiler may move it.
- If the value's type is not copyable, assignment moves it and the source binding becomes unusable.

Reassigning a `MUT` first drops the old value in the binding, then initializes the binding with the new value. If evaluating the right-hand side fails, the old value remains live.

## 14.3 Function calls and returns

Function arguments are owned values. Passing an argument follows the same copy-or-move rules as assignment. A call cannot observe or mutate a caller-owned value after the argument has been passed, except through a standard resource borrow described in §15.

Returning a value moves it into the caller's return slot. Returning a local collection is valid because ownership leaves the callee before local scope cleanup. Returning a `MUT` collection freezes the mutable buffer into an immutable owned collection value. Returning a non-copyable local value moves it; the callee does not drop that moved-from binding.

Default arguments are evaluated at the call site and then passed under the same rules as explicit arguments.

## 14.3.1 Native heap value contract

Native backends use one allocator-agnostic IR contract for heap-backed values. The IR names value operations; native lowering chooses whether a value is inline, static, stack-resident, or arena-backed.

This language specification defines the ownership, aliasing, copy, move, and return behavior of heap-backed values; it does not define a universal per-object header or a byte-for-byte native representation for every value kind. Concrete runtime layouts for strings, records, unions, collections, and any future heap-backed value category — including the arena allocator, its block headers, the per-package-instance arena model, and drop/reclamation at instance shutdown — are specified by the memory spec (`./mfb spec memory heap-values`, `./mfb spec memory arenas`). Source code only observes the value-model rules above: copies are independent, returns never point into a shorter-lived frame or arena, and a value that crosses an execution context (e.g. a thread boundary) must reach storage valid for the receiver before the receiver observes it.

A failed heap allocation surfaces as an ordinary language-level error — `ErrInvalidArgument` for an invalid request and `ErrOutOfMemory` on exhaustion — and auto-propagates like any other failure.

## 14.4 Closures and first-class functions

Closures capture `LET` bindings by value when the closure is created. Capturing a copyable value copies it into the closure environment unless the compiler can move it without changing later validity.

Capturing `MUT` bindings is a compile-time error because closures do not capture live mutable cells. Capturing resource handles or any other non-copyable values is also a compile-time error in v1 unless a later non-escaping closure feature explicitly defines local borrowing or move rules.

A closure environment is owned by the function value. Dropping the function value drops its captured values in reverse capture order.

## 14.5 Recursive unions and allocation

Recursive concrete unions are represented through compiler-managed owned nodes. A recursive edge is an owned child value, not a shared pointer. The compiler rejects value cycles; a program cannot construct a `List`, `Map`, record, or union value that directly or indirectly owns itself.

Independently of this construction-time check, a record type whose fields cycle back to itself without passing through a `List`, `Map`, or `UNION` is rejected at declaration time with `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION` (see §4.2), because such a type has no base case and can never be constructed.

Because cycles are impossible and each edge has one owner, dropping a recursive value recursively drops its owned children without GC or refcounting. Implementations may use iterative drop internally to avoid stack overflow on deeply nested values.

## 14.6 Containers and aliasing

`List` and `Map` own every stored element, key, and value. Inserting into a container copies or moves the inserted value into the container; it never stores a borrowed alias to an external binding. Removing from a container moves the removed value out when the API returns it, or drops it when the API discards it.

Ordinary containers cannot store thread handles, and cannot store resource handles as a `Map` *key* (handles are not comparable, §4.10). A `List` element or `Map` *value*, however, may hold a **borrow** of a resource — a copy of the one handle pointer (§15.6). Such a borrow is never an owner: the resource stays owned by a scope, which closes it exactly once on exit; the collection closes nothing, and copying or dropping a collection only copies or discards borrows. Containers can store functions only when the function value is copyable or movable under the closure rules above.

No two live mutable bindings may refer to the same collection buffer. A `MUT` collection buffer may be destructively updated only while it is owned by that single live `MUT` binding. Reads produce owned values, not aliases into the buffer.

## 14.7 Drop order

At normal scope exit, `RETURN`, `EXIT FOR`/`EXIT DO`/`EXIT WHILE`, `EXIT SUB`,
`CONTINUE FOR`/`CONTINUE DO`/`CONTINUE WHILE`, `FAIL`, `PROPAGATE`, or
auto-propagated errors, live bindings are dropped in reverse declaration order
within each scope. `EXIT PROGRAM` is a stack-wide drop edge that unwinds every
live scope up to the entry point before process termination. Nested scopes drop
before enclosing scopes continue. Record fields drop in declaration order. Union
member values drop according to the active member's record layout. List elements
drop from highest index to lowest. Map entries drop in implementation-defined
storage order; programs must not depend on map drop order.

Moved-from bindings are not dropped. Frozen buffers are dropped as immutable collection values by their final owner.

## 14.8 Diagnostics

The compiler must diagnose:

- Use after move.
- Copy attempts for non-copyable types.
- Cyclic value construction.
- Capturing `MUT` bindings in closures.
- Capturing resource handles in ordinary closures.
- Capturing other non-copyable values in ordinary closures.
- Storing thread handles in ordinary collections, or using a resource handle as a `Map` key.
- Binding a borrowed collection element of resource type with `RES` (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`), or otherwise treating such a borrow as an owner.
- Any control-flow path that could drop the same resource or owned value more than once.

`.mfp` packages must preserve enough ownership metadata for import-time checking and Binary Representation semantic verification (§21).
At minimum, exported type shape metadata must remain sufficient to reconstruct copyability, resource/thread containment, and drop-sensitive ownership checks when imported packages participate in move analysis.

## 14.9 Move tracking

Use-after-move (§14.1, §14.8) is detected on the **typed IR** by `ir::verify`'s
resource-move pass — the same checker that runs on decoded `.mfp` packages, so
a crafted package cannot smuggle a double-free past it (plan-20). The check is
resource-linearity: a *move* is the transfer of a resource's close obligation —
a call to the resource type's registered close op with the binding as its first
argument, a `RETURN` of a resource binding, or `RES new = old` (which transfers
ownership of `old`). Copying a copyable value is never a move. [[src/ir/verify/mod.rs:check_resource_moves]]

State is a single per-block set of moved binding names threaded through the op
list (moved-or-not, not a three-value lattice). Reading a name already in the
set is a use-after-move; the consuming op reads the name too, but it is inserted
into the set only *after* that op, so the consume itself is fine and a *second*
consume (a double close) is what trips. Reporting is under `TYPE_USE_AFTER_MOVE`
with a single message — `"Binding \`name\` was moved and cannot be used again."`
— regardless of whether the earlier move was definite or path-dependent.

At a branch — each arm of `IF`/`ELSE`, every `MATCH` case, and loop/`TRAP`
bodies — the arm runs against a *clone* of the entering set, and the new moves
it accumulates are unioned back into the outer set only if the arm *falls
through*. An arm that diverges (a top-level `RETURN`/`FAIL`/`EXIT PROGRAM`)
cannot reach the code after the branch, so its moves are dropped from the merge
— close-then-return in one branch does not taint the join. Only moves of
bindings the outer scope knows propagate; branch-local resources die with the
branch. Because "moved on *some* fall-through path" unions into the set, a
subsequent use is rejected exactly as a definite move would be. [[src/ir/verify/mod.rs:check_resource_moves]]

A *borrow* — a resource parameter, a `FOR EACH` element binding, or a `RES`
binding whose ownership floated into a collection — never owns the close
obligation, so closing/returning/transferring one is rejected as
`TYPE_RESOURCE_BORROW_INVALIDATE` rather than tracked as a move. [[src/ir/verify/mod.rs:check_resource_moves]]

## See Also

* ./mfb spec memory heap-values — concrete runtime value layouts
* ./mfb spec memory arenas — arena allocator and reclamation
* ./mfb spec language resource-management — resource ownership and lexical drop
* ./mfb spec architecture native — how these semantics are realized in codegen
