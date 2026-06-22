# MFBASIC Resources In Collections Plan

Last updated: 2026-06-21

This document records why resources cannot currently be referenced from `List`
values or `Map` values, what the runtime resource model actually is after the
resource overhaul, and a design for letting collections **hold borrows of
resources** while keeping the single rule that governs them: **a resource is
owned by a scope** — and closed exactly once when that scope exits.

It complements:

- `specifications/mfbasic.md` (§12 collections, §14 ownership, §15 resources)
- `specifications/standard_package.md` (collection helpers)
- `specifications/memory_layouts.md`
- `specifications/error_codes.md`

## 1. The Ownership Model (the thing to get right)

**A resource is owned by a scope. Nothing else owns it.**

- The resource is one record (§3). A *handle* is a pointer to that record.
- A resource is still bound **only** with `RES` (`TYPE_RESOURCE_REQUIRES_RES`),
  and `RES` still binds only resources (`TYPE_RES_REQUIRES_RESOURCE`). This plan
  does not touch how resources are bound.
- What holds a **borrow** (a copy of the pointer) is: a `RES` binding, a borrowed
  `RES` parameter, and — new in this plan — a collection slot (a `List` element or
  `Map` value). **The pointer is freely copyable; copying it is a borrow, never a
  duplication of the resource.** A collection slot holds a resource *borrow*; it
  is not a resource binding.
- None of those bindings or slots own the resource and none of them close it.
- A **scope** owns the resource. Exactly one scope owns it at any instant. That
  scope closes it exactly once when it exits — on every path (normal,
  `RETURN`, `FAIL`, `PROPAGATE`, auto-propagation, trap routing, `EXIT PROGRAM`).
- By default the owning scope is the scope where the resource is produced.

Everything in this plan follows from that. "Resources in collections" is not
about a collection *owning* anything; it is about a collection holding borrows,
and about which **scope** owns the resource those borrows point at.

### 1.1 Existing resource rules are preserved (nothing here repeals them)

This plan is purely additive — it only lets a collection slot be one more place a
borrow can live. Every existing resource rule (`mfbasic.md` §15) stands unchanged:

- **`RES`-only binding** and the distinct ownership axis.
- **Borrow by default**; the fixed invalidation events (registered close op +
  re-export aliases, `thread::transfer`, `RETURN`, scope drop); borrows may not
  invalidate (`TYPE_RESOURCE_BORROW_INVALIDATE`).
- **Records never hold resources** (`TYPE_RESOURCE_FIELD_FORBIDDEN`) — unchanged.
- **`STATE`** rules (copyable/defaultable payload, shared through borrows).
- **Resource unions**: a union all of whose variants are resources is itself a
  resource, `RES`-bound, **tag-dispatched drop**, no mixing data and resource
  variants (`TYPE_MIXED_RESOURCE_UNION`), no `STATE` on a resource union. A `Map`
  value or `List` element may be such a resource union; the borrow is the union
  value and drop/close stays tag-dispatched at the owning scope.
- **Native `RESOURCE … CLOSE BY …`**, thread-sendability opt-in, and
  `thread::transfer`/`accept` — unchanged.

## 2. Current State

The spec and compiler forbid resource handles (and thread handles) anywhere in
ordinary collections, transitively:

- `mfbasic.md` §14.6: "Ordinary containers cannot store resource handles or
  thread handles." §14.8 lists it as a required diagnostic; §15 forbids resource
  fields in records.
- One shared predicate, `contains_resource_or_thread()` (`src/typecheck.rs`),
  rejects a resource as a `List` element or `Map` key/value, recursing through
  nested `List`/`Map`/`Result`/record/union.
- Diagnostic: `TYPE_COLLECTION_OWNERSHIP_VIOLATION` (`2-203-0056`, `src/rules.rs`).

## 3. The Runtime Resource Model (what actually exists)

The resource overhaul implemented the lifecycle on **compile-time, per-scope**
machinery. There is no runtime registry of live resources.

**Representation.** A handle is a pointer to a single 24-byte arena record
(`src/target/shared/code/mod.rs`):

| offset | field |
|--------|-------|
| 0 (`FILE_OFFSET_FD`) | host handle word (fd / socket / native `CPtr`) |
| 8 (`FILE_OFFSET_CLOSED`) | closed flag |
| 16 (`FILE_OFFSET_STATE`) | `STATE` payload pointer |

`RESOURCE_RECORD_SIZE = 24`. Exactly **one record per resource**. Closing sets
the closed flag and releases the OS handle; it does **not** free the record (the
record memory is arena-lived until package-instance shutdown).

