# 12. Collections (owned, binding-driven mutability)

All access is via free functions — **no indexing brackets, no key brackets**. Brackets construct values; functions read and update them.

List literals use the declared or otherwise expected `List OF T` element type when one is available; otherwise the element type is inferred from the first item. Every element must be compatible with that element type. This allows annotated lists of union members, such as `LET shapes AS List OF Shape = [Circle[5], Rect[2, 3]]`.

```basic
LET list  = [1, 2, 3]                          ' List OF Integer (literal)
LET first = collections::get(list, 0)           ' read (fallible -> auto-propagates)
LET list2 = collections::append(list, 4)        ' new immutable snapshot
LET safe  = collections::getOr(list, 99, 0)     ' read with default, never fails

LET m  = Map OF String TO Integer { "a" := 1, "b" := 2 }   ' literal
LET a  = collections::get(m, "a")               ' read (fallible)
LET m2 = collections::set(m, "c", 3)            ' new map
LET n  = len(list)

MUT pts AS List OF Vec3 = []
pts = collections::append(pts, v)               ' in-place append on the mutable buffer
```

- `collections::get` can fail (missing key / out-of-range index fails with an `Error`) and therefore auto-propagates. Use `collections::getOr(coll, key, default)` for the common defaulted read.
- A collection bound with `LET` is an immutable snapshot. Update functions such as `collections::append` and `collections::set` may read it and produce a new collection value, but assigning back to the same `LET` binding or otherwise modifying it is a compile-time error.
- A collection bound with `MUT` is a locally mutable buffer. When the result of an update function is assigned back to the same `MUT` binding, such as `pts = collections::append(pts, v)`, the compiler performs the update destructively in place instead of allocating a replacement collection.
- Update helpers are semantically pure functions. `collections::append(pts, v)` by itself computes and discards a result; it has no lasting effect unless the result is assigned, returned, passed, or otherwise consumed. Destructive update is an optimization only for the assignment-back-to-the-same-`MUT` pattern.
- Update functions on `MUT` collections preserve ownership semantics at boundaries: passing or returning the collection freezes it into an immutable owned value (§14).
- Containers own their contents. Adding a value to a collection stores an owned value in the collection, never a borrowed reference to an external binding. The one exception is a resource handle: a `List` element or `Map` value may hold a **borrow** of a resource (a copy of the handle pointer). The resource itself is owned by a *scope*, not by the collection; the collection closes nothing (§15.6).
- Immutability is deep for the contained value graph. A `LET` collection does not allow mutation of its elements through the collection, and no element can be observed as shared mutable state through another collection or binding.

Built-in collection helpers include the global `len`, plus the `collections` package functions `collections::get`, `collections::getOr`, `collections::find`, `collections::mid`, `collections::replace`, `collections::set`, `collections::append`, `collections::prepend`, `collections::insert`, `collections::removeAt`, `collections::removeKey`, `collections::keys`, `collections::values`, `collections::hasKey`, `collections::contains`, `collections::forEach`, `collections::transform`, `collections::filter`, `collections::reduce`, and `collections::sum`.

The native collection memory layout is specified in `specifications/memory_layouts.md`.

`FOR EACH` over `List OF T` visits items left to right. `FOR EACH` over `Map OF K TO V` visits `MapEntry OF K TO V` values in the map's implementation-defined stable iteration order.
