# 6. Functions

`FUNC` is the one kind of callable. No methods.

```basic
FUNC greet(name AS String, greeting AS String = "Hello") AS String
  RETURN greeting & ", " & name & "!"
END FUNC

FUNC log(msg AS String) AS Nothing
  io::print("[log] " & msg)
END FUNC
```

- **Every function may fail.** `FUNC F(...) AS T` yields a `T` on success and an `Error` on failure. A function declared `AS Nothing` yields `NOTHING` on success and may still fail.
- **Effect-only functions** are declared `AS Nothing`. Such a body may end with a bare `RETURN` (or `RETURN NOTHING`), or fall through to `END FUNC`; each succeeds with `NOTHING` (`mfb spec language error-model` §8.6 rules 8–9). Its call is usually a statement, but it is an ordinary value expression too: `LET v = log("x")` binds `NOTHING`, which binds, passes, compares and collects like any other value (§4.6). A call to a built-in declared `AS Nothing` (`io::print`, `fs::writeAll`) is the same. [[src/ir/lower.rs:function_return_type]]
- **Effect-only callbacks.** Naming a function yields a function value of its type, so naming an `AS Nothing` function yields a `FUNC(...) AS Nothing` value — what APIs such as `collections::forEach` take, with no wrapper needed. APIs that infer and collect a result, such as `collections::transform`, reject a callback whose success type is `Nothing`: there would be nothing to collect.

  ```basic
  FUNC printItem(x AS Integer) AS Nothing
    io::print(toString(x))
  END FUNC

  collections::forEach(nums, printItem)
  ```
- **Default args** allowed (trailing). A non-default parameter may not follow a
  defaulted one (the semantic verifier rejects it with `TYPE_DEFAULT_ARG_ORDER`:
  "Parameter … must have a default because an earlier parameter has one").
  A call that omits a defaulted argument receives the default **evaluated on that
  call**, with the default's names resolved **where the function is declared**: the
  declaring file's top-level bindings, functions, imports and own-file `PRIVATE`
  names. A default never sees a caller's locals, and it may not name any parameter
  of its own function — an earlier one, itself, or a later one — because it is
  evaluated outside the function; that is `SYMBOL_DEFAULT_NAMES_PARAMETER`, reported
  at the parameter (to give two parameters the same default, write it twice). The
  same rules hold for a `LINK` function's parameters. A lambda parameter may not
  declare a default at all (`SYMBOL_LAMBDA_PARAMETER_DEFAULT`): a call through a
  function value carries no defaults. A default that names nothing — a string,
  number, scalar or boolean literal, `NOTHING`, or a built-in package constant — is
  passed as that value; any other default is compiled once, in the declaration's
  scope, into a hidden function that every omitting call invokes.
  [[src/ir/lower.rs:default_kind]] [[src/internal_name.rs:hidden_default_function_name]]
  [[src/resolver/resolution.rs:resolve_parameter_default]]
- **Many parameters allowed.** A callable may take **any number of parameters**.
  The first eight are passed in registers and the rest in a stack tail, on every
  native target; there is no parameter-count limit. (See `./mfb spec
  memory 06_native-calling-convention` for the register/stack layout.)