**Borrow = pointer aliasing.** Passing a `RES` binding to an ordinary function is
a call-scoped borrow (`ExprMode::Borrow`, the default): the callee gets the same
pointer, shares the same record and `STATE`. A borrowed parameter cannot close,
`RETURN`, or `thread::transfer` (`TYPE_RESOURCE_BORROW_INVALIDATE`).

**Close obligation = a per-scope, name-keyed cleanup stack.** This is where the
ownership already lives — and where it is too narrow. A `RES` binding pushes
`ActiveCleanup::Resource { name, symbol }` onto `active_cleanups`
(`builder_control.rs`). Scope exit unrolls the stack in reverse, emitting one
close per entry; close/transfer/return deactivates the entry **by name**
(`deactivate_resource_cleanup`).

The mechanism is per-scope, which is correct. The limitation is that it assumes
**the owning scope is exactly the scope of a named `RES` binding**, and it
identifies the obligation by that binding's name. To own a resource whose only
handle lives inside a collection — or to move ownership to an outer scope — the
obligation must be keyed to the **record**, not a binding name, and must be
**re-homeable to an outer scope**.

**Collections do no per-element close.** Collections are arena-backed and
bulk-freed; there is no per-element close path. That is fine and stays true:
collections hold borrows, so they must **never** close anything.

## 4. Why The Restriction Exists (in these terms)

Today the only scope that can own a resource is one holding a named `RES`
binding, and the obligation is found by name. A collection element has no name,
and the owning scope might need to be one that has no `RES` binding for the
resource at all. So:

- there is no name to register or deactivate for a handle that lives only in a
  collection;
- there is no way to express "this *outer* scope owns the resource that an inner
  block produced and stashed in an outer-scope collection";
- so the safe conservative rule is: refuse resources in collections entirely.

`Map` keys have an extra blocker: handles are not comparable/hashable in the
source language. Keys stay out of scope here; this plan is about `Map` values.

## 5. What Collections Need (and don't)

Because a collection only ever holds **borrows (copied pointers)**:

- A resource collection is an **ordinary copyable collection of pointers.** No
  linearity, no move-only, no "non-copyable collection." Copying the collection
  copies pointers (more borrows), never the resource.
- The standard helpers work unchanged in spirit — they shuffle pointers like any
  value: `get`/`getOr` and `FOR EACH` yield a **borrow** of an element;
  `append`/`insert`/`prepend`/`set`/`removeAt`/`mid`/`filter`/`transform` move
  pointers around. Removing or dropping an element just discards a borrow — it
  **does not close** anything.
