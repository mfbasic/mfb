use crate::ast::{AstFile, AstProject};
use crate::codegen::registry::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    DefaultValue, Implementation, InjectionRule, Lowering, Parameter, ParameterType, ReturnType,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

mod func_add;
mod func_append;
mod func_get;
mod func_get_or;
mod func_has_key;
mod func_keys;
mod func_prepend;
mod func_remove;
mod func_remove_key;
mod func_to_list;
mod func_values;

/// Path of the compiler-owned `collections` package source injected into every
/// project that imports it. This is the `AstFile.path` (see `source_file`), so
/// `AstProject::to_json` can filter it out of `-ast` output.
pub(crate) const SOURCE_PATH: &str = "builtins/collections.mfb";

/// The public `collections::` function names (without the `collections.`
/// qualifier). The implementations live in `package.mfb` as generic
/// `__collections_<name>` functions; a user call `collections::sort(...)` is
/// rewritten to `__collections_sort(...)` during monomorphization so the generic
/// machinery instantiates it like any other generic function.
// `toMap`, `zipWith`, and `filterEntries` from §6.4 are not yet exported: they
// depend on runtime capabilities MFBASIC does not have today — storing the
// compiler-owned `MapEntry` record inside a `List` (toMap/filterEntries) and
// applying a two-argument function value element-wise (zipWith). They are
// deferred until that infrastructure lands; see plan-01-functions.md §6.4.
const FUNCTIONS: &[&str] = &[
    "sort",
    "sortBy",
    "take",
    "drop",
    "any",
    "all",
    "findIndex",
    "findLastIndex",
    "groupBy",
    "mapValues",
    "flatten",
    "zip",
    "chunks",
    "window",
    "distinct",
    "merge",
    "partition",
    // Set algebra (plan-63-C): source generics over B's native Set members.
    "toSet",
    "union",
    "intersection",
    "difference",
    "symmetricDifference",
    "isSubset",
    "isSuperset",
    "isDisjoint",
];

/// The native `collections::` members migrated out of the bare global namespace
/// (plan-01-functions.md §5). These keep the native code generator's bare-name
/// lowering: the resolve logic is reused verbatim from `general`, and the IR
/// call target is dequalified back to the bare native name (see
/// `crate::builtins::native_builtin_target`). `find`/`mid`/`replace` accept ONLY the List
/// overload here; their String overloads live in `strings::`.
const NATIVE_MEMBERS: &[&str] = &[
    "get",
    "getOr",
    "set",
    "append",
    "prepend",
    "insert",
    "removeAt",
    "removeKey",
    "keys",
    "values",
    "hasKey",
    "contains",
    "forEach",
    "transform",
    "filter",
    "reduce",
    "reduceRight",
    "sum",
    "find",
    "mid",
    "replace",
    // Set members (plan-63-B): `add`/`remove`/`toList` are Set-only; `contains`
    // gains a Set overload alongside its List overload.
    "add",
    "remove",
    "toList",
];

// plan-72-E: `COLLECTIONS` is the descriptor authority for this package's NATIVE
// members (the source-generic `FUNCTIONS` above are resolved by the monomorphizer,
// not here, so they are not descriptor functions). The descriptor owns
// membership, per-position parameter names/aliases, and arity. Return-type
// resolution is genuinely generic (`get(List OF T, Integer) → T`, map/set/
// function-typed overloads), so it lives on a `BuiltinResolver` that delegates to
// the existing `dispatch_resolve` (below). Parameter *types* are documentation
// only — a member like `get` has List/Map overloads a single `ParameterType`
// cannot express, and no delegating wrapper reads them (resolution is
// resolver-owned; `expected_arguments` keeps its hand-authored "or"-phrased
// strings). The `.mfb` source companion is modelled as `WhenImported`.
const fn req(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}

/// An optional trailing parameter (only `find`'s `start`). The `Fill` is inert:
/// collections has no default-argument padding, so nothing reads it — it exists
/// solely so `DefaultResolver::arity` derives `find`'s `(2, 3)` range.
const fn opt(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::Fill {
            type_name: ty,
            expr: "",
        },
    }
}

const fn native(
    name: &'static str,
    slug: &'static str,
    doc_intro: &'static str,
    doc_desc: &'static str,
    errors: &'static [&'static str],
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        doc_intro,
        doc_desc,
        errors,
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

// One overload per member, carrying the merged parameter-name table; the resolver
// owns the actual per-overload (List/Map/Set) type resolution, so `return_type` is
// `Custom` throughout and the parameter *types* are documentation only.
const fn custom(params: &'static [Parameter]) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Custom,
    }
}

// Authored documentation strings for the native `collections::` members,
// derived from src/docs/man/builtins/collections/*.md (one-line summary + the
// Description section, citation markers stripped). See the `doc_intro`/`doc_desc`
// fields on `BuiltinFunction`.
// (`get`'s doc consts + entry moved to func_get.rs, plan-95.)

// ---- set ----
const INTO_SET: &str = "Return a collection with one element replaced, or one map key assigned";
const DESC_SET: &str = r#"`collections::set` returns a new collection with one position updated. It takes
exactly three arguments; none is optional and none is variadic. The first
argument selects the overload: a `List OF T` is addressed by an `Integer` index,
and a `Map OF K TO V` is addressed by a key of type `K`.

The two overloads differ in more than addressing — they differ in whether a
missing position is an error:

- For a **list**, the index must already exist. The bound is
  `0 <= index < len(value)`; the result has the same length as `value` and only
  the element at `index` differs. An index equal to the length is **not** an
  append position and raises `ErrIndexOutOfRange`, as does any negative index.
  Use `collections::append` or `collections::insert` to grow a list.

- For a **map**, the key need not exist. When the key is present its value is
  overwritten; when it is absent a new entry is inserted. The map overload has no
  failure path at all — it raises no domain error for any key.

`set` is value-semantic in both overloads. The collection named by `value` is
unchanged; the updated collection is the returned value, and a program observes
the update only through what it does with that return value. When the compiler
can prove the target is a uniquely owned local being reassigned — the
`c = collections::set(c, k, v)` shape, on a non-`by_ref` local that is not the
live iterable of an enclosing `FOR EACH` — it lowers the call to an in-place
update instead of rebuilding the collection. This is an optimization only; the
observable semantics, including the list bounds check, are identical either way.

On the general (copying) path the list overload is composed from
`removeAt(index)` followed by an insert of the replacement at the same index,
which is where its `0 <= index < len(value)` bound comes from; the map overload
is composed from `removeKey` — which is a filter and never fails on a missing
key — followed by a concatenation of the single new entry, which is why an
absent key inserts rather than raising.

`set` is classified **fallible** overall because of the list overload's range
check, so an inline `TRAP` on a `set` call compiles and catches that failure
rather than being reported as a dead handler. On the list path the bounds test
runs before any replacement value is materialized, so a rejected index allocates
nothing."#;

// ---- insert ----
const INTO_INSERT: &str = "Return a list with one element inserted before a given index";
const DESC_INSERT: &str = r#"`collections::insert` returns a new list in which `item` occupies position
`index`, every element of `value` below `index` keeps its position, and every
element from `index` onward is shifted up by one. The result is always exactly
one element longer than `value`. It takes exactly three arguments; none is
optional and none is variadic.

`index` is zero-based and is validated as `0 <= index <= len(value)`. The upper
bound is **inclusive**: `index` equal to the current length is the append
position and is accepted, producing the same result as
`collections::append(value, item)`. A negative `index`, or an `index` strictly
greater than the length, raises `ErrIndexOutOfRange`.

Only the single-element form exists. `item` must have exactly the element type
`T`; passing another `List OF T` resolves no overload, and the lowering rejects a
list-typed item explicitly with "insert expects a single item, not a list".
Internally the element is wrapped as a one-element list and spliced into `value`
at `index`, which is the same splice that backs `append` (index `= len`) and
`prepend` (index `0`).

`insert` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. There is no in-place fast path for `insert` at an
arbitrary index — the compiler's in-place assignment recognizers cover
`append`, bulk `append`, `prepend`, `set`, and string concatenation, not
`insert`.

`insert` is **fallible**: the range check is a real trappable domain error, so an
inline `TRAP` on an `insert` call compiles and catches the out-of-range failure
rather than being reported as a dead handler. The bounds test runs before any
allocation for the result, so a rejected index allocates nothing."#;

// ---- removeAt ----
const INTO_REMOVE_AT: &str = "Return a list with the element at a given index removed";
const DESC_REMOVE_AT: &str = r#"`collections::removeAt` returns a new list containing every element of `value`
except the one at `index`, with the elements above `index` shifted down by one to
close the gap and all other relative order preserved. The result is always
exactly one element shorter than `value`. It takes exactly two arguments; neither
is optional and neither is variadic.

`index` is zero-based and is validated as `0 <= index < len(value)`. The upper
bound is **exclusive**: unlike `collections::insert`, `index` equal to the length
is not a valid position — there is nothing there to remove — and raises
`ErrIndexOutOfRange`, as does any negative `index`. Removing from an empty list
therefore always raises, since no index satisfies the range.

