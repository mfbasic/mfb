# 6. Functions

Only `FUNC` (returns a value) and `SUB` (no value). No methods.

```basic
FUNC greet(name AS String, greeting AS String = "Hello") AS String
  RETURN greeting & ", " & name & "!"
END FUNC

SUB log(msg AS String)
  io::print("[log] " & msg)
END SUB
```

- **Every function may fail.** `FUNC F(...) AS T` yields a `T` on success and an `Error` on failure. A `SUB` yields nothing on success and may still fail (see §7).
- **Default args** allowed (trailing).
- **Named args** at call site: `greet("Ada", greeting := "Hi")`. Named arguments bind by parameter name, may be mixed with positional arguments, and are evaluated/lowered in declaration order after omitted default parameters are filled.
- **Overloading.** Several `FUNC`s or `SUB`s may share one name as long as their signatures differ. A callable's identity is its name together with its **ordered parameter types and return type**; two declarations collide (`SYMBOL_DUPLICATE_TOP_LEVEL`) only when *all three* match (default values never distinguish an overload). Overloads usually differ by **arity** or **parameter type** and are selected bottom-up from the argument types. They may also differ **only by return type** (identical parameter lists): such a *return-type overload set* is selected by the call's **expected (contextual) type** — the declared type of the assignment/`LET`/`DIM` target, the parameter type of the argument slot, the enclosing function's return type for a `RETURN` operand, or the element/field type of a typed initializer. When two or more return-type overloads remain and no expected type uniquely selects one, the call is a `TYPE_OVERLOAD_AMBIGUOUS` error; the fix is a type annotation (e.g. `LET b AS List OF Byte = utf8Encode(s)`). Overloads may be declared across the files of one package, and an `EXPORT`ed overload set is resolved across the package boundary by importers.

  ```basic
  FUNC area(width AS Float, height AS Float) AS Float   ' rectangle
    RETURN width * height
  END FUNC
  FUNC area(radius AS Float) AS Float                   ' circle — different arity
    RETURN 3.14159 * radius * radius
  END FUNC
  FUNC area(side AS Integer) AS Integer                 ' square — different type
    RETURN side * side
  END FUNC
  ```

- **Overload resolution** is by the call's **argument count and positional argument types**: a call binds to the one overload whose parameter count equals the number of supplied arguments and whose declared parameter types match the argument types position by position. Resolution does not rank or prefer among candidates and does not coerce argument types to find a match; if no overload matches exactly, the call is a resolution error. **Default arguments do not combine with overloading**: trailing defaults are filled only for a name that has a single declaration, so within an overload set every parameter — including one declared with a default — must be supplied explicitly. (A name therefore either uses default/omitted arguments *or* is overloaded, not both.)
- **Parameter passing**: arguments are passed as owned values under the memory model (§14). Copyable values are copied when they remain needed by the caller; movable values are moved when ownership can be transferred. Containers own their contents, so passing a container never passes an aliasable reference.
- **Resource parameters**: a parameter whose type is a `RESOURCE` is handled by compiler-known resource rules (§15). Ordinary resource operations borrow the handle for the duration of the call; close operations consume it. MFBASIC source does not add `BORROW` or `MOVE` parameter keywords.
- **Collection boundaries freeze mutable buffers.** When a `MUT` collection is passed to a function or returned from a function, it crosses the boundary as an immutable, owned collection value (§14). The compiler may move or freeze the existing buffer when ownership permits; the semantic guarantee is that no caller and callee can secretly share a mutable collection.
- **Isolated functions**: an exported top-level `FUNC` may be marked `ISOLATED` to declare that it can run as a thread entry point. `ISOLATED` is invalid on `SUB`, lambdas, closures, and local functions.
- **First-class functions & lambdas**:

```basic
LET square = LAMBDA(n AS Integer) -> n * n
FUNC applyTwice(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer
  RETURN f(f(x))
END FUNC
```

- **Closures** capture copyable `LET` bindings by value. Capturing `MUT` is a **compile error** because closures capture values at creation time, not live cells. Capturing resource handles or other non-copyable values is also a compile error in v1. (This is distinct from inner-block reassignment of an outer `MUT`, which is allowed because the scope is still live.)
- **Non-escaping closures** are not part of the v1 source language. Because ordinary closures cannot capture `MUT` bindings, resource handles, or other non-copyable values, the memory model does not require `NONESCAPING`, `BORROW`, or lifetime annotations for closure safety. A future version may add non-escaping closures only if it also specifies local borrow lifetimes and escape diagnostics.
- **Effects are inferred, not annotated, in v1.** The compiler records fallible calls, resource use, thread use, filesystem/network/native access, and package permissions as audit metadata (§22). Source-level effect or purity annotations are reserved for a future version.
- **Recursion** is allowed. Implementations are not required to perform tail-call optimization. A call stack or recursion-depth exhaustion fails with `ErrOutOfMemory` or a more specific future runtime error rather than causing undefined behavior.
