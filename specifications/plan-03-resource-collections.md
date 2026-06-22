# MFBASIC Resources In Collections Plan

Last updated: 2026-06-21

This document records why resources cannot currently be stored in `List` values
or `Map` values, what the runtime resource model actually is after the
resource overhaul, and a concrete design for allowing resources to be **owned by
a collection** without weakening single-ownership or deterministic cleanup.

It complements:

- `specifications/mfbasic.md` (§12 collections, §14 ownership, §15 resources)
- `specifications/standard_package.md` (§ collection helpers)
- `specifications/memory_layouts.md`
- `specifications/error_codes.md`

## 1. Goal

Allow a collection to own resources, using the language's actual collection
model (free functions, no indexing, no mutating methods):

```basic
MUT pool AS List OF Socket = []
pool = append(pool, net::connect(host, port))   ' the connection now lives with `pool`
FOR EACH conn IN pool                            ' borrow each element
    net::send(conn, payload)
NEXT
' every still-present Socket is closed when `pool` drops
```

without:

- introducing a second owner of any resource (single-ownership invariant),
- introducing a tracing GC, reference counting, or per-value drop flags,
- losing deterministic, exactly-once close on every exit path,
- silently shortening a resource's lifetime.

`Map` **values** are in scope. `Map` **keys** are out of scope: a resource handle
is not comparable or hashable (`mfbasic.md` §4), so it can never be a key.

## 2. Current State

Both the spec and the compiler forbid resources (and thread handles) in ordinary
collections, transitively:

- `mfbasic.md` §14.6: "Ordinary containers cannot store resource handles or
  thread handles." §14.8 lists it as a required diagnostic; §15 forbids resource
  fields in records.
- The check is one shared predicate, `contains_resource_or_thread()`
  (`src/typecheck.rs`), called on `List` element types and `Map` key/value types
  at both literal-inference and type-declaration sites; it recurses through nested
  `List`/`Map`/`Result`/record/union.
- The diagnostic is `TYPE_COLLECTION_OWNERSHIP_VIOLATION` (`2-203-0056`,
  `src/rules.rs`).

This is a deliberate invariant, not an unfinished corner.

## 3. The Runtime Resource Model (what actually exists)

The resource overhaul (Phases 1–6, plus native `LINK` resources) implemented the
entire resource lifecycle on two **compile-time, name-keyed** mechanisms. There
is **no runtime registry of live resources.**

**Representation.** A resource value is a pointer to a single 24-byte arena
record (`src/target/shared/code/mod.rs`):

| offset | field |
|--------|-------|
| 0 (`FILE_OFFSET_FD`) | host handle word (fd / socket / native `CPtr`) |
| 8 (`FILE_OFFSET_CLOSED`) | closed flag |
| 16 (`FILE_OFFSET_STATE`) | `STATE` payload pointer (null until default-init) |

`RESOURCE_RECORD_SIZE = 24`. There is exactly **one record per resource**.

**Borrow = pointer aliasing.** Passing a `RES` binding to an ordinary function is
a call-scoped borrow (`ExprMode::Borrow`, the default): the callee gets the same
pointer and shares the same record and `STATE`. The caller's binding stays live;
ownership does not move. A borrowed parameter (`borrowed: true`) may use and
mutate `STATE` but may not close, `RETURN`, or `thread::transfer` the handle
(`TYPE_RESOURCE_BORROW_INVALIDATE`).

**Liveness/move = per-named-binding state machine.** Each local carries an
`OwnershipState` (`Available` / `Moved` / `MaybeMoved`), keyed by binding name.
Four invalidation events flip a binding to `Moved` via `consume_local_if_needed`:
the registered close op and its re-export aliases; `thread::transfer`; `RETURN`
of the resource; scope drop.

**Drop = static, name-keyed cleanup stack.** A `RES` binding pushes
`ActiveCleanup::Resource { name, symbol }` (or `ResourceUnion`) onto
`active_cleanups`; scope exit statically unrolls that stack in reverse, emitting
one close call per entry. Close / transfer / return deactivates the entry **by
name** (`deactivate_resource_cleanup`). Resource unions drop by tag-dispatch.

**Collections do no per-element drop.** Collections are arena-backed; element
teardown is bulk memory release at package-instance shutdown
(`memory_layouts.md`; `mfbasic.md` §14.3.1). There is **no** code path that calls
a close op per collection element. Closing a resource sets the closed flag and
releases the OS handle but does **not** free the 24-byte record.

## 4. The Real Obstacle: The Collection API Is Value-Semantic