`removeAt` is value-semantic. The list named by `value` is unchanged; the
shortened list is the returned value, and a program observes the update only
through what it does with that return value. There is no in-place fast path for
`removeAt` — the compiler's in-place assignment recognizers cover `append`, bulk
`append`, `prepend`, `set`, and string concatenation, not `removeAt`.

`removeAt` is **fallible**: the range check is a real trappable domain error, so
an inline `TRAP` on a `removeAt` call compiles and catches the out-of-range
failure rather than being reported as a dead handler. The bounds test runs before
the result block is allocated, so a rejected index allocates nothing.

`removeAt` operates on lists only. To drop a key from a `Map OF K TO V`, use
`collections::removeKey`, which takes a key rather than an index and does not
raise when the key is absent."#;

// ---- contains ----
const INTO_CONTAINS: &str = "Test whether a list holds an item equal to a given value.";
const DESC_CONTAINS: &str = r#"`collections::contains` scans `value` from index `0` upward and returns `TRUE`
as soon as an element matches `item`, or `FALSE` after every element has been
examined without a match. The list is neither copied nor mutated, and no element
payload is materialized — the scan compares stored bytes in place.

`contains` also has a **`Set OF T`** overload. Both forms take
`(collection, element) AS Boolean` and answer the same membership question; the
compiler picks the overload from the static type of the first argument. On a
`List` the scan is linear (below); on a `Set` membership is an O(1)-average hash
probe for a probe-eligible element type and a linear scan otherwise. It does not
accept a `Map`, and it is not the substring test: the `String` form of
`contains` lives in the `strings::` package, not here.

Equality is payload comparison, resolved by the element type:

- `Boolean` and `Byte` compare one stored byte; `Scalar` compares four; and
  `Integer`, `Float`, `Fixed`, and `Money` compare their stored 64-bit value.
- `String` compares length first, then bytes, so the match is exact and
  byte-oriented — no case folding, trimming, or Unicode normalization is applied.
- A record element is compared field by field.
- A resource handle, or a nested collection that is not stored flat, is compared
  by its stored handle rather than by its contents.

Because numeric comparison is bitwise, a `Float` search for `NaN` is always
`FALSE` even if the list contains `NaN`, and searching for `-0.0` does not match
a stored `0.0`.

An empty list always yields `FALSE`, since the loop exits on the first bounds
check. `collections::contains` raises no trappable domain error, so an inline
`TRAP` on a `contains` call has a dead handler.

`contains` answers only whether a match exists. Use `collections::find` when the
position of the match is needed."#;

// ---- forEach ----
const INTO_FOR_EACH: &str = "Call an action once for each element of a list, in order";
const DESC_FOR_EACH: &str = r#"`collections::forEach` walks `value` from the first element to the last and
calls `action` once per element, passing the element as the single argument. It
is a **native** member: the compiler emits the traversal loop directly rather
than instantiating an MFBASIC generic.

The loop is a straight forward scan over the list's entry table with no
reordering and no skipping, so `action` observes exactly the elements of `value`
in their stored order. `value` is neither copied nor modified; `forEach` builds
no result collection at all and evaluates to `Nothing`.

`action` must accept exactly one argument of the element type `T` and its
success type must be `Nothing`. A `SUB` is therefore accepted directly, since a
`SUB` has success type `Nothing`; a `FUNC` that produces a value is rejected at
compile time. To collect results instead of discarding them, use
`collections::transform`.

`action` must be a callable *value* — a reference to a declared `SUB` or `FUNC`.
A package member such as `io::print` is not a callable value and cannot be
passed here; wrap it in a `SUB` of your own, as the first example below does.

`action` is invoked through the shared direct-callable path, which restores a
closure's captured environment around each call, so a callable value that
carries an environment works as well as a plain named reference.

`forEach` raises no domain error of its own. It is classified fallible solely
because a failing `action` propagates: when the callback returns a non-`Ok`
result, the loop stops immediately at that element, later elements are never
visited, and the callback's own error is passed straight through — unchanged, so
whatever code and message the callback raised is what the caller sees. Because
`forEach` owns no accumulator, no cleanup runs on that path.

An inline `TRAP` on a `forEach` call captures that propagated callback error at
the call site rather than letting it auto-propagate.

An empty `value` calls `action` zero times."#;

// ---- transform ----
const INTO_TRANSFORM: &str =
    "Map every element of a list through a function and collect the results";
const DESC_TRANSFORM: &str = r#"`collections::transform` walks `value` from the first element to the last,
calls `f` once per element with that element as its only argument, and appends
each returned value to a new list. The result therefore has exactly as many
elements as `value`, in the same order. It is a **native** member: the compiler
emits the mapping loop directly rather than instantiating an MFBASIC generic.

The element type of the result is `f`'s success type `U`, so mapping a
`List OF Integer` through a `FUNC(Integer) AS String` yields a `List OF String`.
`U` may differ from `T` or equal it.

`f` must be a callable *value* — a reference to a declared `FUNC`, or a
`LAMBDA`. An overloaded built-in such as `toString` is not a callable value and
cannot be passed here; wrap it in a one-line `FUNC` of your own instead. The
single-argument `general` predicates (`isEven`, `isOdd`, and friends) *are*
ordinary callables and can be passed directly where their type fits.

`f` must produce a value: a callback whose success type is `Nothing` — such as a
`SUB` — does not resolve, because there would be nothing to collect. Use
`collections::forEach` to run a callback purely for its side effects.

`value` is neither modified nor consumed; the result is a freshly allocated
list. The output is pre-sized to the source list's working set, since
`transform` emits exactly one entry per source element, and each mapped value is
then appended in place.

An empty `value` calls `f` zero times and yields an empty `List OF U`.

`transform` raises no domain error of its own. It is classified fallible solely
because a failing `f` propagates: when the callback returns a non-`Ok` result,
the loop stops immediately at that element, later elements are never visited, no
result list is produced, and the callback's own error is passed through
unchanged. The partially built output is freed on that path before the error
leaves.

An inline `TRAP` on a `transform` call captures that propagated callback error
at the call site rather than letting it auto-propagate."#;

// ---- filter ----
const INTO_FILTER: &str = "Keep the elements of a list for which a predicate returns TRUE";
const DESC_FILTER: &str = r#"`collections::filter` walks `value` from the first element to the last, calls
`predicate` once per element, and appends the element to a new list when the
predicate returns `TRUE`. Elements for which the predicate returns `FALSE` are
skipped. It is a **native** member: the compiler emits the selection loop
directly rather than instantiating an MFBASIC generic.

Relative order is preserved: kept elements appear in the result in the same
order they had in `value`. The result has the same type as `value`, so filtering
a `List OF String` yields a `List OF String`, and its length is between zero and
the length of `value`.

`value` is neither modified nor consumed; the result is a freshly allocated
list, pre-sized to the source so the per-element append never has to regrow.

`predicate` must accept exactly one argument of the element type `T` and return
`Boolean`. This is enforced both when the call is resolved and again in the
lowering.

The single-argument `general` predicates — `isEven`, `isOdd`, `isPositive`,
`isNegative`, `isZero`, `isEmpty`, and `isNotEmpty` — are ordinary
`FUNC(T) AS Boolean` callables and can be passed directly whenever their
argument type matches the element type.

An empty `value` calls `predicate` zero times and yields an empty list.

`filter` raises no domain error of its own. It is classified fallible solely
because a failing `predicate` propagates: when the callback returns a non-`Ok`
result, the loop stops immediately at that element, later elements are never
visited, no result list is produced, and the callback's own error is passed
through unchanged. The partially built output is freed on that path before the
error leaves.

An inline `TRAP` on a `filter` call captures that propagated callback error at
the call site rather than letting it auto-propagate."#;

// ---- reduce ----
const INTO_REDUCE: &str = "Fold a list left to right into a single accumulated value";
const DESC_REDUCE: &str = r#"`collections::reduce` folds `value` into one value. The accumulator starts as
`initial`. The list is walked from the first element to the last, and for each
element the reducer is called as `f(accumulator, element)` — **accumulator
first, element second** — with its return value becoming the accumulator for the
next step. The accumulator left after the final element is the result. It is a
**native** member: the compiler emits the fold loop directly rather than
instantiating an MFBASIC generic.

The fold direction is left, from index 0 upward: the loop starts at the head of
the entry table and advances one entry per step. For a right-to-left fold, use
`collections::reduceRight`.

The accumulator type `U` is fixed by `initial`. `f`'s first parameter type, its
success type, and the type of `initial` must all be that same `U`, while `f`'s
second parameter must be the list element type `T`. `U` may differ from `T`, so
a `List OF String` can be folded into an `Integer`.

When `value` is empty, the loop body never runs, `f` is never called, and
`initial` is returned unchanged.

`value` is not modified. Unlike the other three callback members, `reduce`
deliberately does not free the per-element item it materializes for the
callback, because the reducer is allowed to return that item itself as the new
accumulator — freeing it would turn a leak into a use-after-free. Intermediate
accumulators are likewise left unfreed.

`reduce` raises no domain error of its own. It is classified fallible solely
because a failing `f` propagates: when the reducer returns a non-`Ok` result,
the fold stops immediately at that element, later elements are never visited,
and the reducer's own error is passed through unchanged. No cleanup runs on that
path, since the accumulator may still alias the borrowed `initial`.

