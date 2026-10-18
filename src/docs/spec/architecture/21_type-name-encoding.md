# Type-Name Encoding

A type name is a **flat string in the AST, and a rendering everywhere after
it.** The parser builds the string when it reads a type annotation, and
`hir::elaborate` turns it into a `ParameterType` once. From there to the
emitted byte the compiler carries the variant tree: the resolver, monomorphizer,
`ir::shape`, `ir::verify`, the `TypeModel` builder and codegen all **match
variants**, and none of them re-derives structure by prefix-stripping (enforced by `tests/guards/no_type_strings.rs`).

The string still matters, because it is what the wire formats store and what
diagnostics print. This document is the canonical contract for that encoding,
and `name()` supplies canonical spellings. Parsing accepts grouping and
normalizes it; redundant parentheses need not survive rendering. The forms below
describe emitted spelling, not every alternate spelling accepted by a decoder.

Where the two worlds meet is a short, closed list — the parser, the `.mfp` type
codec, the IR binary codec, the package manifest, and the AST→HIR seam. Those
are the "boundary files" the gate names, and they are the only places allowed to
call `ParameterType::parse` or build a spelling by hand.

The source-level type system these strings denote is [language types](./mfb spec
language types); the stage that specializes their typed forms is
[architecture monomorphization](./mfb spec architecture monomorphization).

## Canonical grammar

A type name is produced by `parse_type_name`, which is recursive: every nested
type is itself a canonical type name. [[src/ast/expr.rs:parse_type_name]]

```
Type        := FuncType | "(" Type ")" | BaseName [" OF " Args]
FuncType    := ["ISOLATED "] "FUNC(" [Type ("," " " Type)*] ") AS " Type
Args        := ListArg | MapArg | ThreadArg | TemplateArgs
```

| Form | Canonical string |
|------|------------------|
| List | `List OF X` |
| Resource-transfer list | `List OF RES X` |
| Stateful resource-transfer list | `List OF RES X STATE S` |
| Set | `Set OF X` |
| Map | `Map OF K TO V` |
| Resource-transfer map value | `Map OF K TO RES V` |
| Stateful resource-transfer map value | `Map OF K TO RES V STATE S` |
| Map entry | `MapEntry OF K TO V` |
| Function | `FUNC(P1, P2) AS R` |
| Isolated function | `ISOLATED FUNC(P1, P2) AS R` |
| Zero-arg function | `FUNC() AS R` |
| Parent thread | `Thread OF Msg TO Out` |
| Worker thread | `ThreadWorker OF Msg TO Out` |
| Thread + resource plane | `Thread OF Msg RES Res TO Out` |
| Resource-only thread | `Thread OF RES Res TO Out` |
| User template | `Name OF A, B` |
| Grouping | `(T)` |
| Internal success | `#Result OF X` (the `#` sigil keeps it out of user space; a source `Result OF X` is a user template) |
| C ABI type (LINK only) | `CPtr`, `CInt32`, … — see below |

### Fixed-width separators

These are the separators used by the boundary parser and renderer; spaces
inside the quoted forms are significant:

- `" OF "` — base name from its type arguments.
- `" TO "` — map key from value, thread message/resource from output.
- `") AS "` — function parameter list from return type.
- `", "` — successive template / function-parameter arguments
  (`args.join(", ")`). The reader splits commas at parenthesis depth zero and trims whitespace.
  [[src/ast/expr.rs:parse_type_name]]
- `"RES "` — the leading resource-transfer prefix on a collection element/value
  or thread plane (see below).

`OF`, `TO`, and `AS` are **infix keywords**, recovered by
`strip_prefix`/`split_once` on the surrounding literal rather than by tokenizing.
That recovery happens in exactly one place: `ParameterType::parse`, which turns a
name into the variant tree. Downstream stages do not re-derive it — the resolver,
for example, is a plain `match` on `ListOf`/`MapOf`/`Func`/`ThreadHandle`, and a
new shape reaches it as a new variant, not as a new prefix test.
[[src/types.rs:parse]] [[src/resolver/resolution.rs:resolve_type]]

## Base names and package identity

`parse_type_base_name` reads an identifier or `Nothing`, finishes any package
qualification, and normalizes the name through the package registry. Built-in
types retain package-qualified identities: `net::Url` becomes `net.Url`, not
bare `Url`. The package and type must match; an unrelated qualifier does not
make the type available. User/dependency types also retain their qualifier.
[[src/ast/expr.rs:parse_type_base_name]]
[[src/codegen/builtins/mod.rs:qualified_builtin_type]]

## Dotted names: `pkg::Ident` and `EnumType.Member`

The flat encoding uses `.` (a period) as its **single qualifier/member
separator**. Two distinct surface syntaxes collapse onto it at parse time:

- A `::`-qualified reference `pkg::Ident` is rewritten to dotted `pkg.Ident`
  by `finish_qualified_name`. Exactly two parts are allowed; a third `::`
  segment is a parse error. [[src/ast/expr.rs:finish_qualified_name]]
- A member access `EnumType.Member` is already written with `.`, so an
  enum-member reference and a (non-built-in) package-qualified name share one
  flat spelling. The resolver routes any name containing `.` to
  package-qualified resolution. [[src/resolver/resolution.rs:resolve_type]]

A non-built-in user/dependency type therefore keeps its dotted qualifier in the
flat string, as do package-qualified built-in identities.

## The `RES` resource-transfer prefix

A leading `RES ` on a collection element or value marks a **resource-transfer
collection** ([language resource-management](./mfb spec language
resource-management), §15.6): the element/value is a pointer to a resource whose
scope-ownership transfers across a function boundary.

| Position | Canonical form | Notes |
|----------|----------------|-------|
| List element | `List OF RES fs.File` | `RES` consumed only for `List`, not a template |
| Map value | `Map OF K TO RES fs.File` | prefix sits after `" TO "` |
| Thread resource plane | `Thread OF Msg RES Res TO Out` | infix `RES` clause |

`parse_type_name` accepts `RES` only after `List OF` and after `Map ... TO`. A user
template base does *not* consume `RES`, so `Box OF RES fs::File` is a parse error
(`MFB_PARSE_INVALID_IDENTIFIER`), consistent with the table above. Post-elaboration consumers peel the `ParameterType::Res` wrapper before
resolving the underlying type.
[[src/ast/expr.rs:parse_type_name]] [[src/resolver/resolution.rs:resolve_type]]

### Trailing ` STATE T` on a `RES` collection element

A `RES` collection element or map value may carry a trailing ` STATE T` clause —
a stateful resource (typically a resource union with a uniform state type across
its variants, [language resource-management](./mfb spec language
resource-management) §15.6) — folded into the element type string
(`List OF RES fs::File STATE Cursor`, `Map OF K TO RES fs::File STATE Cursor`). This
mirrors the thread resource plane, whose `RES` element folds the same clause
(`Thread OF RES fs::File STATE Cursor TO Out`) via `parse_resource_plane_type`. The
STATE rides the *element* (not the binding), so an extracted element reads
`.state` against `T`. A `STATE` clause is a parse error after a **non-`RES`**
element (a `STATE` is only meaningful on a resource).
[[src/ast/expr.rs:parse_optional_element_state]]

In the type model this clause is a variant: `ParameterType::parse` builds a
`Stateful { base, state }` for a resource's own top-level ` STATE T`, so
`File STATE Cursor` decomposes structurally instead of surviving as one opaque
`Named` whose spelling every consumer had to re-split. `name()` renders it back
byte-exact, so the encoding on this page is unchanged.
[[src/types.rs:ParameterType]]

**Top-level only.** A `Stateful` is built only when the text before ` STATE ` is
a single bare token — or for a thread type, whose trailing clause is its view's
(see Thread types) — so a clause nested inside an enclosing `List`/`Map`, or on a
thread's resource plane, stays on the *element*: `List OF RES fs::File STATE Cursor` is
`ListOf(Res(Stateful { .. }))`, and the outer list reports no state of its own.
`ParameterType::split_state` is therefore a plain match on that variant.
[[src/types.rs:split_state]]

Consumers recover the underlying resource **structurally**: `strip_res` peels
the `Res` wrapper and `ParameterType::without_state` / `state` peel the clause,
both plain matches on the variant. Element insertion compares the element and
item by their underlying resource type via the registry matcher's
`resource_base_eq`, including when only one side carries a STATE clause.
[[src/codegen/registry/mod.rs:resource_base_eq]]

The thread resource plane is structurally distinct: it is an **infix** ` RES `
clause between message and `" TO "`, not a leading prefix — see threads below.

### C ABI types

A `LINK` binding's `ABI (...)` clause names C types, not MFBASIC ones, and the
sixteen spellings are a closed vocabulary the marshaling backend implements:

`CPtr`, `CString`, `CBuffer`, `CInt8`, `CInt16`, `CInt32`, `CInt64`, `CUInt8`,
`CUInt16`, `CUInt32`, `CUInt64`, `CBool`, `CByte`, `CFloat`, `CDouble`, `CVoid`.

They are `ParameterType::C(CAbiType)` — one variant carrying a closed enum, not
sixteen variants — so a consumer needs one arm and can still enumerate the cases
exhaustively where the decision depends on which C type it is. `name()` renders
the spelling back byte-exact, which is what keeps the IR binary's `LINK` section
and every `.ast`/`.ir` golden unchanged. Each spelling is a bare token, so it
carries no separators and nests like any other leaf (`List OF CPtr` decomposes
normally). [[src/types.rs:CAbiType]]

