# Monomorphization

The monomorphizer takes the augmented generic HIR after initial resolution[[src/monomorph/]]
and produces a fully **concrete** HIR: every template type and template function
is expanded into a concrete declaration per instantiation, each resolved call to
an overloaded `FUNC` is rewritten to a single mangled callee symbol, and
registry-designated source-generic `collections::` calls are
internalized. The rest of the pipeline consumes this concrete HIR, never the
original. Because the pass *introduces* new declarations and rewrites names, the
build runs the resolver again immediately afterward (see
`./mfb spec architecture frontend`).
[[src/monomorph/mod.rs:monomorphize_project]]

This pass owns **user `FUNC` overload resolution and symbol mangling**. Those
callees are already mangled before the second resolver pass and type checking.
Registry-defined built-in calls also have signature and named-argument layout
selection outside this pass, including trailing-default handling; see
`./mfb spec language functions`.
[[src/codegen/builtins/mod.rs:select_param_name_overload]]

## Pipeline position

```text
AST -> augment/elaborate/resolve -> generic HIR -> MONOMORPHIZE -> concrete HIR -> resolve again -> entry validation -> shape pass -> IR -> IR verifier
```

`monomorphize_project` builds a `Monomorphizer`, calls `run` to lower the
seed (non-template) types and functions, then `into_project` to emit the result.
Any diagnostic sets `had_error` and the pass returns `Err(())`.

## Symbol scheme: `$`-delimited concrete names

The pass uses one string form for every generated symbol — a base name followed
by `$`-joined, sanitized type tokens. This is the symbol form that IR,
package metadata, package merge, and the linker all consume.
[[src/monomorph/helpers.rs:mangle_name]]

```text
mangle_name(name, [T1, T2])      -> name$<san(T1)>$<san(T2)>
sanitize_type_name(t)            -> every char not [A-Za-z0-9_] becomes '$'
```

`sanitize_type_name` maps each non-alphanumeric, non-underscore character to `$`,
so a structured type string flattens to a `$`-only-delimited token.
[[src/monomorph/helpers.rs:sanitize_type_name]] For example `List OF Integer` →
`List$OF$Integer`, and a template instantiation `box<Integer>` mangles to
`box$Integer`.

The scheme has three producers:

| Producer | Form | When |
|----------|------|------|
| Template instantiation | `name$Arg1$Arg2` | generic FUNC/TYPE expanded for inferred args |
| Parameter overload | `name$ParamType1$ParamType2` | name shared by ≥2 FUNC (or built-in-named) |
| Return-type overload | `name$ParamTypes$AS$ReturnType` | overload set differing only by return type |

A return-type-disambiguated symbol appends an `AS` segment then the return type
(`args.push("AS"); args.push(return_type)`). Because `AS` is a reserved keyword
and can never be a parameter type, this segment can never collide with a
parameter-distinguished mangled name.
[[src/monomorph/helpers.rs:overload_concrete_name]]

**An `EXPORT` counts only its `EXPORT` siblings.** Both "shared by ≥2" and
"differing only by return type" are measured, for an `EXPORT` function, over the
`EXPORT` members of its name; a `PUBLIC`/`PRIVATE` member counts the whole set. An
export's concrete symbol is its name in the `.mfp` export table and ABI index, and
importers see only exports, so a hidden sibling must not rename it: `EXPORT f(String)`
beside `PUBLIC f(Integer, Integer)` is emitted as `f` (the PUBLIC one as
`f$Integer$Integer`), and adding or removing that helper leaves the package's exports
byte-for-byte unchanged. A non-exported member is mangled whenever any sibling exists,
so its symbol cannot equal an export's. Intra-package calls are unaffected: they
resolve through `overload_names`, which maps each declaration to the symbol it was
given. [[src/monomorph/lower.rs:Monomorphizer::new]]