An inline `TRAP` on a `reduce` call captures that propagated reducer error at
the call site rather than letting it auto-propagate."#;

// ---- reduceRight ----
const INTO_REDUCE_RIGHT: &str =
    "Fold a list into a single value, walking from the last item to the first";
const DESC_REDUCE_RIGHT: &str = r#"`collections::reduceRight` folds `value` into a single accumulated result. The
accumulator starts at `initial`. The function walks the list from the last index
down to index 0, and at each step replaces the accumulator with
`f(accumulator, item)`. When the walk finishes, the accumulator is returned.

The accumulator is the **first** argument of `f` and the list item is the second
— the same argument order `collections::reduce` uses. Only the traversal
direction differs between the two: `reduce` moves from the first item to the
last, `reduceRight` from the last to the first. `f` is therefore declared as
`FUNC(U, T) AS U`, not `FUNC(T, U) AS U`.

For a three-item list `[x, y, z]`, the result is
`f(f(f(initial, z), y), x)`. Direction matters whenever `f` is not associative
and commutative: folding `[1, 2, 3]` from the right with subtraction and an
initial accumulator of `0` yields `((0 - 3) - 2) - 1`, or `-6`.

`f` is called exactly once per item, so an empty `value` calls `f` not at all and
returns `initial` unchanged. `value` is not modified.

The accumulator type `U` need not match the element type `T`; `reduceRight` can
fold a list into a value of an entirely different type, such as building a
`String` from a `List OF Integer`.

`f` is an ordinary MFBASIC function value invoked with an ordinary call. If it
fails at any step, its error propagates out of `reduceRight` to the caller and
can be caught by the caller's `TRAP` block; the partially accumulated value is
discarded. `reduceRight` itself raises no error of its own.

`f` may be a named `FUNC` or a `LAMBDA` expression, since both produce a function
value of the required type."#;

// ---- sum ----
const INTO_SUM: &str = "Add up the elements of an Integer, Float, or Fixed list";
const DESC_SUM: &str = r#"`collections::sum` walks `value` from the first element to the last and adds
each element into a running total, returning that total. It is a **native**
member: the compiler emits the accumulation loop directly rather than
instantiating an MFBASIC generic.

There are exactly **three** overloads — `List OF Integer`, `List OF Float`, and
`List OF Fixed` — and the return type always matches the element type. There is
no `List OF Byte`, no `List OF Money`, and no general "any numeric list" form:
any other element type fails to resolve at compile time, and the lowering
rejects it a second time.

The accumulator is initialized to zero of the element type and the elements are
added in list order, so an empty `value` yields `0`, `0.0`, or `0.0F`
respectively without any addition being performed.

`value` is neither modified nor consumed. `sum` takes no callback and has no
optional argument; it is a single-argument member.

For the `Integer` and `Fixed` overloads each step is a **checked** 64-bit
addition: if the running total leaves the destination range, the addition fails
with `ErrOverflow` rather than wrapping. `Fixed` shares the `Integer` path
because it is a scaled 64-bit integer. The `Float` overload uses IEEE-754
double addition and never raises — an out-of-range total becomes `±Inf` in the
usual floating-point way.

Note a wrinkle worth knowing before writing a handler: the compiler's inline-
built-in fallibility census classifies `sum` as **infallible**, so attaching an
inline `TRAP` to a `sum` call raises the `TYPE_INLINE_TRAP_DEAD_HANDLER`
diagnostic and that handler does not receive the overflow. The overflow is still
raised at run time and still propagates out of the enclosing function, where an
ordinary function-level `TRAP` can handle it.

To total a list of some other element type, or to accumulate with different
rules, fold it with `collections::reduce`."#;

// ---- find ----
const INTO_FIND: &str =
    "Return the index of the first matching element or contiguous sublist in a list";
const DESC_FIND: &str = r#"`collections::find` scans `value` forward from `start` and returns the
zero-based index of the first match. It is a **native** member: the compiler
emits the search loop directly rather than instantiating an MFBASIC generic.

This page documents the `List` form only. `collections::find` accepts nothing
but a `List` as its first argument; the `String` search of the same name lives in
`strings::`.

Two searches share the name, chosen by the type of the second argument. When it
has the element type `T`, `find` performs an **element search**. When it has the
same `List OF T` type as `value`, `find` performs a **contiguous sublist
search**. The element form is tested first, so for a list of lists — where the
element type is itself a `List` — a second argument of that element type is read
as an element search. Any other second-argument type fails to resolve at compile
time.

`start` is optional. When it is omitted the search begins at index 0; the
lowering supplies that default itself, so an omitted `start` and an explicit `0`
behave identically.

`start` is validated before anything is compared. A negative `start`, or a
`start` greater than the length of `value`, fails with `ErrIndexOutOfRange`. A
`start` exactly equal to the length is **valid**: it selects an empty search
range, which yields `ErrNotFound` for an element search and, for a sublist
search with an empty needle, the index `start` itself.

When no match exists at or after `start`, `find` fails with `ErrNotFound`. It
never returns a sentinel such as `-1`; a search that may legitimately come up
empty needs a `TRAP`, or `collections::contains` if only the yes/no answer is
wanted.

Element equality is decided on the stored payload. `String` elements compare by
length and then byte for byte; `Integer`, `Float`, `Fixed`, and `Money` elements
compare as their stored 64-bit pattern, so `Float` matching is bit-exact and a
`NaN` never matches itself; `Boolean`, `Byte`, and `Scalar` compare as their
narrower stored value; record elements compare field by field. A nested
collection that is stored as a handle rather than inlined compares by identity,
not by contents.

`value` is neither modified nor consumed, and no new collection is allocated."#;

// ---- mid ----
const INTO_MID: &str = "Return a new list holding a contiguous run of elements taken from a list";
const DESC_MID: &str = r#"`collections::mid` returns a new list holding the `count` elements of `value`
that begin at the zero-based index `start`, in their original order. It is a
**native** member: the compiler emits the slice loop directly rather than
instantiating an MFBASIC generic.

This page documents the `List` form only. `collections::mid` accepts nothing but
a `List` as its first argument; the `String` slice of the same name lives in
`strings::`.

All three arguments are required — there is no two-argument "to the end" form —
and `start` and `count` must both be exactly `Integer`.

The range is **validated, not clamped**. Before any element is copied the
lowering checks, in order, that `start` is not negative, that `count` is not
negative, that `start` is not greater than the length of `value`, that
`start + count` does not wrap around, and that `start + count` is not greater
than the length of `value`. Any of those failing raises `ErrIndexOutOfRange`.
A short trailing run is therefore an error rather than a truncated result: on a
three-element list, `mid(value, 2, 2)` fails instead of returning one element.

Empty results are legal at the boundaries, since `start` may equal the length of
`value` and `count` may be `0`: on a four-element list, `mid(value, 4, 0)`
returns an empty list.

The result is a freshly allocated, independently owned list of the same type as
`value`; `value` itself is neither modified nor consumed, and element payloads
are copied into the new list's own data region rather than shared.

`mid` copies the selected run using a fast contiguous path when the source
entries covering the slice are stored in order and packed tightly, and falls
back to a per-entry copy otherwise. A list whose entry records have been
permuted without moving the underlying data — the result of a sorted directory
listing, for instance — takes the fallback. Either way the returned elements are
the same."#;

// ---- replace ----
const INTO_REPLACE: &str = "Return a list with every element equal to a given value replaced";
const DESC_REPLACE: &str = r#"`collections::replace` returns a new list of the same length as `value` in which
every element equal to `old` has been replaced by `new`, and every other element
is carried over unchanged. It takes exactly three arguments; none is optional and
none is variadic.

All matches are replaced, not just the first, and positions are preserved: the
result has the same length and the same ordering as `value`, differing only at
the indices where `old` occurred. When `old` does not occur, the result is a copy
of `value`. When `value` is empty, the result is empty.

Matching compares each element's stored payload against `old` using the same
element-equality test the rest of the collections layer uses, so the element type
must be one for which that comparison is defined; `old` and `new` must both have
exactly the element type `T`. `new` may itself be equal to `old`, in which case
the result is equal to `value`.

Only the **List** overload of `replace` lives in `collections`. The `String`
overload — replacing a substring within a `String` — is a different function that
lives in `strings::`. A `String` first argument does not resolve here.

`replace` is value-semantic. The list named by `value` is unchanged; the modified
list is the returned value, and a program observes the update only through what
it does with that return value. There is no in-place fast path for `replace` —
the compiler's in-place assignment recognizers cover `append`, bulk `append`,
`prepend`, `set`, and string concatenation, not `replace`.

`replace` is **infallible**: no path in its lowering raises a trappable domain
error. It has no index to range-check, and a `new` that never matches is a
success producing an unchanged copy, not a failure — so it is classified as
infallible alongside `append` and `prepend`, and an inline `TRAP` written on a
`replace` call has a dead handler (the front end reports
`TYPE_INLINE_TRAP_DEAD_HANDLER`). Allocation exhaustion is not a trappable domain
error in this language."#;

