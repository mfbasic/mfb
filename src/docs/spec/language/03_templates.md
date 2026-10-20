# 3. Templates

MFBASIC supports monomorphized templates, not runtime generics.

**Current package limitation.** Open user templates are available only within
the compilation that owns their source. An ordinary library cannot export an
open template for specialization over an importer-defined type. This also
applies to source dependencies: they are compiled to `.mfp` before the importer,
which receives concrete declarations rather than the template source. This is a
current restriction, not a promise about a future language version.
[[src/cli/build/source_packages.rs:build_source_dependencies]]
[[src/monomorph/lower/mod.rs:into_project]]

Template parameters may appear only on `TYPE`, `UNION`, and `FUNC` declarations. A template is not a runtime entity and is not emitted to binary representation as an open declaration. Every used instantiation is resolved during compilation into a concrete declaration before IR, binary representation, package metadata, or native lowering is produced.

Built-in type constructors such as `List`, `Map`, and `Thread` are compiler-owned templates. User code may define templates with the same `OF` syntax where allowed.

A template with one parameter is written `OF T`. A template with more than one is written with a parenthesized list, at the declaration and at every use: `TYPE Pairing OF (A, B)`, `FUNC flip OF (A, B)(p AS Pairing OF (A, B)) AS Pairing OF (B, A)`, `LET p AS Pair OF (Integer, String)`. A bare `OF T` takes exactly one argument and never reads past a comma, so a template-typed parameter may sit anywhere in a parameter list, `FUNC(Box OF Integer, Integer) AS Integer` is a two-parameter function type, and a multi-argument template nests without ambiguity (`Holder OF Pairing OF (Integer, String)`). The bare multi-argument spelling `Pair OF Integer, String` is a parse error (`MFB_PARSE_UNEXPECTED_TOKEN`) that names the parenthesized form, reported at the comma wherever no enclosing list could own it. [[src/ast/expr/type_names.rs:parse_type_name_inner]] [[src/ast/items.rs:parse_template_params]]

```basic
TYPE Stack OF T
  items AS List OF T
END TYPE

FUNC push OF T(s AS Stack OF T, value AS T) AS Stack OF T
  RETURN WITH s { items := collections::append(s.items, value) }
END FUNC

FUNC printValue OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
```

Template arguments are inferred only from explicit argument, parameter, field, and expected result types. The compiler binds each template parameter by structurally matching declared types against the concrete types at each use site; the matching algorithm, symbol mangling, and instantiation mechanics are specified in `./mfb spec architecture monomorphization`. There is no general inference engine, no trait system, no variance, no higher-kinded types, no boxing, and no runtime template dispatch.

A template is written in one file and every instantiation of it belongs to that file, wherever the use that caused it is written. The names in a template body resolve as they do for any other declaration of the file: an import alias means the package that file's `IMPORT` bound it to, and the file's `PRIVATE` functions, bindings, types and fields are reachable. A compile diagnostic or a runtime `Error.source` inside the body names the declaring file. Only the type arguments come from the use site, so a file may instantiate another file's template with its own `PRIVATE` type or with a type of a package only it imports.

A template parameter of a built-in binds `Nothing` like any other type, so `collections::contains(List OF Nothing, NOTHING)` binds `T = Nothing`. Two positions refuse it. The first is a callback's result type (`transform`'s `FUNC(T) AS U`): a `Nothing`-returning callback does not satisfy a value-producing callback parameter (§6). The second is a slot of a thread handle, where a `Nothing` resource plane means "no resource plane". A refused binding is `TYPE_CALL_ARGUMENT_MISMATCH`. [[src/codegen/registry/mod.rs:unify_at]]

A plain `FUNC`-typed callback parameter (`forEach`/`transform`/`filter`/`reduce`/`sortBy`/… , `json::parse`'s reviver) matches an `ISOLATED FUNC` argument of the same shape — isolation widens in one direction, the same rule §6 states for every other callable-type position. A plain `FUNC` argument never satisfies a parameter that itself requires `ISOLATED` (`thread::start`'s worker).

Template predicates such as comparability, copyability, defaultability, or resource restrictions are checked against each concrete instantiation:

```basic
FUNC getOrDefault OF (K, V)(items AS Map OF K TO V, key AS K, defaultValue AS V) AS V
  IF collections::hasKey(items, key) THEN
    RETURN collections::get(items, key)
  END IF

  RETURN defaultValue
END FUNC
```

The `K` parameter above must be comparable because every concrete `Map` key type must be comparable. The requirement is checked when `getOrDefault` is instantiated, not through a separate bound or trait declaration.

## Publishing a concrete template-backed API

A package can publish explicitly named, non-template wrappers for its supported
types. Calls from those wrappers instantiate the private template while the
package itself is compiled; importers call the exported wrappers:
[[src/monomorph/lower/instantiate.rs:instantiate_function]]

```basic
PRIVATE FUNC identity OF T(value AS T) AS T
  RETURN value
END FUNC

EXPORT FUNC identityInteger(value AS Integer) AS Integer
  RETURN identity(value)
END FUNC

EXPORT FUNC identityString(value AS String) AS String
  RETURN identity(value)
END FUNC
```

These exports do not let an importer instantiate `identity` for its own record.
Any package-owned type named by an exported signature must itself meet the
export-visibility rules in `./mfb spec language modules-and-packages`.

Compiler-owned source generics are a separate mechanism: built-in `collections`
source is injected into the importing compilation before monomorphization.
Ordinary dependencies have no corresponding source-injection interface.
See `./mfb spec architecture monomorphization` for the built-in routing.
[[src/ast/manifest.rs:parse_project]]
[[src/codegen/builtins/collections/mod.rs:augmented_project]]

## See Also

* ./mfb spec architecture monomorphization — the unification, mangling, and instantiation algorithm that expands templates