`mfbasic.md` §12 and `standard_package.md` define collections with **no indexing
brackets and no mutating methods** — all access is free functions, and every
update helper is a **pure function that returns a new collection**:

```
append   OF T(value AS List OF T, item AS T)                AS List OF T
prepend  OF T(value AS List OF T, item AS T)                AS List OF T
insert   OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T
set      OF T(value AS List OF T, index AS Integer, item AS T) AS List OF T
removeAt OF T(value AS List OF T, index AS Integer)         AS List OF T
get      OF T(value AS List OF T, index AS Integer)         AS T
```

The idiom `pts = append(pts, v)` is **pure** semantically; the compiler performs
the update destructively in place only as an optimization of the
assign-back-to-the-same-`MUT` pattern (§12). The pure reading — "produce a new
list while the input list stays valid" — assumes the element type is **copyable**.
Resources are not copyable, so the value-semantic reading is impossible for them.

This is the core tension, and it is not the drop walker I emphasized in earlier
analysis. The consequences:

- `get(xs, i) AS T` returning an owned resource would create a second owner — not
  allowed. For a resource element it must be a **borrow**.
- `removeAt(xs, i)` returns the list without the item, and the removed item is
  **dropped** — i.e. for a resource it is **closed** (`mfbasic.md` §14.6: "Removing
  from a container moves the removed value out when the API returns it, or drops
  it when the API discards it"). There is currently **no helper that hands a
  removed element back**, so there is no move-out-into-a-`RES`-binding path.
- `filter`, `mid` produce sub-collections by duplicating survivors → impossible.
- `find`, `contains`, `replace` require comparable `T` → impossible.

So resources can only live in a collection if that collection is treated as a
**linear (move-only) value**, threaded single-owner through every operation, with
each helper re-specified under move/borrow rules.

## 5. Two Candidate Models

### 5.1 Borrowing collection (rejected)

The collection stores a borrow (pointer alias); the original `RES` binding stays
the owner. The fatal problem is **lifetime**: a stored borrow can outlive the
owner. In this arena design that is memory-*safe* (use-after-close reads the
closed flag → `ErrResourceClosed`), but it converts the static "a borrow never
outlives its owner" guarantee into a runtime error and requires every operation
reached through a borrow to check the closed flag. Rejected for v1.

### 5.2 Owning, linear collection (recommended)

Adding a resource to a collection is a **move**: ownership transfers from the
binding into the collection; the collection is non-copyable and must be threaded
single-owner; it closes each still-present element when it drops. This preserves
single-ownership (exactly one owner at every instant) and needs no runtime
liveness net. It is the only model compatible with §4.

## 6. Resource-Owning Collections Are Linear

A `List OF <Res>` or `Map OF <K>, <Res>` (with `<K>` an ordinary comparable
non-resource key) is **non-copyable** (consistent with "`List`/`Map` are copyable
iff elements are copyable") and therefore **linear**:

- Each update helper **consumes** the input collection and produces the successor;
  the only sound usage is rebinding to the same `MUT` binding
  (`xs = append(xs, r)`) or otherwise consuming the result (pass / return).
- Using such a binding twice, or `LET snap = xs` while `xs` stays live, is
  rejected (would copy a non-copyable collection).
- A `LET`-bound resource collection may be built once and thereafter only
  consumed once (passed, returned, or iterated by borrow then dropped).

The compiler already tracks the per-binding move state this requires
(`OwnershipState`); the new work is allowing a *collection* binding to be
non-copyable and routing the helpers below through consumption.

## 7. Per-Helper Behavior For A Resource Element Type

Using the real signatures from `standard_package.md`:

| helper | behavior on a resource collection |
|--------|-----------------------------------|
| `append`, `prepend`, `insert` | **move-in**: consume `value`, consume `item` (move the resource in), return successor. Subject to the scope rule (§8). |
| `set` | consume `value`, move `item` in, **close the displaced element**, return successor. |
| `removeAt`, `removeKey` | consume `value`, **close the removed element** (API discards → close), return successor. This is the "close one early" path. |
| `get`, `getOr` | yields a **borrow** of the element (call-scoped); not bindable as an owning `RES`. |
| `FOR EACH x IN xs` | binds `x` as a **borrow** of each element; collection retains ownership; read / `STATE`-mutate allowed, no invalidation. |
| `len`, `isEmpty`, `isNotEmpty`, `hasKey`, `keys` | allowed (do not move elements; `keys` returns `List OF K`, `K` resource-free). |
| `transform`, `reduce` | allowed only when the produced element/accumulator type is resource-free (borrow each element, derive data). Borrow the input; do not consume. Optional for v1. |
| `values` | rejected: would yield `List OF <Res>` duplicating ownership. |
| `find`, `contains`, `replace` | rejected: require comparable `T`. |
| `mid`, `filter` | rejected: duplicate survivors into a new collection. |
| `sum` | N/A (numeric). |

Because the collection is always the single live owner during a `get`/`FOR EACH`
borrow, those borrows are statically call/loop-scoped and safe **without** any
runtime closed-flag dependency (unlike §5.1).

## 8. The Scope Direction Is Load-Bearing

Move-in is memory-safe in both scope directions but wrong in one:

- **Into a longer-lived (outer) collection — extends lifetime** (intended: pool /
  registry / batch). The resource closes when the outer collection drops.
- **Into a shorter-lived (inner) collection — shortens lifetime.** Still
  single-owner and safe, but the resource closes at the inner collection's drop,
  earlier than its declaration scope, and cannot be recovered — a silent footgun.

**Rule.** A move-in may transfer ownership to a collection only when the
collection binding's scope is the same as, or outlives, the moved resource's
current scope. Upward/same-scope allowed; downward rejected
(`TYPE_RESOURCE_MOVE_NARROWS_SCOPE`, new). This needs a capability the compiler
lacks today — it tracks moved-from per binding but performs **no scope/escape
comparison** — so the escape check is genuinely new static work. (Plain move
needs no new analysis; the upward constraint is the new part.)

## 9. The Move-Out Gap

The existing API cannot move a resource **out** of a collection into a `RES`
binding: `removeAt`/`removeKey` discard (close) the element, and a pure helper
returning both the resource and the shrunk collection is impossible because
records cannot hold resources (so no `(RES T, List OF T)` tuple). Options:

- **(A) Defer move-out (simplest v1).** Resources enter via move-in and leave only
  by `removeAt`/`set`/`removeKey` (close) or by the whole collection being
  consumed/dropped. The pool pattern can close connections but cannot reclaim one
  as an owned `RES`. Document the limitation.
- **(B) Add one new primitive** `take OF T(value AS List OF T, index AS Integer) AS RES T`
  (and a map `takeKey`) that requires `value` to be a `MUT` binding, removes the
  element by destructive update, and returns it as an owned resource bound with
  `RES` at the call site. This is a specially-typed builtin (mutates the `MUT`
  collection in place, yields a `RES` result) — not a pure function.

Recommend (A) for the first cut; add (B) when a reclaim use case is concrete.

## 10. Compiler / IR / Codegen Changes

### 10.1 Type checker (`src/typecheck.rs`)
- Relax `contains_resource_or_thread()` at collection sites to permit a resource
  as `List` element / `Map` value, while still rejecting it as a `Map` key, a
  record field (unchanged), and a thread `Msg`.
- Mark any collection whose element/value type contains a resource as
  **non-copyable**; enforce linear single-owner threading on its bindings.
- Route resource-element helpers through consumption (§6/§7): consume the input
  collection; for `append`/`insert`/`prepend`/`set` consume the `item` binding
  (reuse `consume_local_if_needed`, the transfer-style event — see §11).
- Type `get`/`getOr`/`FOR EACH` element access on a resource collection as a
  **borrow**, not an owning value; reject binding the result with `RES`.
- Reject `values`/`find`/`contains`/`replace`/`mid`/`filter` on resource
  collections with targeted diagnostics.
- Add the §8 escape/scope check + `TYPE_RESOURCE_MOVE_NARROWS_SCOPE`.

### 10.2 Drop / codegen (`src/target/shared/code/`)
- New `ActiveCleanup::Collection { name, value_close_symbol | union_tags }` so a
  resource-owning collection sits on the cleanup stack and tears down at the right
  lexical / error exit, in reverse declaration order with other cleanups.
- New runtime drop walker: iterate the collection and call each present element's
  close op (tag-dispatch for resource-union element types). This is the principal
  new runtime routine — collections currently have no per-element drop.
- `set`/`removeAt`/`removeKey` emit a close of the displaced/removed element.
- The destructive-update optimization (`xs = op(xs, …)`) must remain correct: the
  successor takes over ownership of the surviving elements with no double-close.

### 10.3 IR / metadata (`src/ir.rs`, `src/binary_repr.rs`, `src/rules.rs`)
- The collection layout already carries a `value_type_code`; make the element
  resource type and its close op recoverable at the drop site (resolve via the
  resource table, as `resource_cleanup_symbol` does for named bindings). No new
  storage shape — a resource element is one pointer.
- Exported collection type shapes must record resource containment so importers
  reconstruct non-copyability and drop obligations (`mfbasic.md` §14.8).
- Verifier: a resource-owning collection must have a drop edge on every exit path;
  no path may drop it (and its elements) twice.

## 11. Is Move-In A "5th Event"?

Conceptually yes — move-into-an-owning-collection is a new way to invalidate a
`RES` binding. Mechanically it should **reuse** invalidation event #2's machinery
(`thread::transfer`): consume the source binding (mark `Moved`, deactivate named
cleanup, restore on failure) and hand ownership to another owner. The genuinely
new pieces are the **owning party** (a collection slot rather than a thread) and
the **drop walker** that discharges that ownership.

## 12. Hard Cases And Open Questions

- **No in-place close by index.** There is no `list[i]`, and a removed element is
  closed by `removeAt`, so the "which slot did I close" static-tracking problem
  does not arise: closing one early *is* `removeAt`/`set`, which consume and
  rebind the whole collection.
- **Borrowing an element across a call.** `get`/`FOR EACH` produce a call/loop
  borrow; the collection keeps ownership. Storing that borrow elsewhere stays
  forbidden.
- **Conditional move-in.** Produces `MaybeMoved` on the source binding (existing
  machinery); conditional drop composes. The walker closes only present elements.
- **Nested collections** (`List OF List OF File`): the walker recurses;
  non-copyability and the scope rule compose (reuse the recursion already in
  `contains_resource_or_thread`).
- **Thread sendability.** A resource-owning collection is sendable only if the
  element resource type is thread-sendable and move-on-send is honored. Out of
  scope for the first cut; default to not sendable.

## 13. Validation Strategy

- Function tests: move-in consumes both the collection and the `item`;
  use-after-move rejected; resource collection is non-copyable (second use / `LET`
  snapshot rejected); `get`/`FOR EACH` yield borrows that cannot be `RES`-bound;
  `values`/`find`/`contains`/`mid`/`filter` rejected; resource-as-`Map`-key
  rejected; downward move rejected.
- Runtime proofs: every still-present element closed exactly once on normal exit,
  `RETURN`, `FAIL`/`PROPAGATE`, trap routing, and `EXIT PROGRAM`; no fd leak
  (verify OS handle counts); `removeAt`/`set` close the removed/displaced element
  exactly once and it is not re-closed at scope drop.
- Drop-order proof: list elements close high index to low; the collection closes
  relative to sibling bindings in reverse declaration order.
- Import proof: an imported `List OF Socket` reconstructs non-copyability and drop
  obligations.

## 14. Recommended Implementation Sequence

1. Spec: amend `mfbasic.md` §12/§14.6/§15 and `standard_package.md` to define
   resource-owning collections, the linear rule, per-helper behavior, the
   upward-only move rule, and `Map`-key exclusion; add error codes to
   `error_codes.md`.
2. Type checker: relax collection containment for values; mark non-copyable;
   thread linearity; classify helpers (§7); wire move-in to
   `consume_local_if_needed`.
3. Drop walker + `ActiveCleanup::Collection`; `set`/`removeAt` element close.
4. Escape/scope check + `TYPE_RESOURCE_MOVE_NARROWS_SCOPE`.
5. Package metadata + verifier.
6. (Optional) `take`/`takeKey` move-out primitive (§9 option B).
7. Validation suite (§13).

## 15. Non-Goals For V1

- Borrowing collections (§5.1).
- Resources as `Map` keys.
- `values`/`find`/`contains`/`replace`/`mid`/`filter` on resource collections.
- Sending resource-owning collections across threads.
- Records owning resources (unchanged prohibition).

## 16. Bottom Line

The real obstacle is not storage or even drop — it is that MFBASIC's collection
helpers are **pure value-semantic functions over copyable elements**, which
resources can never be. Resources can live in a collection only if that
collection is a **linear, non-copyable, owning** value: helpers consume and
rebind it, `append`/`insert` move resources in, `get`/`FOR EACH` borrow them,
`set`/`removeAt` close the displaced/removed one, and a new per-element drop
walker closes the rest when the collection drops. Move-in should reuse
`thread::transfer`'s consumption machinery; the one new static guarantee worth
enforcing is **upward-only move**. Move-out into a fresh `RES` binding has no
home in the current pure API and is deferred (or added as the single new `take`
primitive). `Map` keys remain excluded for lack of comparability.