**A `PRIVATE` member's set is its file's.** File-private scoping renames a `PRIVATE
FUNC f` to `#<hash>$f`, and every reference to `f` inside that file with it, but
leaves the file's own `PUBLIC`/`EXPORT FUNC f` declarations named `f`.
[[src/ast/scope_privates.rs:scope_privates]] The overload set of the file-scoped
name is therefore built as the `PRIVATE` declarations plus those same-file
siblings, each keeping the name it was declared under: a call inside the file
selects among all of them and is rewritten to the chosen member's own symbol,
while the set a bare `f` names from any other file never holds a `PRIVATE`
member. A `PRIVATE` member counts that whole file-scoped set, so it is mangled
(`#<hash>$f$String`) whenever a same-file sibling exists; the `PUBLIC`/`EXPORT`
siblings are counted and named by the bare name's set alone, so a `PRIVATE`
helper never renames them — `EXPORT r(Integer)` beside `PRIVATE r(String)` is
exported as `r`. When the file's `PRIVATE f` is a template and its `PUBLIC f` is
not (or the reverse), the file-scoped name reaches the `PUBLIC` declaration too,
instantiated or called under its own name.
[[src/monomorph/lower.rs:Monomorphizer::new]]
[[src/monomorph/lower.rs:resolve_overload]]
[[src/monomorph/lower.rs:template_name]]

Imported-overload resolution recovers base names by **splitting on `$`** — it
groups a package's exported `Func`/`Sub` symbols by the substring before the
first `$`. [[src/monomorph/helpers.rs:collect_imported_overloads]]

`sanitize_type_name` flattens a RENDERED type into a symbol-safe token; the
grammar it renders from is canonical in
`./mfb spec architecture type-name-encoding`. `unify_type` does not parse — it
matches `ParameterType` variants, so a new type shape reaches it as a
new variant rather than a new prefix test.

## Template instantiation

Generic declarations are keyed `name<arg1,arg2>` (a comma-joined dedup key,
distinct from the `$`-mangled emit symbol). The first use of a key triggers
expansion; later uses reuse the already-emitted concrete declaration via
`emitted_function_keys` / `emitted_type_keys`.

Fresh template expansion is bounded on two independent axes. **Depth:** at most
256 active instantiation frames — a chain that recursively creates distinct
concrete templates beyond that limit is rejected with
`TYPE_INSTANTIATION_TOO_DEEP` instead of recursing until the host stack overflows.
**Breadth:** at most `MAX_TOTAL_INSTANTIATIONS` (a few thousand) concrete
declarations — functions plus user types — across the whole project, counted by
the shared `total_instantiations` counter. The depth cap bounds a single
recursion *path*; a generic that recurses through ≥2 type-widening self-calls
(e.g. `recurse<T>` → `recurse<List OF T>` + `recurse<Set OF T>`) instead fans out
into an exponential *tree* of distinct keys the depth cap never collapses, so the
breadth budget rejects it with `TYPE_INSTANTIATION_BUDGET_EXCEEDED`. Either limit
latches `instantiation_limit_reached`, which halts all further expansion after a
single bounded diagnostic rather than re-reporting on every one of the
(exponentially many) sibling leaves.

### Functions — `instantiate_function`

For a call to a template function: [[src/monomorph/lower.rs:instantiate_function]]

1. Look up every template of the base name (a name may carry several: a template
   overloads by signature like a plain `FUNC`, bug-913); non-template names return
   `None` (the call falls through to overload resolution). A plain overload of the
   name whose parameter types match the call exactly wins: `None` again.
2. For each template, **bind** it (`bind_template`): more arguments than
   parameters do not bind (`TYPE_CALL_ARITY_MISMATCH` for a lone template; fewer is
   allowed — trailing defaults — but when the name has several candidates a
   template whose unsupplied parameter has no default does not bind). **Unify**
   each declared parameter type against the actual argument type (mapped through
   `template_view_type`, which re-expands an already-mangled concrete type back to
   its `name OF args` view), binding template parameters in `substitutions`; a
   failure reports `TYPE_CALL_ARGUMENT_MISMATCH` for a lone template. Every
   template parameter must be bound (a parameter not reachable from any argument
   cannot be inferred). Named arguments are reordered into each template's own
   parameter order first (bug-196).