// (`add`/`remove`/`toList` doc consts + entries moved to their func_*.rs, plan-96.)
// ---- findIndex ----
const INTO_FIND_INDEX: &str =
    "Index of the first element at or after a start position that satisfies a predicate";
const DESC_FIND_INDEX: &str = r#"`collections::findIndex` scans `value` **forward**, beginning at index `start`
and advancing by one, calling `predicate` with each element. It returns the
zero-based index of the first element for which `predicate` returns `TRUE`. The
scan short-circuits at that element: no later element is examined. When the scan
reaches the end of the list without a match, the call raises `ErrNotFound`
(`77050004`) rather than returning a sentinel index.

`start` defaults to `0`, so the common call form scans the whole list. It is
validated **before** any element is read: the call raises `ErrIndexOutOfRange`
(`77050001`) when `start < 0` or `start > len(value)`. Two consequences are
worth stating precisely:

- `start` equal to `len(value)` is **legal**. It selects an empty scan, so the
  call raises `ErrNotFound`, not `ErrIndexOutOfRange`. `start` strictly greater
  than `len(value)` is the out-of-range case.
- A negative `start` is **not** interpreted as an offset from the end of the
  list. It is simply out of range and raises `ErrIndexOutOfRange`. This is
  deliberately asymmetric with `collections::findLastIndex`, whose `endIndex`
  parameter *does* resolve negative values from the end.

On an empty list every legal `start` is `0`, which is `len(value)`, so
`findIndex` on an empty list raises `ErrNotFound`.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findIndex` call to the
caller rather than being reported as a non-match. Note that a lambda passed here
may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findIndex`.

`findIndex` is a generic implemented in MFBASIC source; a call is rewritten to
the internal `__collections_findIndex` generic and instantiated for the element
type like any other generic function.  It
does not mutate `value`."#;

// ---- findLastIndex ----
const INTO_FIND_LAST_INDEX: &str =
    "Index of the last element at or before an end position that satisfies a predicate";
const DESC_FIND_LAST_INDEX: &str = r#"`collections::findLastIndex` scans `value` **backward**, beginning at the
element selected by `endIndex` and decreasing by one down to index `0`, calling
`predicate` with each element. It returns the zero-based index of the first
element (in that backward order) for which `predicate` returns `TRUE` — that is,
the last matching element at or before `endIndex`. The scan short-circuits at
that element: no lower index is examined. When the scan passes index `0` without
a match, the call raises `ErrNotFound` (`77050004`) rather than returning a
sentinel index.

The third parameter is named `endIndex`. It is resolved in two steps, and the
order matters:

1. **Negative resolution.** A negative `endIndex` counts from the end of the
   list: the effective index becomes `len(value) + endIndex`. The default of
   `-1` therefore selects the last element, so the common call form scans the
   whole list from its end. A non-negative `endIndex` is used as written.
2. **Range check.** *After* resolution, the call raises `ErrIndexOutOfRange`
   (`77050001`) when the resolved index is less than `0` or greater than or
   equal to `len(value)`.

Because the range check runs on the resolved index, the upper bound is
`len(value) - 1`, not `len(value)`. This is deliberately asymmetric with
`collections::findIndex`, whose `start` may equal `len(value)` and whose
negative values are rejected instead of resolved.

One consequence is worth stating explicitly: on an **empty** list `len(value)`
is `0`, so every `endIndex` resolves outside `0 .. -1` and is rejected. The
default `-1` resolves to `-1`, which fails the range check. `findLastIndex` on
an empty list therefore raises `ErrIndexOutOfRange` (`77050001`), **not**
`ErrNotFound`. A caller that treats "no match" and "empty input" alike must
handle both codes.

`predicate` is an ordinary function value of type `FUNC(T) AS Boolean` — a named
`FUNC` or a `LAMBDA`. Because it is called as an ordinary call, an error raised
inside `predicate` propagates out of the `collections::findLastIndex` call to
the caller rather than being reported as a non-match. Note that a lambda passed
here may not capture an outer `MUT` binding; the callback position proven
non-escaping is `collections::forEach`, not `findLastIndex`.

`findLastIndex` is a generic implemented in MFBASIC source; a call is rewritten
to the internal `__collections_findLastIndex` generic and instantiated for the
element type like any other generic function.
It does not mutate `value`."#;

const COLLECTIONS_FUNCTIONS: &[BuiltinFunction] = &[
    func_get::GET,
    func_get_or::GET_OR,
    native(
        "collections.set",
        "set",
        INTO_SET,
        DESC_SET,
        &["ErrIndexOutOfRange"],
        &[custom(&[
            req("value", &["collection"], "List OF T"),
            req("index", &["key"], "Integer"),
            req("item", &[], "T"),
        ])],
    ),
    func_append::APPEND,
    func_prepend::PREPEND,
    native(
        "collections.insert",
        "insert",
        INTO_INSERT,
        DESC_INSERT,
        &["ErrIndexOutOfRange"],
        &[custom(&[
            req("value", &["list"], "List OF T"),
            req("index", &[], "Integer"),
            req("item", &[], "T"),
        ])],
    ),
    native(
        "collections.removeAt",
        "removeAt",
        INTO_REMOVE_AT,
        DESC_REMOVE_AT,
        &["ErrIndexOutOfRange"],
        &[custom(&[
            req("value", &["list"], "List OF T"),
            req("index", &[], "Integer"),
        ])],
    ),
    func_remove_key::REMOVE_KEY,
    func_keys::KEYS,
    func_values::VALUES,
    func_has_key::HAS_KEY,
    native(
        "collections.contains",
        "contains",
        INTO_CONTAINS,
        DESC_CONTAINS,
        &[],
        &[custom(&[
            req("value", &["collection"], "List OF T"),
            req("item", &[], "T"),
        ])],
    ),
    native(
        "collections.forEach",
        "forEach",
        INTO_FOR_EACH,
        DESC_FOR_EACH,
        &[],
        &[custom(&[
            req("value", &["collection"], "List OF T"),
            req("action", &[], "FUNC(T) AS Nothing"),
        ])],
    ),
    native(
        "collections.transform",
        "transform",
        INTO_TRANSFORM,
        DESC_TRANSFORM,
        &[],
        &[custom(&[
            req("value", &["collection"], "List OF T"),
            req("f", &["transform"], "FUNC(T) AS U"),
        ])],
    ),
    native(
        "collections.filter",
        "filter",
        INTO_FILTER,
        DESC_FILTER,
        &[],
        &[custom(&[
            req("value", &["collection"], "List OF T"),
            req("predicate", &[], "FUNC(T) AS Boolean"),
        ])],
    ),
    native(
        "collections.reduce",
        "reduce",
        INTO_REDUCE,
        DESC_REDUCE,
        &[],
        &[custom(&[
            req("value", &["collection"], "List OF T"),
            req("initial", &["seed"], "U"),
            req("f", &["combine"], "FUNC(U, T) AS U"),
        ])],
    ),
    native(
        "collections.reduceRight",
        "reduceRight",
        INTO_REDUCE_RIGHT,
        DESC_REDUCE_RIGHT,
        &[],
        &[custom(&[
            req("value", &["collection"], "List OF T"),
            req("initial", &["seed"], "U"),
            req("f", &["combine"], "FUNC(U, T) AS U"),
        ])],
    ),
    native(
        "collections.sum",
        "sum",
        INTO_SUM,
        DESC_SUM,
        &["ErrOverflow"],
        &[custom(&[req("value", &["collection"], "List OF Number")])],
    ),
    native(
        "collections.find",
        "find",
        INTO_FIND,
        DESC_FIND,
        &["ErrIndexOutOfRange", "ErrNotFound"],
        &[custom(&[
            req("value", &["list"], "List OF T"),
            req("item", &["needle"], "T"),
            opt("start", &[], "Integer"),
        ])],
    ),
    native(
        "collections.mid",
        "mid",
        INTO_MID,
        DESC_MID,
        &["ErrIndexOutOfRange"],
        &[custom(&[
            req("value", &["list"], "List OF T"),
            req("start", &[], "Integer"),
            req("count", &[], "Integer"),
        ])],
    ),
    native(
        "collections.replace",
        "replace",
        INTO_REPLACE,
        DESC_REPLACE,
        &[],
        &[custom(&[
            req("value", &["list"], "List OF T"),
            req("old", &["needle"], "T"),
            req("new", &["replacement"], "T"),
        ])],
    ),
    func_add::ADD,
    func_remove::REMOVE,
    func_to_list::TO_LIST,
    // `findIndex`/`findLastIndex` are source-generic (they resolve and, for most
    // element types, run from the injected `.mfb` companion) with a native
    // String-item fast path for `findLastIndex`. They are listed here ONLY so
    // their errors and documentation are registered; they are deliberately kept
    // out of `NATIVE_MEMBERS`, so `is_native_member_call` still routes them
    // through the source-generic path. The parameter types are documentation.
    native(
        "collections.findIndex",
        "findIndex",
        INTO_FIND_INDEX,
        DESC_FIND_INDEX,
        &["ErrIndexOutOfRange", "ErrNotFound"],
        &[custom(&[
            req("value", &["list"], "List OF T"),
            req("predicate", &[], "FUNC(T) AS Boolean"),
            opt("start", &[], "Integer"),
        ])],
    ),
    native(
        "collections.findLastIndex",
        "findLastIndex",
        INTO_FIND_LAST_INDEX,
        DESC_FIND_LAST_INDEX,
        &["ErrIndexOutOfRange", "ErrNotFound"],
        &[custom(&[
            req("value", &["list"], "List OF T"),
            req("predicate", &[], "FUNC(T) AS Boolean"),
            opt("endIndex", &[], "Integer"),
        ])],
    ),
];