A `CSTRUCT` name used as an `ABI` slot is **not** one of these: that namespace is
open (any `CSTRUCT` declared in the same `LINK` alias), so it stays a `Named`.
See [language native-libraries](./mfb spec language native-libraries).

## Thread types

`parse_thread_type_name` handles `Thread`/`ThreadWorker` bodies after `<kind> OF`.
The base token's case is canonicalized to exactly `Thread` or `ThreadWorker`. The
body has three shapes, and a resource-only thread defaults its message to
`Nothing`: [[src/ast/expr.rs:parse_thread_type_name]]

```
Thread OF Msg TO Out               ' data-only
Thread OF Msg RES Res TO Out       ' data + resource planes
Thread OF RES Res TO Out           ' resource-only (message defaults to Nothing)
Thread OF Msg TO Out STATE S       ' any of the above, with the view's own STATE
```

A trailing ` STATE S` after the output belongs to the handle: when the output
parses to its own top-level `Stateful`, `ParameterType::parse` lifts the clause
onto the handle (`Stateful { base: ThreadHandle { .. }, state: S }`), and
`with_state` builds the same shape, so `name()` round-trips byte-exact. An output
is data, which carries no STATE. [[src/types.rs:with_state]]

The single source of truth for **emitting** a thread type is
`format_thread_type`; the single source for **parsing** one back is
`thread_parts_full`, which returns `(kind, message, resource, output)`. Both the
parser and these helpers must agree on the three shapes, including the
`message == "Nothing"` collapse that drops the message segment for a
resource-only thread. [[src/types.rs:format_thread_type]]
[[src/types.rs:thread_parts_full]]

Because a thread output may itself be a grouped or nested type, the thread body
is split by measuring a balanced type prefix (`type_prefix_len`) rather than a
naive `split_once`, and each segment is unwrapped of redundant grouping by
`strip_type_group`. [[src/types.rs:split_thread_types]]
[[src/types.rs:strip_type_group]]

## User templates

`ParameterType::parse` decodes the `Name OF A, B` form into
`UserOf(name, args)`. It first **excludes** every built-in `OF`-bearing shape
(`List OF`, `Set OF`, `Map OF`, `MapEntry OF`, `#Result OF`, `Thread OF`,
`ThreadWorker OF`, and the `FUNC(`/`ISOLATED FUNC(` prefixes); only a base that
is none of these is treated as a user template. The remainder after `" OF "` is
split on top-level `", "` into the argument list. The exclusion list lives in this parser. [[src/types.rs:parse]]
[[src/codegen/builtins/mod.rs:split_top_level_commas]]

The resolver does not apply this precedence itself — it receives the decoded
`UserOf` and resolves the base and each argument. [[src/resolver/resolution.rs:resolve_type]]
The template machinery itself is [language templates](./mfb spec language
templates).

## Round-trip: render out, parse back

Canonical emitted spellings round-trip through `ParameterType::parse` and
`name`. Redundant grouping is normalized rather than preserved. The structural
round trip is load-bearing at the wire seams — the `.mfp` type section, the IR
binary, and the manifest all store the *rendered* spelling and read it back —
and is asserted directly for canonical spellings. [[src/types.rs:parse]] [[src/types.rs:name]]

Map/MapEntry bodies are split with `split_top_level_to`, which tracks
parentheses and pending nested `OF` constructs to locate the outer ` TO `.
Function/template argument lists use `split_top_level_commas`, which respects
parenthesis depth. Both live behind `parse`. [[src/types.rs:split_top_level_to]]
[[src/codegen/builtins/mod.rs:split_top_level_commas]]

## Type-consumer obligations

A new type shape needs a `ParameterType` variant and corresponding `parse` and
`name` arms. Consumers with wildcard matches also need review: resolver,
`ir::shape`, `ir::verify`, monomorphization, and `TypeModel` construction must
handle the variant according to their own contracts.

`Stateful { base, state }` is structurally distinct from `Named`; a check for
nominal resource behavior must account for the wrapper. `C(CAbiType)` represents
the closed C ABI vocabulary, while user-declared `CSTRUCT` slot names remain
`Named`. `with_vars` can reinterpret a C ABI spelling as a template variable
when that spelling is declared as a template parameter.
[[src/types.rs:ParameterType]] [[src/types.rs:CAbiType]] [[src/types.rs:with_vars]]

## See Also

* ./mfb spec language types — the source type system these strings denote
* ./mfb spec language templates — the `Name OF A, B` template form
* ./mfb spec language functions — `FUNC(...) AS R` and `ISOLATED` callables
* ./mfb spec language resource-management — the `RES` transfer marker (§15.6)
* ./mfb spec architecture monomorphization — the stage that substitutes and instantiates typed variants
* ./mfb spec architecture type-inference — how inferred types are spelled in this encoding