3. A name with one template and no plain overload reports its binding failure as
   above. Otherwise the call selects among the templates that bind: none —
   `TYPE_CALL_ARGUMENT_MISMATCH` when the name has no plain overload either (else
   the plain overload set reports); one — that template; several — the **most
   specific** (`most_specific`): template `a` is at least as specific as `b` when
   `b`'s parameter patterns bind `a`'s own parameter types, so `f OF T(x AS List OF
   T)` beats `f OF T(x AS T)` for `f([1])`. No single strictly most specific
   template is `TYPE_OVERLOAD_AMBIGUOUS`.
4. The instantiation key is `name<args>` for a name with one template, and
   `name(<param types>)<return><args>` — the template's identity — for a name with
   several, so two templates instantiated at the same arguments stay two concrete
   functions (`unique_concrete_symbol` suffixes the second symbol). Each
   instantiation symbol maps back to the called name in the bug-697 render table,
   so a downstream diagnostic about the call names `f`, never `f$Integer`.
5. Mangle the concrete name, charge one against the total-instantiation budget
   (`TYPE_INSTANTIATION_BUDGET_EXCEEDED` once exhausted) and reject expansion past
   the 256-frame depth limit (`TYPE_INSTANTIATION_TOO_DEEP`), then lower the body
   under the substitution and cache it.

### Types — `instantiate_type`

`instantiate_type(name, args)` mangles the concrete name, records the
`concrete -> (base, args)` mapping in `type_instantiations` (used by
`template_view_type` to invert mangling), and on first use lowers the template
under the per-parameter substitution after the same total-instantiation budget
and 256-frame depth cap used for functions. [[src/monomorph/lower.rs:instantiate_type]] A constructor
instantiates its type either from the expected (contextual) type, or by unifying
field types against the constructor argument types.

### Where an instantiation is emitted

`into_project` emits each generated instantiation — function or type — into the
source file that **declares its template**, after that file's own items (types
before functions, each sorted by concrete name). Every later stage keyed on a
declaration's file therefore sees the scope the template body was written in:
the post-monomorph resolve binds the body's import aliases through that file's
`IMPORT`s and reaches that file's `PRIVATE` declarations, the constructor and
field visibility checks treat an instantiated `PRIVATE` type or `PRIVATE` field
as that file's, a diagnostic raised in the body cites that file and line, and
`IrFunction::file` — the `ErrorLoc.filename` of a runtime failure in the body —
names it. The result does not depend on the order of the source files.
[[src/monomorph/lower.rs:into_project]]

A template the toolchain declares — a built-in package's injected source, the
prelude's `Pair`/`Partition` — has no user file. Its instantiations are emitted
into the project's **first** source file, and a function among them is marked
`HirFunction::internal_origin` and carries its own call-site attribution (see
"Diagnostics inside a built-in package's body").

A hosted body can name a package its file never imported: a type argument is
spelled where the call is written, as the canonical `<package>.<Type>`, and a
built-in package's body keeps calling that package's helpers. So the resolution
scope (`HirFile::imports`) of every file that hosts an instantiation, and always
of the first file, is widened with the other files' imports, in source-file
order. The hosting file's **own bindings win**: an alias it binds means, in its
template bodies, the package it bound. A foreign import whose alias is already
taken there is added under the binding `#<package>`, which no source can spell,
unless the file already imports that package — a package-qualified type resolves
through any binding of its package. `HirFile::own_imports` is never widened.
The mangled symbol of an instantiation and a package's export table do not
depend on where the instantiation is emitted.
[[src/monomorph/lower.rs:into_project]] [[src/hir/nodes.rs:HirFile]]

## `unify_type`: structural matching with an `Unknown` wildcard

`unify_type(pattern, actual, params, substitutions)` is the structural matcher
driving both function-argument and constructor-field inference.
[[src/monomorph/helpers.rs:unify_type]]

```text
unify(pattern, actual):
  pattern is a template param  -> bind on first use (an Unknown binding is
                                  provisional: a later concrete refines it, and an
                                  Unknown actual never overwrites a concrete);
                                  two concrete actuals must agree
  List OF E                    -> recurse on element
  #Result OF S                 -> recurse on success type
  Map OF K TO V                -> recurse on K and V
  MapEntry OF K TO V           -> recurse on K and V
  Thread/ThreadWorker(...)     -> kind must match; recurse message/resource/output
  user-template (N OF A...)    -> names equal, arity equal, recurse each arg
  FUNC(P...) AS R              -> arity equal, recurse each param and the return
  otherwise                    -> pattern == actual  OR  actual == "Unknown"
```

