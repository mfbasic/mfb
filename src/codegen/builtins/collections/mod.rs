use crate::ast::{AstFile, AstProject};
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, Registry,
    RegistryFunction, RegistryPackage,
};
use std::collections::HashMap;
use std::path::Path;

pub(crate) mod common;
mod func_add;
mod func_all;
mod func_any;
mod func_append;
mod func_chunks;
mod func_contains;
mod func_difference;
mod func_distinct;
mod func_drop;
mod func_filter;
mod func_find_index;
mod func_find_last_index;
mod func_flatten;
mod func_for_each;
mod func_get;
mod func_get_or;
mod func_group_by;
mod func_has_key;
mod func_insert;
mod func_intersection;
mod func_is_disjoint;
mod func_is_subset;
mod func_is_superset;
mod func_keys;
mod func_map_values;
mod func_merge;
mod func_partition;
mod func_prepend;
mod func_reduce;
mod func_reduce_right;
mod func_remove;
mod func_remove_at;
mod func_remove_key;
mod func_set;
mod func_sort;
mod func_sort_by;
mod func_sum;
mod func_symmetric_difference;
mod func_take;
mod func_to_list;
mod func_to_set;
mod func_union;
mod func_window;
mod func_zip;
// `pub(crate)`: source-generic fast paths in `src/target` (sortBy, mapValues,
// groupBy) reuse `lower_transform` directly until they too migrate (plan-96).
pub(crate) mod func_transform;
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

/// One-line package intro (was `BuiltinModule::doc_intro`).
const INTRO: &str = "Sequence and map helper functions";

/// A required native-member parameter. The `collections` man pages are served by
/// the static `.md` files, not the registry, so the per-parameter `desc` carries
/// no documentation weight here — it is left empty.
pub(super) fn param(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

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

const EX_FIND: &str = r#"Find an element, with and without a starting index:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30, 20]
  io::print(toString(collections::find(numbers, 20)))
  io::print(toString(collections::find(numbers, 20, 2)))
  RETURN 0
END FUNC
```

Find a contiguous sublist:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [10, 20, 30, 20]
  LET needle AS List OF Integer = [20, 30]
  io::print(toString(collections::find(numbers, needle)))
  RETURN 0
END FUNC
```

Handle a missing element instead of letting `ErrNotFound` propagate:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  LET index AS Integer = collections::find(numbers, 99) TRAP(e)
    io::print("absent: " & e.message)
    RECOVER -1
  END TRAP
  io::print(toString(index))
  RETURN 0
END FUNC
```"#;

const EX_MID: &str = r#"Take two elements from the middle:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3, 4]
  LET middle AS List OF Integer = collections::mid(numbers, 1, 2)
  io::print(toString(collections::get(middle, 0)))
  io::print(toString(len(middle)))
  RETURN 0
END FUNC
```

An empty slice at the end of the list is legal:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3, 4]
  LET empty AS List OF Integer = collections::mid(numbers, 4, 0)
  io::print(toString(len(empty)))
  RETURN 0
END FUNC
```

An over-long range raises rather than truncating, so handle it:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET numbers AS List OF Integer = [1, 2, 3]
  LET tail AS List OF Integer = collections::mid(numbers, 2, 2) TRAP(e)
    io::print("bad range: " & e.message)
    RECOVER []
  END TRAP
  io::print(toString(len(tail)))
  RETURN 0
END FUNC
```"#;

const EX_REPLACE: &str = r#"Replace every matching element:

```
IMPORT collections

FUNC main AS Integer
  LET values AS List OF Integer = collections::replace([1, 2, 1], 1, 9)
  RETURN 0
END FUNC
```

A needle that does not occur yields an unchanged copy:

```
IMPORT collections
IMPORT strings
IMPORT io

FUNC main AS Integer
  LET words AS List OF String = collections::replace(["a", "b"], "z", "Q")
  io::print(strings::join(words, ","))
  RETURN 0
END FUNC
```

Substituting a placeholder throughout a list:

```
IMPORT collections
IMPORT io

FUNC main AS Integer
  LET cleaned AS List OF String = collections::replace(["x", "b", "x"], "x", "QQ")
  io::print(toString(len(cleaned)))
  RETURN 0
END FUNC
```"#;

/// `collections::find` — List element/sublist search. Reached through the
/// `native_builtin_target` bare-name dispatch (`lower_find`), so its `Body` is
/// [`Body::Intrinsic`] (no `native_lower`, no rewrite); the descriptor exists only
/// for return-type resolution, arity, errors, and parameter names.
fn register_find(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "find",
        intro: INTO_FIND,
        desc: DESC_FIND,
        example: EX_FIND,
        implementations: vec![
            Implementation {
                params: vec![
                    param(
                        "value",
                        &["list"],
                        ParameterType::list_of(ParameterType::Var("T")),
                    ),
                    param("item", &["needle"], ParameterType::Var("T")),
                    Parameter {
                        name: "start",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::Optional,
                    },
                ],
                return_type: ParameterType::Integer,
                errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
                lowering: Lowering::Helper,
                body: Body::Intrinsic,
            },
            Implementation {
                params: vec![
                    param(
                        "value",
                        &["list"],
                        ParameterType::list_of(ParameterType::Var("T")),
                    ),
                    param(
                        "item",
                        &["needle"],
                        ParameterType::list_of(ParameterType::Var("T")),
                    ),
                    Parameter {
                        name: "start",
                        desc: "",
                        aliases: &[],
                        ty: ParameterType::Integer,
                        default: DefaultValue::Optional,
                    },
                ],
                return_type: ParameterType::Integer,
                errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
                lowering: Lowering::Helper,
                body: Body::Intrinsic,
            },
        ],
    });
}

/// `collections::mid` — List slice. Bare-name dispatch (`lower_mid`); [`Body::Intrinsic`].
fn register_mid(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "mid",
        intro: INTO_MID,
        desc: DESC_MID,
        example: EX_MID,
        implementations: vec![Implementation {
            params: vec![
                param(
                    "value",
                    &["list"],
                    ParameterType::list_of(ParameterType::Var("T")),
                ),
                param("start", &[], ParameterType::Integer),
                param("count", &[], ParameterType::Integer),
            ],
            return_type: ParameterType::Arg(0),
            errors: vec!["ErrIndexOutOfRange"],
            lowering: Lowering::Helper,
            body: Body::Intrinsic,
        }],
    });
}

/// `collections::replace` — List element replacement. Bare-name dispatch
/// (`lower_replace`); [`Body::Intrinsic`].
fn register_replace(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "replace",
        intro: INTO_REPLACE,
        desc: DESC_REPLACE,
        example: EX_REPLACE,
        implementations: vec![Implementation {
            params: vec![
                param(
                    "value",
                    &["list"],
                    ParameterType::list_of(ParameterType::Var("T")),
                ),
                param("old", &["needle"], ParameterType::Var("T")),
                param("new", &["replacement"], ParameterType::Var("T")),
            ],
            return_type: ParameterType::Arg(0),
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::Intrinsic,
        }],
    });
}

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

/// Register the `collections` package on the clean-room registry. Only the NATIVE
/// members are registered here (the source-generic members — `sort`, `zip`, … —
/// keep their manifest-injected MFBASIC bodies and are resolved by the
/// monomorphizer, so they are deliberately absent). No records/unions/helpers are
/// added and no `Body::Mfb` member is registered, so `get_mfb()` is empty and
/// `registry().augment_project` does NOT inject `collections` — it stays injected
/// through [`augmented_project`].
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("collections", INTRO, COLLECTIONS_DESC);

    func_get::register(&mut pkg);
    func_get_or::register(&mut pkg);
    func_set::register(&mut pkg);
    func_append::register(&mut pkg);
    func_prepend::register(&mut pkg);
    func_insert::register(&mut pkg);
    func_remove_at::register(&mut pkg);
    func_remove_key::register(&mut pkg);
    func_keys::register(&mut pkg);
    func_values::register(&mut pkg);
    func_has_key::register(&mut pkg);
    func_contains::register(&mut pkg);
    func_for_each::register(&mut pkg);
    func_transform::register(&mut pkg);
    func_filter::register(&mut pkg);
    func_reduce::register(&mut pkg);
    func_reduce_right::register(&mut pkg);
    func_sum::register(&mut pkg);
    register_find(&mut pkg);
    register_mid(&mut pkg);
    register_replace(&mut pkg);
    func_add::register(&mut pkg);
    func_remove::register(&mut pkg);
    func_to_list::register(&mut pkg);

    r.add_package(pkg);
}

