# 3. Templates

MFBASIC supports monomorphized templates, not runtime generics.

Template parameters may appear only on `TYPE`, `UNION`, `FUNC`, and `SUB` declarations. A template is not a runtime entity and is not emitted to binary representation as an open declaration. Every used instantiation is resolved during compilation into a concrete declaration before IR, binary representation, package metadata, or native lowering is produced.

Built-in type constructors such as `List`, `Map`, and `Thread` are compiler-owned templates. User code may define templates with the same `OF` syntax where allowed:

```basic
TYPE Stack OF T
  items AS List OF T
END TYPE

FUNC push OF T(s AS Stack OF T, value AS T) AS Stack OF T
  RETURN WITH s { items := collections::append(s.items, value) }
END FUNC

SUB printValue OF T(value AS T)
  io::print(toString(value))
END SUB
```

Template arguments are inferred only from explicit argument, parameter, field, and expected result types by simple unification. There is no general inference engine, no trait system, no variance, no higher-kinded types, no boxing, and no runtime template dispatch.

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

Exported templates in source packages are instantiated by the importing compilation before binary representation is produced. A compiled `.mfp` package contains only concrete template instantiations; it does not expose templates for later instantiation unless a future package format explicitly adds signed template metadata.