A template parameter binds to whatever it first meets and must be **equal** on
every later occurrence between two concretes (so `box<T>` called with mismatched
`T` positions fails). `Unknown` is the wildcard: an empty `[]` literal types as
`List OF Unknown`, and the terminal rule `actual == "Unknown"` lets it unify
against any concrete pattern. An `Unknown` bound to a param is **provisional** — a
later concrete actual refines it (`Unknown → Integer`) and an `Unknown` actual
never overwrites a concrete binding. That makes inference independent of
field/argument order: a no-information field (an empty collection) does not
block a later field that supplies the concrete type. Recording the
provisional `Unknown` rather than dropping it preserves the degenerate
`T := Unknown` instantiation that width-agnostic native ops (e.g.
`collections::flatten` over lists whose first inner is empty) rely on when no
argument supplies a concrete element type. "Equal" ignores a top-level `RES`
ownership marker, which is not part of a type: a `List OF RES fs.File` argument
binds `T := RES fs.File`, a callback over its elements declares `FUNC(fs.File)`,
and the two agree, keeping the marked binding whichever came first (the
verifier's `compatible` likewise compares a callable's parameters modulo the
marker). [[src/ir/verify/compat.rs:compatible]] The resource slot of a `Thread` type is optional — both-present unify
recursively, both-absent succeed, mismatched presence fails. See
`./mfb spec language templates` for the source-level template semantics this
implements.

## `expression_type`: the local inference engine

`expression_type(expr, context)` computes the argument types fed into unification
and overload resolution. [[src/monomorph/lower.rs:expression_type]] It is a small,
context-driven inference pass — not a general type checker — and returns `None`
when a type cannot be determined locally.

| Expression | Inferred type |
|------------|---------------|
| string / number / boolean literal | `String` / `Integer` or `Float` (by `.`) / `Boolean` |
| `NOTHING` | `Nothing` |
| identifier | `context.locals`, then a concrete function's signature type, then the globals |
| constructor | `Error`, or a known record name (a user `Ok` is an ordinary record, bug-945) |
| list literal | `List OF <first element>` else `List OF Unknown` |
| map literal | `Map OF <key> TO <value>` |
| member access | the record field's declared type |
| call | the concrete callee's declared return type |
| lambda | `FUNC(params) AS <body type>`; assignment-bodied → `AS Nothing` |
| binary | `Boolean` for comparisons/logicals, `String` for `&`, else numeric promotion |
| unary | `Boolean` for `NOT`, else the operand type |
| trapped | the inner expression's type |

The `FunctionContext` carries only SCOPE-local state: `locals` and
`enclosing_return`. Everything program-wide is read live off the monomorphizer
at each query — a function's return and signature types from `concrete_functions`
(via `function_signature_types`), a record's fields from `concrete_types`
(`TYPE` declarations only), and an explicitly-typed top-level binding's declared
type from `globals`, which is derived once when the monomorphizer is built.
Nothing is snapshotted into the context, so a newly-mangled callee is visible to
later inference the moment `instantiate_function` records it; no patch step is
needed. Every scope boundary — both `IF` arms, each `MATCH` case, a loop body, a
lambda, a trap handler — clones the context, and the clone is just the locals.

