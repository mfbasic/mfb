# Type-Name Encoding

A type name is a **flat string in the AST, and a rendering everywhere after
it.** The parser builds the string when it reads a type annotation, and
`hir::elaborate` turns it into a `ParameterType` once. From there to the
emitted byte the compiler carries the variant tree: the resolver, monomorphizer,
`ir::shape`, `ir::verify`, the `TypeModel` builder and codegen all **match
variants**, and none of them re-derives structure by prefix-stripping (plan-111,
enforced by `tests/no_type_strings.rs`).

The string still matters, because it is what the wire formats store and what
diagnostics print. This document is the canonical contract for that encoding,
and the round trip `parse(name).name() == name` is what makes rendering
lossless. A mismatch in spacing, keyword casing, or separator width breaks every
*decoder*, so the grammar below is exact down to single spaces.

Where the two worlds meet is a short, closed list — the parser, the `.mfp` type
codec, the IR binary codec, the package manifest, and the AST→HIR seam. Those
are the "boundary files" the gate names, and they are the only places allowed to
call `ParameterType::parse` or build a spelling by hand.

The source-level type system these strings denote is [language types](./mfb spec
language types); the stage that parses them while specializing generics is
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
| Internal success | `Result OF X` |

### Fixed-width separators

These are the load-bearing literals every stage splits on. They are **exact** —
one leading and one trailing space each:

- `" OF "` — base name from its type arguments.
- `" TO "` — map key from value, thread message/resource from output.
- `") AS "` — function parameter list from return type.
- `", "` — successive template / function-parameter arguments
  (`args.join(", ")`). Splitting is on the literal two-character `", "`.
  [[src/ast/expr.rs:parse_type_name]]
- `"RES "` — the leading resource-transfer prefix on a collection element/value
  or thread plane (see below).

`OF`, `TO`, and `AS` are **infix keywords**, recovered by
`strip_prefix`/`split_once` on the surrounding literal rather than by tokenizing.
That recovery happens in exactly one place: `ParameterType::parse`, which turns a
name into the variant tree. Downstream stages do not re-derive it — the resolver,
for example, is a plain `match` on `ListOf`/`MapOf`/`Func`/`ThreadHandle`, and a
new shape reaches it as a new variant, not as a new prefix test (plan-111).
[[src/types.rs:parse]] [[src/resolver/resolution.rs:resolve_type]]

## Base names and bare-id normalization

`parse_type_base_name` reads one identifier (or the `Nothing` keyword) as the
base. A **package-qualified built-in type** is normalized here, at parse time, to
its bare internal id: `net::Url` becomes `Url`, `http::Response` becomes
`Response`, so no downstream stage ever sees a qualified built-in type. The
rewrite is `qualified_builtin_type`, which only fires when the qualifier is a
built-in import **and** the member is a built-in type id; otherwise the dotted
name passes through unchanged. [[src/ast/expr.rs:parse_type_base_name]]
[[src/codegen/builtins/mod.rs:qualified_builtin_type]]

The same normalization is mirrored in the resolver so a qualified built-in type
in a fully-qualified context resolves to its bare id rather than erroring.
[[src/resolver/resolution.rs:resolve_package_qualified_name]]

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
flat string; only built-in package types are stripped to bare ids.

## The `RES` resource-transfer prefix

A leading `RES ` on a collection element or value marks a **resource-transfer
collection** ([language resource-management](./mfb spec language
resource-management), §15.6): the element/value is a pointer to a resource whose
scope-ownership transfers across a function boundary.

| Position | Canonical form | Notes |
|----------|----------------|-------|
| List element | `List OF RES fs::File` | `RES` consumed only for `List`, not `Result` |
| Map value | `Map OF K TO RES fs::File` | prefix sits after `" TO "` |
| Thread resource plane | `Thread OF Msg RES Res TO Out` | infix `RES` clause |

`parse_type_name` accepts `RES` only after `List OF` and after `Map ... TO`. The
`Result` base does *not* consume `RES`, so `Result OF RES fs::File` is a parse error
(`MFB_PARSE_INVALID_IDENTIFIER`), consistent with the table above. Consumers strip it with
`strip_prefix("RES ").unwrap_or(...)` before resolving the underlying type.
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
a single bare token, so a clause nested inside an enclosing `List`/`Map`/`Thread`
stays on the *element*: `List OF RES fs::File STATE Cursor` is
`ListOf(Res(Stateful { .. }))`, and the outer list reports no state of its own.
`ParameterType::split_state` is therefore a plain match on that variant.
[[src/types.rs:split_state]]