- `find`/`contains`/`replace` still require comparable `T`, so they remain
  unavailable for resource elements (handles aren't comparable) — same reason as
  `Map` keys, not an ownership issue.

So collections need almost nothing new. The whole problem is **which scope owns
the resource**, so that closing it once still happens and no borrow outlives it.

## 6. The Only Real Rule: Ownership Floats Up

A borrow stored in a collection can outlive the scope that produced the resource.
If the producing scope still owned it, it would close at that inner scope's exit
and every borrow still sitting in an outer-scope collection would point at a
closed record.

The fix is to make **ownership follow the longest-lived reference**:

> Adding a borrow of a resource to a collection migrates the resource's **owning
> scope** up to the collection's scope, when that scope outlives the current
> owner. Ownership always floats to the **outermost** scope that references the
> resource; it never moves down.

Consequences:

- Add a borrow to a **higher-scope** collection → the owning scope rises to that
  collection's scope; the resource is closed when that outer scope exits; every
  borrow (the original binding, the collection elements) is within that scope, so
  none dangles.
- Add a borrow to a **same/lower-scope** collection → ownership unchanged; the
  collection just holds a borrow; that collection closes nothing.
- A binding whose ownership has floated to an outer scope becomes a plain
  **borrow** (usable, but it no longer closes at its own scope exit, and cannot
  close/`RETURN`/`transfer`).

Because all references are within the owning scope, `get`/`FOR EACH` borrows are
statically safe with **no** runtime closed-flag dependency. The closed flag is
only a backstop that makes the single close idempotent if a handle is reachable
through more than one path.

## 7. Implementation

### 7.1 Ownership = a re-homeable, per-scope close obligation
- Generalize `active_cleanups` from `{ name, symbol }` to a close obligation
  identified by the **record** (the handle pointer), not a binding name. A scope
  owns a set of such obligations.
- The set is static when the count is known (today's case) and a **per-scope
  runtime owned-list** when it is dynamic (e.g. a loop producing resources into an
  outer-scope collection). At scope exit, close each obligation once
  (closed-flag-idempotent), in reverse order; list elements high index to low.
- Re-homing: when a borrow is added to an outer-scope collection, move the
  obligation from the inner scope's set to the outer scope's set. The canonical
  handle for the close is carried with the obligation (e.g. a hidden owner-slot at
  the owning scope), so closing never has to consult any collection.

### 7.2 Type checker (`src/typecheck.rs`)
- Relax `contains_resource_or_thread()` at collection sites to permit a resource
  as a `List` element / `Map` value (still reject as a `Map` key, a record field,
  and a thread `Msg`).
- Treat collection elements / `get` / `FOR EACH` of a resource type as borrows
  (not owning values); reject `RES`-binding such a borrow as if it were an owner.
- **Escape analysis** to compute the owning scope = the outermost scope that
  references the resource, and to drive re-homing on insertion into an outer-scope
  collection. This is the genuinely new static work; the compiler tracks
  per-binding move state today but does no scope/escape comparison.
- A binding whose ownership floats away transitions to borrow-only (live, but
  not an owner — like a borrowed `RES` parameter).

### 7.3 Codegen (`src/target/shared/code/`)
- Replace name-keyed cleanup with record-keyed obligations; support a per-scope
  runtime owned-list for the dynamic case; emit re-home on insertion.
- Collections emit **no** close logic. Removing/overwriting/dropping an element
  discards a borrow only.

### 7.4 Metadata / verifier (`src/binary_repr.rs`, `src/rules.rs`)
- Exported collection type shapes record resource containment so importers know a
  `List OF Socket` element is a borrow whose owner is a scope (`mfbasic.md` §14.8).
- Verifier: every owned resource is closed exactly once on every exit path; no
  collection ever emits a close.

## 8. Hard Cases

- **No move-out needed.** Because the binding stays a usable borrow and ownership
  is scope-level, there is no "take it back out of the collection" problem — the
  handle was never removed from the owner; it was only ever borrowed.
- **Duplicate borrows** (same handle in two collections / a binding and a
  collection): owned by one scope (the outermost); closed once; the closed flag
  makes any redundant close path a no-op.
- **Conditional insertion** uses existing `MaybeMoved`-style reasoning to decide
  whether ownership floated on a given path.
- **Nested collections** (`List OF List OF File`): same rule recursively — the
  owning scope is the outermost referencing scope.
- **Threads.** A resource crossing to another thread still uses
  `thread::transfer` (its own per-thread ownership); collection sharing across
  threads is out of scope here.

## 9. Validation

- Function tests: a resource added to an outer-scope collection is not closed at
  the inner scope; it is closed once at the outer scope; the original binding
  remains usable as a borrow; `get`/`FOR EACH` yield borrows that cannot be
  `RES`-bound; resource as `Map` key rejected; `find`/`contains`/`replace`
  rejected (comparability).
- Runtime proofs: exactly-once close on every exit path; no fd leak (OS handle
  counts); copying a resource collection produces more borrows and still exactly
  one close; closed-flag idempotence on duplicate paths.
- Drop-order proof: owned resources close relative to sibling obligations in
  reverse order; list-reachable handles do not trigger collection-side closes.

## 10. Implementation Sequence

1. Spec: amend `mfbasic.md` §12/§14/§15 to state scope ownership explicitly,
   collections-hold-borrows, and the ownership-floats-up rule; note `Map`-key and
   comparability exclusions; add any new error codes to `error_codes.md`.
2. Generalize cleanup to record-keyed, re-homeable per-scope obligations (static
   + runtime owned-list).
3. Type checker: relax collection containment; borrow-type element access; escape
   analysis for owning scope + re-homing.
4. Codegen: re-home on insertion; collections emit no close.
5. Metadata + verifier.
6. Validation suite (§9).

## 11. Non-Goals For V1

- Resources as `Map` keys; `find`/`contains`/`replace` on resource elements
  (comparability).
- Sharing resource-referencing collections across threads.
- Records holding resource handles (unchanged prohibition).

## 12. Bottom Line

A resource is owned by a **scope**, never by a binding or a collection.
Collections hold **borrows** — copies of the one handle pointer — and close
nothing. Letting resources appear in `List`/`Map` values needs almost no change to
collections themselves; it needs the close obligation to be keyed to the
**record** and **re-homeable to an outer scope**, plus escape analysis so that
adding a borrow to a longer-lived collection floats the owning scope up. Then the
resource is closed exactly once, at the outermost scope that references it, and no
borrow can outlive it. `Map` keys stay out for lack of comparability.
