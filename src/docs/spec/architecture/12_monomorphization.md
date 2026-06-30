# Monomorphization

The monomorphizer (`src/monomorph.rs`) takes the parsed, initially-resolved AST
and produces a fully **concrete** AST: every template type and template function
is expanded into a concrete declaration per instantiation, every overloaded call
is rewritten to a single mangled callee symbol, and every `collections::` call is
internalized. The rest of the pipeline consumes this concrete AST, never the
original. Because the pass *introduces* new declarations and rewrites names, the
build runs the resolver again immediately afterward (see
`./mfb spec architecture frontend`).
[[src/monomorph/mod.rs:monomorphize_project]]

This pass is the **single source of truth for overload resolution and symbol
mangling**. Overload resolution does not happen in the resolver or the type
checker; the callees are already mangled before either runs the second time.

## Pipeline position

```text
parsed AST -> resolve -> MONOMORPHIZE -> resolve again -> entry validation -> typecheck -> IR
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
| Parameter overload | `name$ParamType1$ParamType2` | name shared by ≥2 FUNC/SUB (or built-in-named) |
| Return-type overload | `name$ParamTypes$AS$ReturnType` | overload set differing only by return type |

A return-type-disambiguated symbol appends an `AS` segment then the return type
(`args.push("AS"); args.push(return_type)`). Because `AS` is a reserved keyword
and can never be a parameter type, this segment can never collide with a
parameter-distinguished mangled name.
[[src/monomorph/helpers.rs:overload_concrete_name]]

Imported-overload resolution recovers base names by **splitting on `$`** — it
groups a package's exported `Func`/`Sub` symbols by the substring before the
first `$`. [[src/monomorph/helpers.rs:collect_imported_overloads]]

The grammar of the type-token strings that `sanitize_type_name` flattens (and
that `unify_type` parses) is canonical in `./mfb spec language type-name-encoding`.

## Template instantiation

Generic declarations are keyed `name<arg1,arg2>` (a comma-joined dedup key,
distinct from the `$`-mangled emit symbol). The first use of a key triggers
expansion; later uses reuse the already-emitted concrete declaration via
`emitted_function_keys` / `emitted_type_keys`.

### Functions — `instantiate_function`

For a call to a template function: [[src/monomorph/lower.rs:instantiate_function]]

1. Look up the template by base name; non-template names return `None` (the call
   falls through to overload resolution).
2. Reject more arguments than parameters (`TYPE_CALL_ARITY_MISMATCH`). Fewer is
   allowed (trailing defaults).
3. **Unify** each declared parameter type against the actual argument type
   (mapped through `template_view_type`, which re-expands an already-mangled
   concrete type back to its `name OF args` view), binding template parameters in
   `substitutions`. A failure reports `TYPE_CALL_ARGUMENT_MISMATCH`.
4. Require every template parameter to be bound (the `collect::<Option<_>>` fails
   to `None` otherwise — a parameter not reachable from any argument cannot be
   inferred).
5. Mangle the concrete name, lower the body under the substitution, cache it.

### Types — `instantiate_type`

`instantiate_type(name, args)` mangles the concrete name, records the
`concrete -> (base, args)` mapping in `type_instantiations` (used by
`template_view_type` to invert mangling), and on first use lowers the template
under the per-parameter substitution. [[src/monomorph/lower.rs:instantiate_type]] A
constructor instantiates its type either from the expected (contextual) type, or
by unifying field types against the constructor argument types.

## `unify_type`: structural matching with an `Unknown` wildcard

`unify_type(pattern, actual, params, substitutions)` is the structural matcher
driving both function-argument and constructor-field inference.
[[src/monomorph/helpers.rs:unify_type]]

```text
unify(pattern, actual):
  pattern is a template param  -> bind on first use; on repeat require equality
  List OF E                    -> recurse on element
  Result OF S                  -> recurse on success type
  Map OF K TO V                -> recurse on K and V
  MapEntry OF K TO V           -> recurse on K and V
  Thread/ThreadWorker(...)     -> kind must match; recurse message/resource/output
  user-template (N OF A...)    -> names equal, arity equal, recurse each arg
  FUNC(P...) AS R              -> arity equal, recurse each param and the return
  otherwise                    -> pattern == actual  OR  actual == "Unknown"