Consumers recover the underlying resource **structurally**: `strip_res` peels
the `Res` wrapper and `ParameterType::without_state` / `state` peel the clause,
both plain matches on the variant. The old `&str` adapters over the same grammar
(`base_resource_name` / `state_type_name`) are down to `base_resource_name`'s
last few callers and a `cfg(test)` parity partner; they were composite-safe by
construction because they shared `types::split_state_clause` with `parse`, and
the variant form inherits that property from the parse itself. Element insertion (`append`/`insert`/`set`) compares the
element and the item by their bare resource type, so an item passed with or
without its STATE clause both resolve; that comparison is now the registry
matcher's `resource_base_eq`, which every builtin overload goes through.
[[src/codegen/resource/mod.rs:base_resource_name]]
[[src/codegen/registry/mod.rs:resource_base_eq]]

The thread resource plane is structurally distinct: it is an **infix** ` RES `
clause between message and `" TO "`, not a leading prefix — see threads below.

## Thread types

`parse_thread_type_name` handles `Thread`/`ThreadWorker` bodies after `<kind> OF`.
The base token's case is canonicalized to exactly `Thread` or `ThreadWorker`. The
body has three shapes, and a resource-only thread defaults its message to
`Nothing`: [[src/ast/expr.rs:parse_thread_type_name]]

```
Thread OF Msg TO Out               ' data-only
Thread OF Msg RES Res TO Out       ' data + resource planes
Thread OF RES Res TO Out           ' resource-only (message defaults to Nothing)
```

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
(`List OF`, `Set OF`, `Map OF`, `MapEntry OF`, `Result OF`, `Thread OF`,
`ThreadWorker OF`, and the `FUNC(`/`ISOLATED FUNC(` prefixes); only a base that
is none of these is treated as a user template. The remainder after `" OF "` is
split on top-level `", "` into the argument list. Monomorph's private
`user_template_parts` was retired into this arm by plan-105-B, so the exclusion
list exists once. [[src/types.rs:parse]]
[[src/codegen/builtins/mod.rs:split_top_level_commas]]

The resolver no longer applies this precedence itself — it receives the decoded
`UserOf` and resolves the base and each argument. [[src/resolver/resolution.rs:resolve_type]]
The template machinery itself is [language templates](./mfb spec language
templates).

## Round-trip: render out, parse back

The encoding's defining property is that `ParameterType::parse(name).name()`
returns `name` byte-for-byte, for every spelling this document describes. That
round trip is load-bearing at the wire seams — the `.mfp` type section, the IR
binary, and the manifest all store the *rendered* spelling and read it back —
and it is asserted directly. It is also the reason plan-111 could retype the
whole pipeline without touching a single golden: rendering is lossless, so the
bytes a stage emits do not depend on whether it held a string or a variant tree. [[src/types.rs:parse]] [[src/types.rs:name]]

Map/MapEntry bodies are split with `split_top_level_to` (a `" TO "`
`split_once`) and function/template argument lists with `split_top_level_commas`
(a `", "` split). Both live behind `parse`. [[src/types.rs:split_top_level_to]]
[[src/codegen/builtins/mod.rs:split_top_level_commas]]

**A new type shape is added in one place: a `ParameterType` variant, with its
`parse` and `name` arms.** This is the inversion plan-111 performed. It used to
be true that a shape had to be added in lockstep to `parse_type_name`,
`resolve_type_name`, `concrete_type_name` and its sibling substitution passes,
the source checker, and the IR semantic verifier, because each re-derived the
grammar from the string; `concrete_type_name` and its siblings are deleted, and
the rest match variants. The remaining obligation is the opposite one, and it is
quieter: every consumer has a `_` arm, so an **unwired** variant is silently
mis-handled rather than failing to compile. Adding a variant means auditing the
matches that need it — the resolver, `ir::shape`, `ir::verify`, monomorph's
`unify`/`normalize`, and the `TypeModel` builder — not just `types.rs`.

`Stateful` is the worked example, and it cost a real bug. Before plan-111 a
stateful resource was one opaque `Named("Stream STATE Cursor")` whose spelling
`split_state` string-split on demand; afterwards it is a variant. Two guards
elsewhere asked `matches!(t, Named(_))` meaning "is this a nominal?", and both
silently changed answer — `ir::shape::checker_binds_pattern` stopped binding
`CASE Variant(v)` over a stateful union, and `ir::verify`'s member check started
rejecting `.state` on every stateful resource. Neither failed to compile; the
signal was one acceptance fixture that no longer built. **Audit every
`Named(_)` when the new variant is one a `Named` used to stand in for.**

## See Also

* ./mfb spec language types — the source type system these strings denote
* ./mfb spec language templates — the `Name OF A, B` template form
* ./mfb spec language functions — `FUNC(...) AS R` and `ISOLATED` callables
* ./mfb spec language resource-management — the `RES` transfer marker (§15.6)
* ./mfb spec architecture monomorphization — the stage that parses and rebuilds these strings
* ./mfb spec architecture type-inference — how inferred types are spelled in this encoding