/// Generic return-type resolution for collections native members. Delegates to
/// the same `dispatch_resolve` logic the public `resolve_call` used pre-migration,
/// so every List/Map/Set/function-typed overload resolves byte-identically.
struct CollectionsResolver;
impl BuiltinResolver for CollectionsResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static COLLECTIONS_RESOLVER: CollectionsResolver = CollectionsResolver;

/// Package-overview description, from `src/docs/man/builtins/collections/package.md`
/// (its Description section, citation markers stripped).
const COLLECTIONS_DESC: &str = r#"The `collections` package provides package-qualified helpers for `List` and `Map`
values: element access and mutation (`get`, `set`, `append`, `prepend`, `insert`,
`removeAt`, `removeKey`), higher-order transforms (`transform`, `filter`,
`reduce`, `reduceRight`, `forEach`, `mapValues`), queries (`find`, `findIndex`,
`findLastIndex`, `contains`, `any`, `all`, `hasKey`, `keys`, `values`), reshaping
(`sort`, `sortBy`, `distinct`, `flatten`, `zip`, `chunks`, `window`, `partition`,
`groupBy`, `merge`), and numeric folding (`sum`). `collections` is a built-in
package: `IMPORT collections` needs no manifest dependency.

These helpers do not mutate their arguments. A function that changes a collection
returns a new value and leaves the original unchanged. List indexes are
zero-based, and access reads without copying the collection.

Element and key types follow the comparable/orderable rules: `sort` and `sortBy`
require an orderable element or key type, and `distinct` requires a comparable
element type. Map helpers operate on `Map OF K TO V` values, where the key type
`K` is the map's declared key type.

Predicates and other function arguments are passed as function values: a named
`FUNC`, a `LAMBDA`, or a general built-in predicate such as `isEven`,
`isPositive` or `isEmpty`. A built-in predicate resolves against the type
expected at that position — the element type of the list for a higher-order
call, or the declared type of a `FUNC(T) AS Boolean` binding — because a bare
name like `isPositive` is defined over `Integer`, `Float` and `Fixed` and nothing
in the reference alone chooses between them.

Some helpers introduce built-in result types: `zip` produces a `List OF Pair OF
A, B`, and `partition` produces a `Partition OF T` holding the matched and
unmatched elements. See `mfb man types pair` and `mfb man types partition`.

The List-only overloads of `find`, `mid`, and `replace` live here; their String
overloads live in `strings::`."#;

pub(crate) static COLLECTIONS: BuiltinModule = BuiltinModule {
    name: "collections",
    doc_intro: "Sequence and map helper functions",
    doc_desc: COLLECTIONS_DESC,
    functions: COLLECTIONS_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&COLLECTIONS_RESOLVER),
};

/// The internal generic-function name implementing a public `collections::`
/// member, e.g. `sort` -> `#collections_sort`. The injected package is lexed in
/// internal mode, so its `__collections_*` definitions carry the internal sigil;
/// the monomorphizer's rewrite target must match.
pub(crate) fn internal_name(member: &str) -> String {
    crate::internal_name::internalize(&format!("__collections_{member}"))
}

/// Whether `member` is a public `collections::` function name.
pub(crate) fn is_collections_function(member: &str) -> bool {
    FUNCTIONS.contains(&member)
}

/// Whether `member` is a migrated native `collections::` member (`get`,
/// `transform`, the List overloads of `find`/`mid`/`replace`, ...).
pub(crate) fn is_native_member(member: &str) -> bool {
    NATIVE_MEMBERS.contains(&member)
}

/// Whether `name` (a canonical `collections.<fn>` call) names a `collections::`
/// builtin — either a source generic function (`sort`, ...) or a migrated native
/// member (`get`, ...). Used by the resolver's builtin-member check.
pub(crate) fn is_collections_call(name: &str) -> bool {
    name.strip_prefix("collections.")
        .is_some_and(|member| is_collections_function(member) || is_native_member(member))
}

/// Whether `name` is a migrated native `collections::` member call
/// (`collections.get`, ...). Used to route the call into `general`'s resolve
/// logic and to dequalify the IR target back to the bare native name.
pub(crate) fn is_native_member_call(name: &str) -> bool {
    // Keyed off `NATIVE_MEMBERS`, NOT `DefaultResolver::contains(&COLLECTIONS, ..)`:
    // `findIndex`/`findLastIndex` are descriptor functions (for their errors/doc
    // metadata) but are source-generic, so they must NOT be routed as native
    // members here. `native_member_bare` consults `NATIVE_MEMBERS`, which
    // deliberately excludes them.
    native_member_bare(name).is_some()
}

/// Whether a native `collections.<member>` call takes a **unary callback over
/// the list's element type** as its second argument.
///
/// These are the positions where a bare general built-in predicate (`isEven`,
/// `isPositive`, …) must resolve: the callback's parameter type is not written
/// at the call site, it is the element type of the first argument, so the
/// checker has to bind it before the predicate reference can be typed
/// (bug-368).
///
/// `reduce` is deliberately absent — its callback is binary, so no unary
/// predicate fits it.
pub(crate) fn unary_callback_member(name: &str) -> bool {
    unary_callback_member_bare(name.strip_prefix("collections.").unwrap_or(name))
}

/// The bare-member form of [`unary_callback_member`], for the unqualified call
/// spelling that reaches `ir::lower` before canonicalization.
pub(crate) fn unary_callback_member_bare(name: &str) -> bool {
    matches!(name, "filter" | "transform" | "forEach")
}

/// `collections.get` -> `get`. Returns `None` for source generic functions and
/// non-`collections` names.
pub(crate) fn native_member_bare(name: &str) -> Option<&str> {
    name.strip_prefix("collections.")
        .filter(|member| is_native_member(member))
}

/// Resolves a `collections.<member>` native-member call by routing through the
/// descriptor's `BuiltinResolver` (plan-72-E), which delegates to
/// `dispatch_resolve`. The returned type string is identical to the pre-migration
/// path; only the `Cow` variant changes (`Owned` vs `Borrowed`), which no
/// consumer observes. Production dispatch goes through
/// `CollectionsResolver::resolve_return_type`; this wrapper exists only to let the
/// module tests exercise the full descriptor → resolver → `dispatch_resolve` path.
#[cfg(test)]
pub(crate) fn resolve_call<'a>(
    name: &str,
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    let return_type = COLLECTIONS
        .resolver?
        .resolve_return_type(&COLLECTIONS, name, arg_types)?;
    Some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Owned(return_type),
    })
}

/// The generic per-member resolution, delegating to the granular
/// `general::resolve_*`/local `resolve_*` helpers (which carry the original
/// bare-name semantics). `find`/`mid`/`replace` use the List-only overload here;
/// their String overloads live in `strings::`. Invoked through the descriptor
/// resolver by `resolve_call`.
fn dispatch_resolve<'a>(
    name: &str,
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    match native_member_bare(name)? {
        "get" => resolve_get(arg_types),
        "getOr" => resolve_get_or(arg_types),
        "set" => resolve_set(arg_types),
        "append" => resolve_append(arg_types),
        "prepend" => resolve_prepend(arg_types),
        "insert" => resolve_insert(arg_types),
        "removeAt" => resolve_remove_at(arg_types),
        "removeKey" => resolve_remove_key(arg_types),
        "keys" => resolve_keys(arg_types),
        "values" => resolve_values(arg_types),
        "hasKey" => resolve_has_key(arg_types),
        "contains" => resolve_contains(arg_types),
        "forEach" => resolve_for_each(arg_types),
        "transform" => resolve_transform(arg_types),
        "filter" => resolve_filter(arg_types),
        "reduce" => resolve_reduce(arg_types),
        "reduceRight" => resolve_reduce(arg_types),
        "sum" => resolve_sum(arg_types),
        "find" => resolve_find_list(arg_types),
        "mid" => resolve_mid_list(arg_types),
        "replace" => resolve_replace_list(arg_types),
        "add" => resolve_set_add(arg_types),
        "remove" => resolve_set_remove(arg_types),
        "toList" => resolve_set_to_list(arg_types),
        _ => None,
    }
}

/// `collections::add(Set OF T, T) AS Set OF T` (plan-63-B): insert an element,
/// idempotent (a duplicate is dropped). Set-only — a List uses `append`.
fn resolve_set_add<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = crate::builtins::general::set_element(&arg_types[0])?;
    (arg_types[1] == element).then_some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

/// `collections::remove(Set OF T, T) AS Set OF T` (plan-63-B): remove an element;
/// removing an absent element is a no-op. Set-only.
fn resolve_set_remove<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = crate::builtins::general::set_element(&arg_types[0])?;
    (arg_types[1] == element).then_some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