```

A template parameter binds to whatever it first meets and must be **equal** on
every later occurrence (so `box<T>` called with mismatched `T` positions fails).
`Unknown` is the wildcard: an empty `[]` literal types as `List OF Unknown`, and
the terminal rule `actual == "Unknown"` lets it unify against any concrete
pattern. The resource slot of a `Thread` type is optional — both-present unify
recursively, both-absent succeed, mismatched presence fails. See
`./mfb spec language templates` for the source-level template semantics this
implements.

## `expression_type`: the local inference engine

`expression_type(expr, context)` computes the argument types fed into unification
and overload resolution. [[src/monomorph/lower.rs:expression_type]] It is a small,
context-driven inference pass — not the type checker — and returns `None` when a
type cannot be determined locally.

| Expression | Inferred type |
|------------|---------------|
| string / number / boolean literal | `String` / `Integer` or `Float` (by `.`) / `Boolean` |
| `NOTHING` | `Nothing` |
| identifier | `context.locals` then `context.function_types` |
| constructor | `Error`, `Ok`→`Result OF Unknown`, or a known record name |
| list literal | `List OF <first element>` else `List OF Unknown` |
| map literal | `Map OF <key> TO <value>` |
| member access | the record field's declared type |
| call | the callee's recorded return type (`function_returns`) |
| lambda | `FUNC(params) AS <body type>`; assignment-bodied → `AS Nothing` |
| binary | `Boolean` for comparisons/logicals, `String` for `&`, else numeric promotion |
| unary | `Boolean` for `NOT`, else the operand type |
| trapped | the inner expression's type |

The `FunctionContext` carries `locals`, `function_returns`, `function_types`,
`record_fields`, and `enclosing_return`. It is seeded by `function_context` from
every concrete function and record, then extended as statements are lowered:
`LET` records the bound type, `FOR` promotes the loop variable's numeric type,
`FOR EACH` strips `List OF`/`Map OF` to the element/entry type, match-`Union`
patterns and lambda params bind their declared types, and the `RETURN` slot is
typed by `enclosing_return`. A newly-mangled callee is added to the context
(`add_function_to_context`) so a later inference sees its return type.

## Overload resolution

A call is lowered in `lower_expression`'s `Call` arm, which tries, in order:

```text
1. instantiate_function          (template generic expansion)
2. resolve_general_builtin_override  (overridable built-in, gap-fill only)
3. resolve_overload              (ordinary user overload set)
4. resolve_imported_overload     (imported package overload)
5. <unchanged callee>            (bare name; codegen/builtins dispatch)
```

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

`resolve_general_builtin_override` fires for a user FUNC/SUB whose name is an
overridable general built-in (even a sole declaration). The built-in is
authoritative for the types it already supports: an override is selected **only
when** `general::resolve_call` rejects the argument types (the gap-fill rule). A
non-matching call keeps the bare built-in name for codegen dispatch.
[[src/monomorph/lower.rs:resolve_general_builtin_override]] A user FUNC whose name is an
overridable built-in is *always* force-mangled at registration so its codegen
symbol never equals the built-in's dispatch name.

### Expected-type propagation

The expected (contextual) type that selects among return-type overloads is
threaded through `lower_expression`'s `expected_type` parameter and set at these
sites:

| Site | Source of expected type |
|------|-------------------------|
| `LET … AS T = …` | the declared type `T` |
| `RETURN <call>` | the enclosing function's return type (only when the operand is a call) |
| call argument slot | the selected parameter's declared type — **only when the argument is itself a call** [[src/monomorph/helpers.rs:arg_slot_expected]] |
| list literal element | the `List OF` element of the surrounding expected type |
| typed constructor field | the field's declared type |

The argument-slot expectation is supplied only when the callee names **exactly
one** user function (`single_signature_params` returns `None` for an overloaded,
package, or unknown callee), because that is the only place a return-type set
nested as an argument can resolve. [[src/monomorph/lower.rs:single_signature_params]]
Literals always keep their own inferred typing, never an expected type, in the
argument-slot and return positions.

## Imported overloads across the package boundary

Overloads exported by an imported package are resolved against compiled `.mfp`
metadata, since the importer has no AST for them.
[[src/monomorph/helpers.rs:collect_imported_overloads]]

`collect_imported_overloads` runs once at construction. For each distinct
`(binding, package)` import pair it:

1. Records the qualifier prefixes `binding.` and `package.` for later
   normalization.
2. Reads the package's exports (`binary_repr::read_package_exports`).
3. Keeps only `Func`/`Sub` exports, groups them by base name (split on `$`).
4. Records `(param_types, package.mangledName)` **only** for a base name with
   ≥2 exports — a non-overloaded import resolves by its bare name and is skipped.

`resolve_imported_overload(callee, arg_types)` then finds the candidate whose
arity and per-position `types_compatible` match, returning the
package-qualified mangled name the package merge expects.
[[src/monomorph/lower.rs:resolve_imported_overload]] `types_compatible` is token-wise
equality with `Unknown` as a wildcard on either side.
[[src/monomorph/lower.rs:types_compatible]] Both the declared parameter type and the
actual argument type are first run through `normalize_type`, which strips every
known qualifier prefix so an importer's `sqlite.Db` matches the package's bare
`Db`. [[src/monomorph/lower.rs:normalize_type]]

## `collections::` internalization

A call whose callee is `binding.member` where `binding` is bound to the built-in
`collections` package (or an alias) and `member` is a collections function is
rewritten to the internal generic implementation `__collections_<member>` before
instantiation, so it expands like any other generic function.
[[src/monomorph/lower.rs:collections_internal_callee]] The set of collections binding
names is computed once from the imports (`collections::collections_bindings`).
Non-collections calls return the callee unchanged.

## See Also

* ./mfb spec architecture frontend — pass ordering and the double resolve
* ./mfb spec architecture ir — what consumes the concrete AST
* ./mfb spec language type-name-encoding — the type-string grammar these algorithms parse
* ./mfb spec language templates — source template semantics
* ./mfb spec language functions — overload source rules
* ./mfb spec language type-inference — the broader inference model