The context is extended as statements are lowered: `LET` records the bound type,
`FOR` promotes the loop variable's numeric type, `FOR EACH` strips `List
OF`/`Map OF` to the element/entry type, match-`Union` patterns and lambda params
bind their declared types, and the `RETURN` slot is typed by `enclosing_return`.

A match-`Union` pattern that names a `TYPE` template (`CASE Some(s)`) carries no
arguments of its own, so `union_pattern_type` takes them from the scrutinee: it
types the lowered scrutinee with `expression_type`, walks that concrete union's
members and, recursively, its `INCLUDES`, in `concrete_types`, and keeps the
members whose `type_instantiations` entry names that template. Exactly one match
becomes the pattern's type and the binding's; none, or more than one, is
reported as `TYPE_MATCH_PATTERN_MISMATCH` in template spellings. Any other
pattern goes through `concrete_type` like every other type position.
[[src/monomorph/lower.rs:union_pattern_type]]
[[src/monomorph/lower.rs:collect_union_instantiations]]

## Overload resolution

A call is lowered in `lower_expression`'s `Call` arm, which tries, in order:

```text
1. instantiate_function          (template generic expansion)
2. resolve_general_builtin_override  (overridable built-in, gap-fill only)
3. resolve_overload              (ordinary user overload set)
4. resolve_imported_overload     (imported package overload)
5. <unchanged callee>            (bare name; codegen/builtins dispatch)
```

None of the four runs when the callee names a binding in scope — a parameter,
`LET`/`MUT`, loop variable, `CASE` or `TRAP` binding, or lambda parameter. A local
shadows a top-level function of the same name, so `pick(2)` through a local
`pick` calls the local even when `pick` is also an overloaded function. The
scope's bound names are tracked in `FunctionContext::bound` whether or not the
binding's type is known.

### `resolve_overload`

`resolve_overload(name, arg_types, expected, line)`: [[src/monomorph/lower.rs:resolve_overload]]

- Returns `None` for an overridable built-in name (those route through
  `resolve_general_builtin_override`) or for a name with ≤1 declaration.
- Filters candidates by `params_match` — **exact arity and exact positional type
  equality, no coercion**. [[src/monomorph/helpers.rs:params_match]] Default values never
  distinguish or fill within an overload set, so every parameter must be supplied.
- One survivor → that overload's mangled symbol via the `overload_key`.
- Several survivors → a **return-type overload set** (identical parameter types,
  differing return). The call's **expected (contextual) type** must uniquely
  select one; otherwise `TYPE_OVERLOAD_AMBIGUOUS`.

The per-overload map key is `overload_key` = `name(p1,p2) AS ReturnType`, so a
return-type set maps each member to its own distinct concrete symbol.
[[src/monomorph/helpers.rs:overload_key]] A name belongs to a return-type set when ≥2 of
its declarations share parameter types (`param_types_eq`), which forces the
`AS`-segment in their concrete names. [[src/monomorph/helpers.rs:param_types_eq]]

### Built-in-named overrides

`resolve_general_builtin_override` fires for a user FUNC whose name is an
overridable general built-in (even a sole declaration). The built-in is
authoritative for the types it already supports: an override is selected **only
when** `general::resolve_call` rejects the argument types (the gap-fill rule). A
non-matching call keeps the bare built-in name for codegen dispatch.
[[src/monomorph/lower.rs:resolve_general_builtin_override]] The check is
kind-aware: it passes the resolver a `TypeKinds` oracle that answers from each
declaration's kind (a source `ENUM`, or an imported package's), so a built-in
that supports enums stays authoritative for them while a record override keeps
its gap-fill dispatch.
[[src/codegen/builtins/mod.rs:resolve_call_return_type_with_kinds]] A user FUNC whose name is an
overridable built-in is *always* force-mangled at registration so its codegen
symbol never equals the built-in's dispatch name.

### Function-value references

A bare name in value position (`HirExpression::Identifier`) that names an
overload set is rewritten to one member's concrete symbol by
`resolve_function_reference`; it has no arguments, so the expected type alone
selects. [[src/monomorph/lower.rs:resolve_function_reference]]

- The set is the one a direct call resolves against: the project's declarations
  of the name with ≥2 members (`function_overloads`), or an imported package's
  `EXPORT` members of `binding.name` (`imported_overloads`). A single
  declaration, an unknown name, or a name a local binding shadows is left
  unchanged.
- An overridable built-in name follows the gap-fill rule instead
  (`resolve_builtin_override_reference`): its user overrides are always
  force-mangled, so the reference is rewritten to the one override whose callable
  type is the expected one, but only when the built-in rejects that type's
  parameter types; otherwise the bare name stays for IR lowering's built-in
  reference handling, and nothing is reported here.
  [[src/monomorph/lower.rs:resolve_builtin_override_reference]]
- The expected type and each member's callable type are first made concrete with
  `concrete_type`, so a generic body's `FUNC(T) AS T` is compared after
  substitution and a user generic compares by its instantiated nominal. A member
  without an `AS` clause has the callable type `… AS Nothing`.
  [[src/monomorph/helpers.rs:reference_signature]]
- A member is selected when its parameter count, each parameter type (ignoring a
  top-level `RES` marker), and its return type equal the expected `FUNC` type's,
  and it is `ISOLATED` whenever the expected type is (an isolated member may stand
  in a plain `FUNC` slot, never the reverse). Imported members compare their
  package types with `types_compatible` over `normalize_type`, as an imported call
  does. [[src/monomorph/helpers.rs:callable_matches]]
- Exactly one selected member → the identifier becomes its concrete symbol (the
  `overload_names` entry, or the package-qualified mangled export name).
- No expected type → `TYPE_OVERLOAD_AMBIGUOUS`, listing the members' callable
  types. An expected type no member has — a different `FUNC` type or a non-callable
  type — → `TYPE_OVERLOAD_NO_MATCH`. Both are reported here, while the set is
  visible; the bare name left behind would otherwise reach the post-monomorph
  resolver after every member was mangled and read as undeclared.

Parameter names and defaults are not part of the comparison, and the reference
never coerces: the selected member's type is exactly the expected one (or its
isolated form).

### Expected-type propagation

The expected (contextual) type that selects among return-type overloads, and
among the members of an overloaded function referenced as a value, is threaded
through `lower_expression`'s `expected_type` parameter and set at these sites:

| Site | Source of expected type |
|------|-------------------------|
| `LET … AS T = …`, top-level `LET`/`MUT … AS T` | the declared type `T` (template parameters substituted, inside `FUNC(...)` types too) [[src/monomorph/helpers.rs:substitute_type_params]] |
| assignment `x = …` | the target local's type |
| `h.state = …` | the handle's `STATE` type |
| `RETURN <call>` / `RETURN <name>` | the enclosing function's return type (only when the operand is a call or a bare name) |
| call argument slot | the selected parameter's declared type — **only when the argument is a call or a bare name** [[src/monomorph/helpers.rs:arg_slot_expected]] |
| list / set literal element | the `List OF` / `Set OF` element of the surrounding expected type |
| map literal key / value | the literal's spelled key / value type — only for a call or bare-name operand |
| typed constructor field | the field's declared type |
| `WITH` update field | the updated field's declared type — only for a call or bare-name operand |

The argument-slot expectation is supplied only when the callee names **exactly
one** user function (`single_signature_params` returns `None` for an overloaded,
package, or unknown callee, and a callee that is a local binding supplies none),
because that is the only place a return-type set nested as an argument, or an
overloaded function passed as a callback, can resolve.
[[src/monomorph/lower.rs:single_signature_params]] Literals always keep their own
inferred typing, never an expected type, in the argument-slot, return, map-entry
and `WITH` positions. [[src/monomorph/helpers.rs:consults_expected]] A callback
passed to an overloaded, package or built-in callee therefore has no expected
type; an overloaded function passed there is `TYPE_OVERLOAD_AMBIGUOUS` and is
written through an annotated binding instead.

## Imported overloads across the package boundary

Overloads exported by an imported package are resolved against compiled `.mfp`
metadata, since the importer has no AST for them.
[[src/monomorph/helpers.rs:collect_imported_overloads]]

`collect_imported_overloads` runs once at construction. For each distinct
imported package (under whichever binding a file imports it) it:

1. Records the qualifier prefixes `binding.` and `package.` of a built-in import
   for later normalization.
2. Reads the package's exports.[[src/binary_repr/]]
3. Keeps only `Func` exports, groups them by base name (split on `$`).
4. Records `(param_types, mangledName, return_type, isolated)` under
   `package.base` **only** for a base name with ≥2 exports — a non-overloaded
   import resolves by its bare name and is skipped. A call selects by
   `param_types`; a value reference to the set (`LET f AS FUNC(…) AS … =
   pkg::name`) also needs the return type and isolation (see "Function-value
   references").

The set is keyed by PACKAGE because import bindings are per file: one alias may
name different packages in different files (bug-914). A `binding.member` callee
reaches its set through the binding of the file being lowered
(`imported_overload_set`, from each file's own import bindings; a global
initializer is lowered in its own file), and the rewrite target is
`binding.mangledName` with that same binding, which the post-monomorph resolver
maps per file. [[src/monomorph/lower.rs:imported_overload_set]]

`resolve_imported_overload(callee, arg_types)` then finds the candidate whose
arity and per-position `types_compatible` match, returning that
`binding.mangledName` target.
[[src/monomorph/lower.rs:resolve_imported_overload]] `types_compatible` is token-wise
equality with `Unknown` as a wildcard on either side.
[[src/monomorph/lower.rs:types_compatible]] The match must be **unique**: because
`Unknown` (from an untyped `[]` literal) matches any element type, a call like
`f([])` against exports `f(List OF Integer)` and `f(List OF String)` matches both
and is `TYPE_OVERLOAD_AMBIGUOUS` — annotate the argument
(`LET empty AS List OF String = []`) to select one. Taking the first candidate
would silently bind the call to whichever overload the package exported first. Both the declared parameter type and the
actual argument type are first run through `normalize_type`, which strips every
known qualifier prefix so an importer's `sqlite.Db` matches the package's bare
`Db`. [[src/monomorph/lower.rs:normalize_type]]

## `collections::` internalization

A call whose callee is `binding.member` where `binding` is bound to the built-in
`collections` package (or an alias) and `member` is a collections function is
looked up by member and arity in `registry::rewrite_target_for_arity`. A matching
source-generic member is rewritten to its sigil-internalized implementation
(such as `#collections_sort`) before instantiation. Native-direct members keep
their qualified builtin target.
[[src/monomorph/lower.rs:collections_internal_callee]] The set of collections binding
names is computed once from the imports (`collections::collections_bindings`).
Non-collections calls return the callee unchanged.