- **Named args** at call site: `greet("Ada", greeting := "Hi")`. Named arguments bind by parameter name, may be mixed with positional arguments, and are evaluated/lowered in declaration order after omitted default parameters are filled. For a builtin whose overloads place the same name at different positions — `tcp::connect`'s `timeoutMs` is the second parameter of the `Address` forms and the third of the `host`/`port` forms — the overload is selected first (the one naming every supplied name, leaving no name on a slot a positional argument already filled, and filled by the supplied arguments as a contiguous prefix — an overload taking exactly this many arguments wins, else the shortest longer one whose unsupplied parameters all trail the supplied ones, which are then filled from their defaults), and names bind within it. So `tls::connect("h", 443, timeoutMs := 5000, serverName := "h")` leaves its trailing `allowSelfSigned` to its default. A call may not skip a parameter before a later named one. Selection looks only at names and positions, so such a call can still select another overload whose contiguous prefix the arguments fill: `tls::connect("h", 443, serverName := "h")` (no `timeoutMs`) fills the `Address` form, whose type check then rejects it with `TYPE_CALL_ARGUMENT_MISMATCH` listing every signature. When no overload's prefix fits, or the builtin has a single layout, the checker reports `TYPE_CALL_ARITY_MISMATCH` — "omits parameter `p` before a later supplied argument" — and then type-checks the arguments it did bind, in parameter order, which can add a `TYPE_CALL_ARGUMENT_MISMATCH` for the same call (`compress::zlibDecode(data, ignoreChecksum := TRUE)` reports both). A named argument therefore always binds to the parameter it names, or the call is rejected. [[src/codegen/builtins/mod.rs:select_param_name_overload]] [[src/ir/shape.rs:check_builtin_call]]
- **Overloading.** Several `FUNC`s may share one name as long as their signatures differ. A callable's identity is its name together with its **ordered parameter types and return type**; two declarations collide (`SYMBOL_DUPLICATE_TOP_LEVEL`) only when *all three* match (default values never distinguish an overload). A name sharing parameter types but differing in return type is a legal return-type overload set. Overloads usually differ by **arity** or **parameter type** and are selected from the argument types. They may also differ **only by return type** (identical parameter lists): such a *return-type overload set* is selected by the call's **expected (contextual) type** (the declared type of the assignment/`LET`/`MUT` target, the parameter type of an argument slot, the enclosing function's return type for a `RETURN` operand, or the element/field type of a typed initializer). When two or more return-type overloads remain and no expected type uniquely selects one, the call is a `TYPE_OVERLOAD_AMBIGUOUS` error; the fix is a type annotation (e.g. `LET b AS List OF Byte = utf8Encode(s)`). Overloads may be declared across the files of one package, and an `EXPORT`ed overload set is resolved across the package boundary by importers. Visibility is not part of a callable's identity, so an `EXPORT` and a `PUBLIC` or `PRIVATE` declaration may share a name; an importer sees only the `EXPORT` members, and a call it makes is resolved against those alone. (For how expected types are propagated, see `./mfb spec architecture type-inference`.)

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

- **User-defined overload resolution** is by the call's **argument count and positional argument types**: a call binds to the one overload whose parameter count equals the number of supplied arguments and whose declared parameter types match the argument types position by position. Resolution is **exact** — it does not rank or prefer among candidates and does not coerce argument types to find a match; if no overload matches exactly, the call is a resolution error. **Default arguments do not combine with overloading**: within an overload set every parameter — including one declared with a default — must be supplied explicitly; trailing defaults are filled only for a name with a **single** declaration. (A user-defined overload set therefore requires explicit arguments for every parameter. Registry-defined built-ins use the separate rule below.) The resolution algorithm and overload-symbol mangling run in the monomorphizer (see `./mfb spec architecture monomorphization`); return-type-overload disambiguation by expected (contextual) type is described in `./mfb spec architecture type-inference`.
- **Registry-defined built-in overloads are an exception to the user rule.**
  Their named-argument layout selection, described above, can select a longer
  signature and fill trailing defaults. Layout selection uses names and positions;
  type compatibility is checked against the built-in registry afterward. It is
  not user overload selection with implicit coercion added. Contextual argument
  types and generic built-in resolution are specified in `./mfb spec architecture
  type-inference`. [[src/codegen/builtins/mod.rs:select_param_name_overload]]
  [[src/ir/shape.rs:check_builtin_call]]
- **Literal types during user overload selection.** Scalar literals already have
  types: strings are `String`, booleans `Boolean`, scalar literals `Scalar`, and
  numbers are classified as `Integer`, `Float`, `Fixed`, or `Money` from their
  spelling, including suffixes. Exact matching compares those types; it does not
  try the later literal-to-parameter coercions to make a candidate fit. For
  example, an integer literal does not become `Byte` merely because an overload
  takes `Byte`. Empty collections and imported overload matching have their
  own unknown-element/ambiguity rules in `./mfb spec architecture monomorphization`.
  [[src/monomorph/lower.rs:expression_type]]
  [[src/monomorph/helpers.rs:params_match]]
- **Native `LINK` calls.** A qualified `alias::member` resolves to the declared
  native member signature; LINK blocks are preserved rather than processed as
  user `FUNC` overload sets. Their defaults are lowered for that declared
  signature. Ordinary MFBASIC wrappers around native calls still follow the user
  overload rules. [[src/resolver/mod.rs:link_target_signature]]
  [[src/monomorph/lower.rs:into_project]]
  [[src/ir/lower.rs:lower_link_defaults]]
- **Calls through function values.** The value has an already selected callable
  type; calling it does not resolve a declaration overload set or recover the
  original declaration's defaults. All parameters of the callable type must be
  supplied. [[src/ir/shape.rs:check_call_shape]]
- **Parameter passing**: arguments are passed as owned values under the memory model (§14). Copyable values are copied when they remain needed by the caller; movable values are moved when ownership can be transferred. Containers own their contents, so passing a container never passes an aliasable reference.
- **Resource parameters**: a parameter whose type is a `RESOURCE` is handled by compiler-known resource rules (§15). Ordinary resource operations receive a pointer to the one handle and do not move ownership; close operations consume it. MFBASIC source does not add ownership-annotation parameter keywords such as `MOVE`.
- **Collection boundaries freeze mutable buffers.** When a `MUT` collection is passed to a function or returned from a function, it crosses the boundary as an immutable, owned collection value (§14). The compiler may move or freeze the existing buffer when ownership permits; the semantic guarantee is that no caller and callee can secretly share a mutable collection.
- **Isolated functions**: any top-level `FUNC` may be marked `ISOLATED` to declare that it can run as a thread entry point. `ISOLATED` is independent of visibility — `PRIVATE`, `PUBLIC` (the default) and `EXPORT` are all valid. `ISOLATED` is invalid on lambdas, closures, and local functions; a non-`FUNC` declaration marked `ISOLATED` is rejected ("ISOLATED function … must be a top-level FUNC declaration"). [[src/ir/verify/mod.rs]]
- **First-class functions & lambdas**:

```basic
LET square = LAMBDA(n AS Integer) -> n * n
FUNC applyTwice(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer
  RETURN f(f(x))
END FUNC
```

- **Closures** capture copyable `LET` bindings by value. Capturing `MUT` is a **compile error** (`TYPE_LAMBDA_CAPTURE_UNSUPPORTED`) because an ordinary closure would observe a frozen copy, never the live cell. Capturing resource handles or other non-copyable values is also `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`. (This is distinct from inner-block reassignment of an outer `MUT`, which is allowed because the scope is still live.) The one exception the compiler allows is a lambda passed directly into a **compiler-proven non-escaping callback position** — the `collections::forEach` action argument: such a lambda may capture an outer `MUT` by reference for the duration of the synchronous call. [[src/codegen/builtins/mod.rs:is_nonescaping_callback_arg]] It still may **not** capture a resource even there. This is an internal call-bound by-reference capture, not a general source-level capability.
- There is no source `NONESCAPING` annotation. The direct `collections::forEach` callback exception above is selected by the compiler. [[src/codegen/builtins/mod.rs:is_nonescaping_callback_arg]]
- **Effects are inferred, not annotated, in v1.** The compiler records fallible calls, resource use, thread use, filesystem/network/native access, and package permissions as audit metadata (see `./mfb spec tooling auditability`). There are no source-level effect or purity annotations.
- **Recursion** is allowed. Implementations are not required to perform tail-call optimization. Native calls consume the host thread stack. The compiler does not currently turn stack exhaustion into a language error; sufficiently deep recursion can terminate the process with a host fault, bypassing `TRAP` and lexical cleanup. [[src/codegen/engine/function/function_lowering.rs:lower_function]]

## See Also

* ./mfb spec architecture monomorphization — the authoritative overload-resolution algorithm (exact arity/positional matching) and `$`-mangled overload symbol naming
* ./mfb spec architecture type-inference — expected (contextual) type propagation and return-type-overload disambiguation
* ./mfb spec language resource-management — resource-parameter ownership and lexical drop
* ./mfb spec architecture escape-analysis — closure capture and non-escaping callback rules
