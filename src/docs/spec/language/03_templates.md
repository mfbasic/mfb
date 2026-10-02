# 3. Templates

MFBASIC supports monomorphized templates, not runtime generics.

Template parameters may appear only on `TYPE`, `UNION`, and `FUNC` declarations. A template is not a runtime entity and is not emitted to binary representation as an open declaration. Every used instantiation is resolved during compilation into a concrete declaration before IR, binary representation, package metadata, or native lowering is produced.

Built-in type constructors such as `List`, `Map`, and `Thread` are compiler-owned templates. User code may define templates with the same `OF` syntax where allowed:

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

A template parameter of a built-in binds `Nothing` like any other type, so `collections::contains(List OF Nothing, NOTHING)` binds `T = Nothing`. Two positions refuse it. The first is a callback's result type (`transform`'s `FUNC(T) AS U`): a `Nothing`-returning callback does not satisfy a value-producing callback parameter (§6). The second is a slot of a thread handle, where a `Nothing` resource plane means "no resource plane". A refused binding is `TYPE_CALL_ARGUMENT_MISMATCH`. [[src/codegen/registry/mod.rs:unify_at]]

Template predicates such as comparability, copyability, defaultability, or resource restrictions are checked against each concrete instantiation:

```basic
FUNC getOrDefault OF K, V(items AS Map OF K TO V, key AS K, defaultValue AS V) AS V
  IF collections::hasKey(items, key) THEN
    RETURN collections::get(items, key)
  END IF

  RETURN defaultValue
END FUNC
```

The `K` parameter above must be comparable because every concrete `Map` key type must be comparable. The requirement is checked when `getOrDefault` is instantiated, not through a separate bound or trait declaration.

Templates are instantiated within the compilation that owns their source. A source dependency is compiled to a `.mfp` before the importer is compiled; its open template declarations are not supplied to the importing compilation. Compiled packages expose concrete instantiations, not open templates for new importer-selected types. [[src/cli/build/source_packages.rs:build_source_dependencies]] [[src/monomorph/lower.rs:lower_type]]

## See Also

* ./mfb spec architecture monomorphization — the unification, mangling, and instantiation algorithm that expands templates