### Diagnostics inside a built-in package's body

An instantiation of a built-in package's template is emitted into the project's
first source file (see "Where an instantiation is emitted"), so the file it is
checked in is that file while its statements keep the line numbers of the package
source. A rule that fails while checking the body — an operator the
element type does not support (`collections::sort` over `Boolean`), a
comparability requirement (`collections::distinct` over a `FUNC`) — would cite a
line past the end of the user's file. So an instantiation of a template declared in
an internal (built-in package) file records every user call that reaches it,
`HirFunction::instantiated_at`: the call's file and line and the member as the user
spells it (`registry::public_member_for_rewrite` maps `#collections_sort` back to
`collections::sort`). A later call that reuses the cached instantiation appends its
own site, and an instantiation made from inside such a body inherits the outermost
user call, so a nested helper is attributed to it too.
[[src/monomorph/lower.rs:user_instantiation_site]] [[src/hir/nodes.rs:InstantiationSite]]

The three passes that can report inside the body relocate to those sites: the
monomorphizer's own `report` (to the current site), `ir::shape`'s `emit`, and
`ir::verify`'s `emit`, which reads the sites from `LinkSpans::instantiation_sites`
because the IR carries no call position. Each emits the diagnostic once per site,
at the call's line, with the detail prefixed by "In the body of `collections::sort`,
instantiated by this call:". [[src/ir/verify/mod.rs:TypeEnv]]
[[src/ir/lower_link.rs:LinkSpans]] The same public spelling replaces the internal
`__collections_sort` in the call-level arity and inference messages
`instantiate_function` reports.

## See Also

* ./mfb spec architecture frontend — pass ordering and the double resolve
* ./mfb spec architecture ir — what consumes the concrete HIR
* ./mfb spec architecture type-name-encoding — the type-string grammar these algorithms parse
* ./mfb spec language templates — source template semantics
* ./mfb spec language functions — overload source rules
* ./mfb spec architecture type-inference — the broader inference model