/// `collections::toList(Set OF T) AS List OF T` (plan-63-B): the elements in
/// stable insertion order. Set-only.
fn resolve_set_to_list<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let element = crate::builtins::general::set_element(&arg_types[0])?;
    Some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Owned(format!("List OF {element}")),
    })
}

/// List-overload resolvers for `find`/`mid`/`replace`, migrated to `collections::`
/// (plan-01-functions.md §5). These keep the original bare-name overload logic so
/// `collections::` can reuse it; the String overloads live in `strings::`.
fn resolve_find_list<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if !(2..=3).contains(&arg_types.len()) {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    (arg_types.get(2).is_none_or(|type_| type_ == "Integer")
        && (arg_types[1] == element || arg_types[1] == arg_types[0]))
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed("Integer"),
        })
}

fn resolve_mid_list<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    (arg_types.len() == 3
        && crate::builtins::general::list_element(&arg_types[0]).is_some()
        && arg_types[1] == "Integer"
        && arg_types[2] == "Integer")
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

fn resolve_replace_list<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    // Arity first: `arg_types[0]`/`list_element` must not be indexed before the
    // length is known, or an empty/short slice panics (bug-98).
    if arg_types.len() != 3 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    (arg_types[1] == element && arg_types[2] == element).then_some(
        crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        },
    )
}

fn resolve_get<'a>(arg_types: &'a [String]) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    if let Some(element) = crate::builtins::general::list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer").then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(element),
        });
    }
    let (key, value) = crate::builtins::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Borrowed(value),
    })
}

fn resolve_get_or<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    if let Some(element) = crate::builtins::general::list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer"
            && crate::builtins::general::element_accepts_item(element, &arg_types[2]))
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(element),
        });
    }
    let (key, value) = crate::builtins::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key && crate::builtins::general::element_accepts_item(value, &arg_types[2]))
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(value),
        })
}

fn resolve_set<'a>(arg_types: &'a [String]) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    if let Some(element) = crate::builtins::general::list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer"
            && crate::builtins::general::element_accepts_item(element, &arg_types[2]))
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        });
    }
    let (key, value) = crate::builtins::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key && crate::builtins::general::element_accepts_item(value, &arg_types[2]))
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

fn resolve_append<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    (crate::builtins::general::element_accepts_item(element, &arg_types[1])
        || arg_types[1] == arg_types[0])
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

fn resolve_prepend<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    crate::builtins::general::element_accepts_item(element, &arg_types[1]).then_some(
        crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        },
    )
}

fn resolve_insert<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    (arg_types[1] == "Integer"
        && crate::builtins::general::element_accepts_item(element, &arg_types[2]))
    .then_some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

fn resolve_remove_at<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    (arg_types.len() == 2
        && crate::builtins::general::list_element(&arg_types[0]).is_some()
        && arg_types[1] == "Integer")
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

fn resolve_remove_key<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let (key, _) = crate::builtins::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

fn resolve_keys<'a>(arg_types: &'a [String]) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let (key, _) = crate::builtins::general::map_parts(&arg_types[0])?;
    Some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Owned(format!("List OF {key}")),
    })
}

fn resolve_values<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let (_, value) = crate::builtins::general::map_parts(&arg_types[0])?;
    Some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Owned(format!("List OF {value}")),
    })
}

fn resolve_has_key<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let (key, _) = crate::builtins::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Borrowed("Boolean"),
    })
}

fn resolve_contains<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    // `contains` has a List overload (linear scan) and a Set overload (hash
    // probe, plan-63-B); both take `(collection, element) AS Boolean`.
    let element = crate::builtins::general::list_element(&arg_types[0])
        .or_else(|| crate::builtins::general::set_element(&arg_types[0]))?;
    (arg_types[1] == element).then_some(crate::builtins::general::ResolvedCall {
        return_type: Cow::Borrowed("Boolean"),
    })
}

fn resolve_sum<'a>(arg_types: &'a [String]) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    match arg_types[0].as_str() {
        "List OF Integer" => Some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed("Integer"),
        }),
        "List OF Float" => Some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed("Float"),
        }),
        "List OF Fixed" => Some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed("Fixed"),
        }),
        _ => None,
    }
}

fn resolve_for_each<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    let (params, returns) = crate::builtins::general::function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns == "Nothing").then_some(
        crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed("Nothing"),
        },
    )
}

fn resolve_transform<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    let (params, returns) = crate::builtins::general::function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns != "Nothing").then_some(
        crate::builtins::general::ResolvedCall {
            return_type: Cow::Owned(format!("List OF {returns}")),
        },
    )
}

fn resolve_filter<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    let (params, returns) = crate::builtins::general::function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns == "Boolean").then_some(
        crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        },
    )
}

fn resolve_reduce<'a>(
    arg_types: &'a [String],
) -> Option<crate::builtins::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    let element = crate::builtins::general::list_element(&arg_types[0])?;
    let (params, returns) = crate::builtins::general::function_parts(&arg_types[2])?;
    (params.len() == 2
        && params[0] == arg_types[1]
        && params[1] == element
        && returns == arg_types[1])
        .then_some(crate::builtins::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[1]),
        })
}

// `call_param_names` returns a `&'static` borrowed nested slice the owned
// `DefaultResolver::param_names` cannot produce, so it stays a static literal
// PINNED equal to `COLLECTIONS` by `parity_matches_descriptor` until plan-72-BB.
// `expected_arguments` keeps its hand-authored "or"-phrased strings (the
// descriptor's per-position types cannot express `List OF T, Integer or Map OF K
// TO V, K`), and `call_return_type_name` delegates to `general` — neither is
// descriptor-derivable, so both stay as-is (documented in the plan Corrections).
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match native_member_bare(name)? {
        "get" => Some(&[&["value", "collection"], &["index", "key"]]),
        "getOr" => Some(&[
            &["value", "collection"],
            &["index", "key"],
            &["default", "fallback"],
        ]),
        "set" => Some(&[&["value", "collection"], &["index", "key"], &["item"]]),
        "append" => Some(&[&["value", "list"], &["item", "items"]]),
        "prepend" => Some(&[&["value", "list"], &["item"]]),
        "insert" => Some(&[&["value", "list"], &["index"], &["item"]]),
        "removeAt" => Some(&[&["value", "list"], &["index"]]),
        "removeKey" => Some(&[&["value", "map"], &["key"]]),
        "keys" => Some(&[&["value", "map"]]),
        "values" => Some(&[&["value", "map"]]),
        "hasKey" => Some(&[&["value", "map"], &["key"]]),
        "contains" => Some(&[&["value", "collection"], &["item"]]),
        "forEach" => Some(&[&["value", "collection"], &["action"]]),
        "transform" => Some(&[&["value", "collection"], &["f", "transform"]]),
        "filter" => Some(&[&["value", "collection"], &["predicate"]]),
        "reduce" => Some(&[
            &["value", "collection"],
            &["initial", "seed"],
            &["f", "combine"],
        ]),
        "reduceRight" => Some(&[
            &["value", "collection"],
            &["initial", "seed"],
            &["f", "combine"],
        ]),
        "sum" => Some(&[&["value", "collection"]]),
        "find" => Some(&[&["value", "list"], &["item", "needle"], &["start"]]),
        "mid" => Some(&[&["value", "list"], &["start"], &["count"]]),
        "replace" => Some(&[
            &["value", "list"],
            &["old", "needle"],
            &["new", "replacement"],
        ]),
        // Set members (plan-63-B).
        "add" => Some(&[&["value", "set"], &["item", "element"]]),
        "remove" => Some(&[&["value", "set"], &["item", "element"]]),
        "toList" => Some(&[&["value", "set"]]),
        _ => None,
    }
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match native_member_bare(name)? {
        "get" => Some("List OF T, Integer or Map OF K TO V, K"),
        "getOr" => Some("List OF T, Integer, T or Map OF K TO V, K, V"),
        "set" => Some("List OF T, Integer, T or Map OF K TO V, K, V"),
        "append" => Some("List OF T, T or List OF T, List OF T"),
        "prepend" => Some("List OF T, T"),
        "insert" => Some("List OF T, Integer, T"),
        "removeAt" => Some("List OF T, Integer"),
        "removeKey" => Some("Map OF K TO V, K"),
        "keys" => Some("Map OF K TO V"),
        "values" => Some("Map OF K TO V"),
        "hasKey" => Some("Map OF K TO V, K"),
        "contains" => Some("List OF T, T"),
        "forEach" => Some("List OF T, FUNC(T) AS Nothing"),
        "transform" => Some("List OF T, FUNC(T) AS U"),
        "filter" => Some("List OF T, FUNC(T) AS Boolean"),
        "reduce" => Some("List OF T, U, FUNC(U, T) AS U"),
        "reduceRight" => Some("List OF T, U, FUNC(U, T) AS U"),
        "sum" => Some("List OF Integer, List OF Float, or List OF Fixed"),
        "find" => Some("List OF T, T[, Integer]"),
        "mid" => Some("List OF T, Integer, Integer"),
        "replace" => Some("List OF T, T, T"),
        // Set members (plan-63-B). `contains` also accepts `Set OF T, T` (its
        // overload); the message above stays List-first to match its historical
        // shape.
        "add" => Some("Set OF T, T"),
        "remove" => Some("Set OF T, T"),
        "toList" => Some("Set OF T"),
        _ => None,
    }
}

