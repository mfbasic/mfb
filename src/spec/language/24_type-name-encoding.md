# Type-Name Encoding

Every type in MFBASIC is carried between compiler stages as a single **flat
string** — never a structured AST node. The parser builds this string when it
reads a type annotation; the resolver, monomorphizer, type checker, and IR all
**re-derive structure by prefix-stripping and separator-splitting** the same
string. This document is the canonical contract for that encoding. A mismatch in
spacing, keyword casing, or separator width silently breaks every consumer, so
the grammar below is exact down to single spaces.

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
| Map | `Map OF K TO V` |
| Resource-transfer map value | `Map OF K TO RES V` |
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

`OF`, `TO`, and `AS` are **infix keywords**: every downstream consumer recovers
them by `strip_prefix`/`split_once` on the surrounding literal, not by tokenizing.
The resolver, for example, dispatches purely on `strip_prefix("List OF ")`,
`strip_prefix("Map OF ")`, `split_once(" TO ")`, `split_once(") AS ")`.
[[src/resolver/resolution.rs:resolve_type_name]] [[src/monomorph/helpers.rs:func_type_parts]]

## Base names and bare-id normalization

`parse_type_base_name` reads one identifier (or the `Nothing` keyword) as the
base. A **package-qualified built-in type** is normalized here, at parse time, to
its bare internal id: `net::Url` becomes `Url`, `http::Response` becomes
`Response`, so no downstream stage ever sees a qualified built-in type. The
rewrite is `qualified_builtin_type`, which only fires when the qualifier is a
built-in import **and** the member is a built-in type id; otherwise the dotted
name passes through unchanged. [[src/ast/expr.rs:parse_type_base_name]]
[[src/builtins/mod.rs:qualified_builtin_type]]

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
  package-qualified resolution. [[src/resolver/resolution.rs:resolve_type_name]]

A non-built-in user/dependency type therefore keeps its dotted qualifier in the
flat string; only built-in package types are stripped to bare ids.

## The `RES` resource-transfer prefix

A leading `RES ` on a collection element or value marks a **resource-transfer
collection** ([language resource-management](./mfb spec language
resource-management), §15.6): the element/value is a resource borrow whose
scope-ownership transfers across a function boundary.

| Position | Canonical form | Notes |
|----------|----------------|-------|
| List element | `List OF RES File` | `RES` consumed only for `List`, not `Result` |
| Map value | `Map OF K TO RES File` | prefix sits after `" TO "` |
| Thread resource plane | `Thread OF Msg RES Res TO Out` | infix `RES` clause |

`parse_type_name` accepts `RES` after `List OF` (the `Result` base accepts the
keyword token but the marker is harmless and later rejected by type checking) and
after `Map ... TO`. Consumers strip it with
`strip_prefix("RES ").unwrap_or(...)` before resolving the underlying type.
[[src/ast/expr.rs:parse_type_name]] [[src/resolver/resolution.rs:resolve_type_name]]

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
resource-only thread. [[src/builtins/thread.rs:format_thread_type]]
[[src/builtins/thread.rs:thread_parts_full]]

Because a thread output may itself be a grouped or nested type, the thread body
is split by measuring a balanced type prefix (`type_prefix_len`) rather than a
naive `split_once`, and each segment is unwrapped of redundant grouping by
`strip_type_group`. [[src/builtins/thread.rs:split_thread_types]]
[[src/builtins/thread.rs:strip_type_group]]

## User templates

`user_template_parts` decodes the `Name OF A, B` form. It first **excludes**
every built-in `OF`-bearing shape (`List OF`, `Map OF`, `MapEntry OF`,
`Result OF`, `Thread OF`, `ThreadWorker OF`, and the `FUNC(`/`ISOLATED FUNC(`
prefixes); only a base that is none of these is treated as a user template. The
remainder after `" OF "` is split on top-level `", "` into the argument list.
[[src/monomorph/helpers.rs:user_template_parts]] [[src/monomorph/helpers.rs:split_top_level_commas]]

The resolver applies the same precedence: it checks the built-in prefixes first,
then treats `base OF args` as a template only when `base` is a known type or an
active template parameter, splitting `args` on `", "`. [[src/resolver/resolution.rs:resolve_type_name]]
The template machinery itself is [language templates](./mfb spec language
templates).

## Round-trip: rebuild by prefix-stripping

The encoding's defining property is that it round-trips through pure string
operations. `concrete_type_name` (the monomorphizer's substitution pass)
reconstructs each form by the identical prefix tests the parser used to build it,
recursing on the sub-strings and re-joining with the same separators:
[[src/monomorph/lower.rs:concrete_type_name]]

```
strip_prefix("List OF ")        -> "List OF " + recurse(element)
strip_prefix("Result OF ")      -> "Result OF " + recurse(success)
strip_prefix("Map OF ") + split_once(" TO ")   -> "Map OF " K " TO " V
func_type_parts + ") AS "       -> prefix + params.join(", ") + ") AS " + R
thread_parts_full               -> format_thread_type(kind, msg, res, out)
user_template_parts             -> instantiate_type(name, args)
```

Map/MapEntry bodies are split with `split_top_level_to` (a `" TO "`
`split_once`) and function/template argument lists with `split_top_level_commas`
(a `", "` split). [[src/monomorph/helpers.rs:split_top_level_to]]
[[src/monomorph/helpers.rs:func_type_parts]] Any new type shape must be added in lockstep
to **all** of: `parse_type_name`, `resolve_type_name`, `concrete_type_name`
(plus its sibling substitution passes), and the type checker — there is no shared
parser to change in one place.

## See Also

* ./mfb spec language types — the source type system these strings denote
* ./mfb spec language templates — the `Name OF A, B` template form
* ./mfb spec language functions — `FUNC(...) AS R` and `ISOLATED` callables
* ./mfb spec language resource-management — the `RES` transfer marker (§15.6)
* ./mfb spec architecture monomorphization — the stage that parses and rebuilds these strings
* ./mfb spec language type-inference — how inferred types are spelled in this encoding