/// The native fast-path dispatch for the SOURCE-GENERIC members: a
/// `#collections_<member>$<TypeArgs>` monomorph target is routed to the member's
/// `<member>_fast_path` fn, which either lowers the instantiation natively or
/// declines (`Ok(None)`), in which case the caller instantiates the injected
/// `.mfb` body instead. Only the source-generic members with a native accelerator
/// appear here; every other member returns `None` (no fast path).
pub(crate) fn mfb_fast_path(target: &str) -> Option<crate::codegen::registry::MfbFastPath> {
    let member = target.strip_prefix("#collections_")?.split('$').next()?;
    Some(match member {
        "sort" => func_sort::sort_fast_path,
        "sortBy" => func_sort_by::sort_by_fast_path,
        "mapValues" => func_map_values::map_values_fast_path,
        "groupBy" => func_group_by::group_by_fast_path,
        "chunks" => func_chunks::chunks_fast_path,
        "window" => func_window::window_fast_path,
        "merge" => func_merge::merge_fast_path,
        "partition" => func_partition::partition_fast_path,
        "flatten" => func_flatten::flatten_fast_path,
        "findLastIndex" => func_find_last_index::find_last_index_fast_path,
        "zip" => func_zip::zip_fast_path,
        _ => return None,
    })
}

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
    // Keyed off `NATIVE_MEMBERS`, NOT the registry: `findIndex`/`findLastIndex`
    // are source-generic, so they must NOT be routed as native members here.
    // `native_member_bare` consults `NATIVE_MEMBERS`, which deliberately excludes
    // them.
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

// `call_param_names` returns a `&'static` borrowed nested slice used by the
// keyword-argument matcher. It stays a static literal (the registry's
// `call_param_names` returns owned `Vec`s, which this borrowed shape cannot
// produce). `expected_arguments` keeps its hand-authored "or"-phrased strings
// (the descriptor's per-position types cannot express `List OF T, Integer or Map
// OF K TO V, K`).
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

/// Parses the built-in `collections` package source. `package.mfb` is now
/// self-contained (all source-generic bodies inlined at their original marker
/// positions), so it is parsed directly with no body-splicing step.
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
    use crate::codegen::registry::{self, registry};

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
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
    fn collections_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("collections")
            .expect("collections package");
        // Exactly the 24 native members (source generics are not registered here).
        assert_eq!(pkg.functions().len(), NATIVE_MEMBERS.len());
        assert!(registry::is_member("collections.get"));
        assert!(!registry::is_member("collections.sort")); // source generic
        assert!(!registry::is_member("collections.nope"));
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        registry::resolve_call(name, &strings(args))
    }

    #[test]
    fn generic_dispatch_resolves_native_members() {
        assert_eq!(
            rt("collections.get", &["List OF Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt("collections.get", &["Map OF String TO Integer", "String"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt("collections.keys", &["Map OF String TO Integer"]),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rt("collections.append", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        // RES marker preserved + STATE-agnostic (bug-427).
        assert_eq!(
            rt(
                "collections.append",
                &["List OF RES File STATE Cursor", "File"]
            ),
            Some("List OF RES File STATE Cursor".to_string())
        );
        assert_eq!(
            rt("collections.toList", &["Set OF Integer"]),
            Some("List OF Integer".to_string())
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
            rt(
                "collections.filter",
                &["List OF Integer", "FUNC(Integer) AS Boolean"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.sum", &["List OF Integer"]),
            Some("Integer".to_string())
        );
        // Wrong-type rejection.
        assert_eq!(rt("collections.get", &["List OF Integer", "String"]), None);
    }
}
