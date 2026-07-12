# 12. Collections (owned, binding-driven mutability)

All access is via free functions — **no indexing brackets, no key brackets**. Brackets construct values; functions read and update them.

List literals use the declared or otherwise expected `List OF T` element type when one is available; otherwise the element type is inferred from the first item. Every element must be compatible with that element type. This allows annotated lists of union members, such as `LET shapes AS List OF Shape = [Circle[5], Rect[2, 3]]`.

**Bare-literal synthesis is asymmetric.** With no expected `List` type, the element type is taken from the **first** element only; every later element must then be *expression-compatible* with that fixed type. The check is one-directional — there is no join or numeric widening across elements — so element order matters. `[1, 2.0]` infers `List OF Integer` and **rejects** `2.0` (`TYPE_LIST_ELEMENT_MISMATCH`), while `[2.0, 1]` infers `List OF Float` and accepts the `Integer`, because an `Integer` is expression-compatible with `Float` but not the reverse. See type-inference (`./mfb spec language type-inference`) for the directional compatibility rule. Element-type inference direction is computed by `infer_list_literal`, but the mismatch itself is rejected on the IR by `ir::verify`. [[src/syntaxcheck/inference.rs:infer_list_literal]] [[src/ir/verify/mod.rs:1488]]

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

The global `len` is always available. Every other helper lives in the
`collections` package and requires `IMPORT collections` (a built-in package, so
no manifest dependency is needed). The package members fall into two
implementation groups (`src/builtins/collections.rs`).

**Native members** (`NATIVE_MEMBERS`) — code-generated list/map primitives whose
IR target is dequalified back to the bare native name:
`collections::get`, `collections::getOr`, `collections::set`,
`collections::append`, `collections::prepend`, `collections::insert`,
`collections::removeAt`, `collections::removeKey`, `collections::keys`,
`collections::values`, `collections::hasKey`, `collections::contains`,
`collections::forEach`, `collections::transform`, `collections::filter`,
`collections::reduce`, `collections::sum`, `collections::find`,
`collections::mid`, `collections::replace`. The `find`/`mid`/`replace` members
here are the **List** overloads only; their `String` overloads live in
`strings::`. [[src/builtins/collections.rs:NATIVE_MEMBERS]]

**Source generics** (`FUNCTIONS`) — generic MFBASIC functions defined in
`src/builtins/collections_package.mfb` and injected when the package is imported.
A call `collections::sort(x)` is rewritten to `__collections_sort(x)` during
monomorphization and instantiated like any generic function:
`collections::sort`, `collections::sortBy`, `collections::take`,
`collections::drop`, `collections::reduceRight`, `collections::any`,
`collections::all`, `collections::findIndex`, `collections::findLastIndex`,
`collections::groupBy`, `collections::mapValues`, `collections::flatten`,
`collections::zip`, `collections::chunks`, `collections::window`,
`collections::distinct`, `collections::merge`, `collections::partition`.

Comparability/orderability constraints (enforced on the IR by `ir::verify`):

- `collections::contains`, `collections::find`, and `collections::replace`
  require a **comparable** element type.
- A `Map OF K TO V` key type `K` must be comparable, enforced by
  `ir::verify`'s `check_map_key_comparable` ("Map key type"); a resource
  handle may never be a `Map` key.
- A type is comparable when it is `Boolean`, `Byte`, `Error`, `ErrorLoc`,
  `Fixed`, `Float`, `Integer`, `Nothing`, `String`, an `ENUM`, or a `TYPE`
  record whose fields are all comparable. `List`, `Map`, function values,
  `Result`, resources, threads, and `UNION` types are not comparable
  (`is_comparable_with_seen`).
- `collections::sort`/`collections::sortBy` order their elements/keys with the
  `<` operator, so the element (or key) type must be orderable; `distinct`
  relies on `contains` and therefore requires a comparable element type.

`collections::zip` produces a `List OF Pair OF A, B`, and
`collections::partition` produces a `Partition OF T`. `Pair OF A, B` (fields
`first`, `second`) and `Partition OF T` (fields `matched`, `unmatched`) are
compiler-owned generic record templates in the always-in-scope builtin prelude
(`builtin_prelude_file`, `src/ast/manifest.rs`); they are constructed and field-accessed
like ordinary records. `MapEntry OF K TO V` (fields `key`, `value`) is the
compiler-owned record yielded when iterating a `Map` with `FOR EACH`.

The names `toMap`, `zipWith`, and `filterEntries` are reserved but not exported.

The native collection memory layout — one uniform contiguous header + lookup
table + packed data region for both `List` and `Map` — is specified by the
memory spec, "Collections" (`./mfb spec memory collections`).

`FOR EACH` over `List OF T` visits items left to right. `FOR EACH` over `Map OF K TO V` visits `MapEntry OF K TO V` values in the map's implementation-defined stable iteration order.

## See Also

* ./mfb spec memory collections — the native `List`/`Map` memory layout
* ./mfb man collections — collection built-in help
* ./mfb spec language types — collection type forms and defaults
* ./mfb spec language type-inference — directional expression-compatibility rule behind bare list-literal element synthesis