/// Whether any file in `ast` imports the `collections` package.
pub(crate) fn uses_package(ast: &AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "collections")
    })
}

/// Parses the built-in `collections` package source.
pub(crate) fn source_file() -> Result<AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new(SOURCE_PATH),
        SOURCE_PATH,
        include_str!("package.mfb"),
    )
}

/// Injects the `collections` package source into `ast` when the project imports
/// it. The source is appended last (so the monomorphizer's first-file emission
/// target is unchanged) and is filtered out of `-ast` output by its sentinel
/// path. Call rewriting (`collections.sort` -> `__collections_sort`) happens in
/// the monomorphizer.
pub(crate) fn augmented_project(ast: AstProject) -> Result<AstProject, ()> {
    if !uses_package(&ast) {
        return Ok(ast);
    }
    let mut augmented = ast;
    augmented.files.push(source_file()?);
    Ok(augmented)
}

/// Builds a binding-name -> package-name map covering every `collections` import
/// (including aliases) across the project. The monomorphizer uses it to map a
/// call's `binding.member` callee onto the internal generic implementation.
pub(crate) fn collections_bindings(ast: &AstProject) -> HashMap<String, ()> {
    let mut bindings = HashMap::new();
    for file in &ast.files {
        for import in &file.imports {
            if import.package_name() == "collections" {
                bindings.insert(import.binding_name().to_string(), ());
            }
        }
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rc(r: Option<crate::builtins::general::ResolvedCall>) -> Option<String> {
        r.map(|r| r.return_type.into_owned())
    }

    fn project(src: &str) -> AstProject {
        let file =
            crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse source");
        AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn function_and_native_membership() {
        assert!(is_collections_function("sort"));
        assert!(is_collections_function("partition"));
        assert!(!is_collections_function("get"));
        assert!(!is_collections_function("nope"));

        assert!(is_native_member("get"));
        assert!(is_native_member("replace"));
        assert!(!is_native_member("sort"));
        assert!(!is_native_member("nope"));
    }

    #[test]
    fn is_collections_call_cases() {
        assert!(is_collections_call("collections.sort")); // source generic
        assert!(is_collections_call("collections.get")); // native member
        assert!(!is_collections_call("collections.nope"));
        assert!(!is_collections_call("strings.find"));
        assert!(!is_collections_call("sort"));
    }

    #[test]
    fn native_member_call_and_bare() {
        assert!(is_native_member_call("collections.get"));
        assert!(!is_native_member_call("collections.sort"));
        assert!(!is_native_member_call("get"));
        assert_eq!(native_member_bare("collections.get"), Some("get"));
        assert_eq!(native_member_bare("collections.sort"), None);
        assert_eq!(native_member_bare("get"), None);
    }

    #[test]
    fn internal_name_shape() {
        let name = internal_name("sort");
        assert!(name.contains("collections_sort"), "{name}");
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn resolve_call_delegates_every_member() {
        assert_eq!(
            rt("collections.get", &["List OF Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.getOr",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.set",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.append", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.prepend", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.insert",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.removeAt", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.removeKey",
                &["Map OF String TO Integer", "String"]
            ),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rt("collections.keys", &["Map OF String TO Integer"]),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rt("collections.values", &["Map OF String TO Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.hasKey",
                &["Map OF String TO Integer", "String"]
            ),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rt("collections.contains", &["List OF Integer", "Integer"]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rt(
                "collections.forEach",
                &["List OF Integer", "FUNC(Integer) AS Nothing"]
            ),
            Some("Nothing".to_string())
        );
        assert_eq!(
            rt(
                "collections.transform",
                &["List OF Integer", "FUNC(Integer) AS String"]
            ),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rt(
                "collections.filter",
                &["List OF Integer", "FUNC(Integer) AS Boolean"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.reduce",
                &[
                    "List OF Integer",
                    "String",
                    "FUNC(String, Integer) AS String"
                ]
            ),
            Some("String".to_string())
        );
        assert_eq!(
            rt("collections.sum", &["List OF Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt("collections.find", &["List OF Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.mid",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.replace",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        // Non-native member and unknown name.
        assert!(resolve_call("collections.sort", &strings(&["List OF Integer"])).is_none());
        assert!(resolve_call("get", &strings(&["List OF Integer", "Integer"])).is_none());
        // Wrong types -> None.
        assert_eq!(rt("collections.get", &["List OF Integer", "String"]), None);
    }

    #[test]
    fn call_param_names_all_members() {
        for member in NATIVE_MEMBERS {
            let name = format!("collections.{member}");
            assert!(call_param_names(&name).is_some(), "{member}");
        }
        assert!(call_param_names("collections.sort").is_none());
        assert!(call_param_names("get").is_none());
    }

    #[test]
    fn expected_arguments_all_members() {
        for member in NATIVE_MEMBERS {
            let name = format!("collections.{member}");
            assert!(expected_arguments(&name).is_some(), "{member}");
        }
        assert!(expected_arguments("collections.sort").is_none());
    }

    #[test]
    fn uses_package_and_bindings() {
        let ast = project("IMPORT collections\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        assert!(collections_bindings(&ast).contains_key("collections"));

        let bare = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&bare));
        assert!(collections_bindings(&bare).is_empty());
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT collections\nSUB main\nEND SUB\n");
        let before = ast.files.len();
        let augmented = augmented_project(ast).expect("augment");
        assert_eq!(augmented.files.len(), before + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        let before = ast.files.len();
        assert_eq!(augmented_project(ast).expect("a").files.len(), before);
    }

    #[test]
    fn resolve_replace_list_arity_checks_before_indexing() {
        // bug-98: an empty or short arg slice must not panic (index OOB) before
        // the arity is verified.
        let empty: Vec<String> = Vec::new();
        assert!(resolve_replace_list(&empty).is_none());
        let one = strings(&["List OF Integer"]);
        assert!(resolve_replace_list(&one).is_none());
        let two = strings(&["List OF Integer", "Integer"]);
        assert!(resolve_replace_list(&two).is_none());
        // The valid 3-arg form still resolves.
        let three = strings(&["List OF Integer", "Integer", "Integer"]);
        let ok = resolve_replace_list(&three).map(|r| r.return_type.into_owned());
        assert_eq!(ok, Some("List OF Integer".to_string()));
    }

    #[test]
    fn resolve_find_list_cases() {
        assert_eq!(
            rc(resolve_find_list(&strings(&["List OF Integer", "Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        // sublist search (arg1 == whole list type)
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "List OF Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&["Integer", "Integer"]))),
            None
        );
        assert_eq!(rc(resolve_find_list(&strings(&["List OF Integer"]))), None);
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
    }

    #[test]
    fn resolve_mid_list_cases() {
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "Integer", "Integer", "Integer"
            ]))),
            None
        );
    }

    #[test]
    fn resolve_replace_list_cases() {
        assert_eq!(
            rc(resolve_replace_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_replace_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(rc(resolve_replace_list(&strings(&["Integer"]))), None);
    }

    #[test]
    fn resolve_get_and_getor() {
        assert_eq!(
            rc(resolve_get(&strings(&["List OF Integer", "Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_get(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_get(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_get(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "Map OF String TO Integer",
                "String",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "Map OF String TO Integer",
                "String",
                "String"
            ]))),
            None
        );
        assert_eq!(rc(resolve_get_or(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_set_cases() {
        assert_eq!(
            rc(resolve_set(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "List OF Integer",
                "String",
                "Integer"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "Map OF String TO Integer",
                "String",
                "Integer"
            ]))),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "Map OF String TO Integer",
                "Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_set(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(rc(resolve_set(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_append_prepend_insert() {
        assert_eq!(
            rc(resolve_append(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_append(&strings(&[
                "List OF Integer",
                "List OF Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_append(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(rc(resolve_append(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_append(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_prepend(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_prepend(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(rc(resolve_prepend(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_prepend(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_insert(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_insert(&strings(&[
                "List OF Integer",
                "String",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_insert(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(rc(resolve_insert(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_collection_element_is_state_agnostic() {
        // bug-427: a `List OF RES <union> STATE S` element keeps its STATE clause
        // in the list type string (so an extracted element can read `.state`),
        // but the two resolver callers normalize the *item* argument differently:
        // syntaxcheck passes the item WITH its STATE clause, while `ir::verify`
        // strips the STATE off resource arguments. Both must resolve, so element
        // insertion compares element and item by their bare resource type.
        let list = "List OF RES File STATE Cursor";

        // Item carrying its STATE (syntaxcheck's shape) resolves.
        assert_eq!(
            rc(resolve_append(&strings(&[list, "File STATE Cursor"]))),
            Some(list.to_string())
        );
        // Item stripped to the bare handle (`ir::verify`'s shape) also resolves.
        assert_eq!(
            rc(resolve_append(&strings(&[list, "File"]))),
            Some(list.to_string())
        );
        // A genuinely different resource is still rejected.
        assert_eq!(rc(resolve_append(&strings(&[list, "Socket"]))), None);

        // `get` returns the element type WITH its STATE clause, so an extracted
        // element's `.state` types against the union's uniform STATE.
        assert_eq!(
            rc(resolve_get(&strings(&[list, "Integer"]))),
            Some("File STATE Cursor".to_string())
        );

        // prepend / insert / set share the same STATE-agnostic item compare.
        assert_eq!(
            rc(resolve_prepend(&strings(&[list, "File"]))),
            Some(list.to_string())
        );
        assert_eq!(
            rc(resolve_insert(&strings(&[
                list,
                "Integer",
                "File STATE Cursor"
            ]))),
            Some(list.to_string())
        );
        assert_eq!(
            rc(resolve_set(&strings(&[list, "Integer", "File"]))),
            Some(list.to_string())
        );

        // Map values carry the same treatment.
        let map = "Map OF String TO RES File STATE Cursor";
        assert_eq!(
            rc(resolve_set(&strings(&[map, "String", "File"]))),
            Some(map.to_string())
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                map,
                "String",
                "File STATE Cursor"
            ]))),
            Some("File STATE Cursor".to_string())
        );
    }

    #[test]
    fn resolve_remove_at_and_key() {
        assert_eq!(
            rc(resolve_remove_at(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_remove_at(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_remove_at(&strings(&["Integer", "Integer"]))),
            None
        );

        assert_eq!(
            rc(resolve_remove_key(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rc(resolve_remove_key(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_remove_key(&strings(&["Integer", "a"]))), None);
        assert_eq!(
            rc(resolve_remove_key(&strings(&["Map OF String TO Integer"]))),
            None
        );
    }

    #[test]
    fn resolve_keys_values() {
        assert_eq!(
            rc(resolve_keys(&strings(&["Map OF String TO Integer"]))),
            Some("List OF String".to_string())
        );
        assert_eq!(rc(resolve_keys(&strings(&["Integer"]))), None);
        assert_eq!(
            rc(resolve_keys(&strings(&["Map OF String TO Integer", "x"]))),
            None
        );
        assert_eq!(
            rc(resolve_values(&strings(&["Map OF String TO Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(rc(resolve_values(&strings(&["Integer"]))), None);
        assert_eq!(
            rc(resolve_values(&strings(&["Map OF String TO Integer", "x"]))),
            None
        );
    }

    #[test]
    fn resolve_has_key_contains() {
        assert_eq!(
            rc(resolve_has_key(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rc(resolve_has_key(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_has_key(&strings(&["Integer", "a"]))), None);
        assert_eq!(
            rc(resolve_has_key(&strings(&["Map OF String TO Integer"]))),
            None
        );

        assert_eq!(
            rc(resolve_contains(&strings(&["List OF Integer", "Integer"]))),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rc(resolve_contains(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_contains(&strings(&["Integer", "Integer"]))),
            None
        );
        assert_eq!(rc(resolve_contains(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_sum_cases() {
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Float"]))),
            Some("Float".to_string())
        );
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Fixed"]))),
            Some("Fixed".to_string())
        );
        assert_eq!(rc(resolve_sum(&strings(&["List OF String"]))), None);
        assert_eq!(rc(resolve_sum(&strings(&["List OF Integer", "x"]))), None);
    }

    #[test]
    fn resolve_for_each_transform_filter_reduce() {
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Nothing"
            ]))),
            Some("Nothing".to_string())
        );
        // wrong return
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Boolean"
            ]))),
            None
        );
        // wrong element
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(String) AS Nothing"
            ]))),
            None
        );
        assert_eq!(rc(resolve_for_each(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_for_each(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_transform(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS String"
            ]))),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rc(resolve_transform(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Nothing"
            ]))),
            None
        );
        assert_eq!(rc(resolve_transform(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_transform(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_filter(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Boolean"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_filter(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_filter(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_filter(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_reduce(&strings(&[
                "List OF Integer",
                "String",
                "FUNC(String, Integer) AS String"
            ]))),
            Some("String".to_string())
        );
        assert_eq!(
            rc(resolve_reduce(&strings(&[
                "List OF Integer",
                "String",
                "FUNC(String, Integer) AS Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_reduce(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(
            rc(resolve_reduce(&strings(&["List OF Integer", "String"]))),
            None
        );
    }

    #[test]
    fn higher_order_resolvers_accept_function_valued_elements() {
        // `transform` over a list of two-argument function values: the mapper's
        // sole parameter *is* the element type, so the call must resolve.
        let element = "FUNC(Integer, Integer) AS Integer";
        let mapper = strings(&[
            &format!("List OF {element}"),
            &format!("FUNC({element}) AS String"),
        ]);
        let resolved =
            resolve_transform(&mapper).expect("transform over function-valued elements resolves");
        assert_eq!(resolved.return_type, "List OF String");

        let predicate = strings(&[
            &format!("List OF {element}"),
            &format!("FUNC({element}) AS Boolean"),
        ]);
        let resolved =
            resolve_filter(&predicate).expect("filter over function-valued elements resolves");
        assert_eq!(resolved.return_type, format!("List OF {element}"));

        // A mapper whose parameter is a *different* function type still fails.
        let mismatched = strings(&[
            &format!("List OF {element}"),
            "FUNC(FUNC(String) AS Integer) AS String",
        ]);
        assert!(resolve_transform(&mismatched).is_none());
    }

    /// The `const fn` descriptor constructors (`req`/`opt`/`native`/`custom`) are
    /// only invoked in `const` context by `COLLECTIONS_FUNCTIONS`, so they carry
    /// no runtime coverage. Call each at runtime and assert the fields it builds.
    #[test]
    fn const_constructors_build_expected_fields() {
        let r = req("value", &["collection"], "List OF T");
        assert_eq!(r.name, "value");
        assert_eq!(r.aliases, &["collection"]);
        assert_eq!(r.ty, ParameterType::Named("List OF T"));
        assert_eq!(r.default, DefaultValue::None);

        // `opt`'s `Fill` is inert (empty `expr`); it exists only so `arity`
        // derives `find`'s `(2, 3)` range.
        let o = opt("start", &[], "Integer");
        assert_eq!(o.name, "start");
        assert!(o.aliases.is_empty());
        assert_eq!(o.ty, ParameterType::Named("Integer"));
        assert_eq!(
            o.default,
            DefaultValue::Fill {
                type_name: "Integer",
                expr: ""
            }
        );

        // `custom` takes a `&'static [Parameter]`; a runtime temporary would be
        // E0716, so the parameter slice is a `const`.
        const PARAMS: &[Parameter] = &[req("value", &[], "List OF T")];
        let overload = custom(PARAMS);
        assert_eq!(overload.return_type, ReturnType::Custom);
        assert_eq!(overload.params.len(), 1);
        assert_eq!(overload.params[0].name, "value");

        // `native` takes a `&'static [BuiltinOverload]`; the overloads are a `const`.
        const OVS: &[BuiltinOverload] = &[custom(&[req("value", &[], "List OF T")])];
        let f = native("collections.get", "get", "into", "desc", &[], OVS);
        assert_eq!(f.name, "collections.get");
        assert_eq!(f.doc_slug, "get");
        assert_eq!(f.doc_intro, "into");
        assert_eq!(f.doc_desc, "desc");
        assert_eq!(f.overloads.len(), 1);
        assert_eq!(f.implementation, Implementation::Same);
        assert_eq!(f.lowering, Lowering::Helper);
        assert_eq!(f.flags, BuiltinFlags::default());
    }

    #[test]
    fn resolve_set_members() {
        // `add(Set OF T, T) AS Set OF T` — element must match.
        assert_eq!(
            rc(resolve_set_add(&strings(&["Set OF Integer", "Integer"]))),
            Some("Set OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set_add(&strings(&["Set OF Integer", "String"]))),
            None
        );
        // A List is not a Set here — `set_element` returns None.
        assert_eq!(
            rc(resolve_set_add(&strings(&["List OF Integer", "Integer"]))),
            None
        );
        assert_eq!(rc(resolve_set_add(&strings(&["Set OF Integer"]))), None);

        // `remove(Set OF T, T) AS Set OF T`.
        assert_eq!(
            rc(resolve_set_remove(&strings(&["Set OF Integer", "Integer"]))),
            Some("Set OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set_remove(&strings(&["Set OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_set_remove(&strings(&[
                "List OF Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_set_remove(&strings(&["Set OF Integer"]))), None);

        // `toList(Set OF T) AS List OF T`.
        assert_eq!(
            rc(resolve_set_to_list(&strings(&["Set OF Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set_to_list(&strings(&["List OF Integer"]))),
            None
        );
        assert_eq!(
            rc(resolve_set_to_list(&strings(&["Set OF Integer", "x"]))),
            None
        );

        // Through the full descriptor -> resolver -> `dispatch_resolve` path
        // (the `add`/`remove`/`toList` arms).
        assert_eq!(
            rt("collections.add", &["Set OF Integer", "Integer"]),
            Some("Set OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.remove", &["Set OF Integer", "Integer"]),
            Some("Set OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.toList", &["Set OF Integer"]),
            Some("List OF Integer".to_string())
        );
    }
}
