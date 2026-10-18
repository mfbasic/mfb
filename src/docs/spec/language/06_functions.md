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

- **Every function may fail.** `FUNC F(...) AS T` yields a `T` on success and an `Error` on failure. A function declared `AS Nothing` yields `NOTHING` on success and may still fail. `RETURN` is always the success exit and `FAIL` the failure one; a function declared `AS Error` returns an `Error` as data (`mfb spec language error-model` §8.5).
- **Effect-only functions** are declared `AS Nothing`. Such a body may end with a bare `RETURN` (or `RETURN NOTHING`), or fall through to `END FUNC`; each succeeds with `NOTHING` (`mfb spec language error-model` §8.6 rules 8–9). A `RETURN` operand must be a `Nothing` value like any other success type: `RETURN log("x")` is valid, while `RETURN 5` or `RETURN error(...)` is `TYPE_RETURN_MISMATCH`, never a value silently dropped. [[src/ir/verify/compat.rs:check_return_type]] Its call is usually a statement, but it is an ordinary value expression too: `LET v = log("x")` binds `NOTHING`, which binds, passes, compares and collects like any other value (§4.6). A call to a built-in declared `AS Nothing` (`io::print`, `fs::writeAll`) is the same. [[src/ir/lower.rs:function_return_type]]
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
- **Omitted trailing builtin parameters** are filled with the defaults of the overload the call resolves to, which every supplied argument's type decides, not the first alone: `crypto::open(cipher, key, nonce, sealed, aad)` is the `Sealed` form with its `aad` supplied, and `crypto::open(cipher, key, nonce, ciphertext, tag)` is the ciphertext/tag form with its `aad` defaulted to empty. A call no overload accepts is filled with nothing and rejected as written. [[src/codegen/registry/mod.rs:default_argument_padding]]
- **Overloading.** Several `FUNC`s may share one name as long as their signatures differ. A callable's identity is its name together with its **ordered parameter types and return type**; two declarations collide (`SYMBOL_DUPLICATE_TOP_LEVEL`) only when *all three* match (default values never distinguish an overload). A name sharing parameter types but differing in return type is a legal return-type overload set. Overloads usually differ by **arity** or **parameter type** and are selected from the argument types. They may also differ **only by return type** (identical parameter lists): such a *return-type overload set* is selected by the call's **expected (contextual) type** (the declared type of the assignment/`LET`/`MUT` target, the parameter type of an argument slot, the enclosing function's return type for a `RETURN` operand, or the element/field type of a typed initializer). When two or more return-type overloads remain and no expected type uniquely selects one, the call is a `TYPE_OVERLOAD_AMBIGUOUS` error; the fix is a type annotation (e.g. `LET b AS List OF Byte = utf8Encode(s)`). Overloads may be declared across the files of one package, and an `EXPORT`ed overload set is resolved across the package boundary by importers. A template `FUNC` (`FUNC f OF T(…)`) overloads by signature exactly like a plain one: its identity is its name, ordered parameter types and return type, compared with its type parameters taken by position, so `FUNC f OF T(x AS T)` and `FUNC f OF U(y AS U)` are one signature declared twice (`SYMBOL_DUPLICATE_TOP_LEVEL`). Templates of one name with different parameter shapes, and a template beside a plain `FUNC` of the name, are distinct overloads selected by the call's argument types: a plain overload whose parameter types match the call exactly wins over a template instantiation; otherwise, of the templates whose parameters bind the arguments, the most specific wins (`f OF T(x AS List OF T)` over `f OF T(x AS T)` for `f([1])`), and no single most specific one is `TYPE_OVERLOAD_AMBIGUOUS` (`./mfb spec architecture monomorphization`). [[src/monomorph/lower.rs:instantiate_function]] [[src/resolver/mod.rs:insert_function]] Visibility is not part of a callable's identity, so an `EXPORT` and a `PUBLIC` or `PRIVATE` declaration may share a name; an importer sees only the `EXPORT` members, and a call it makes is resolved against those alone. A `PRIVATE` member is file-local: only a call in its own file can select it, and there it is one overload set with that file's own `PUBLIC`/`EXPORT` declarations of the name, which the other files and importers still call under that name (`./mfb spec language modules-and-packages`). (For how expected types are propagated, see `./mfb spec architecture type-inference`.)

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
  type; the selection, named-argument and default-argument rules are collected
  under "Function-value selection and calls" below.
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

- **Closures** capture copyable `LET` bindings by value. Capturing `MUT` is a **compile error** (`TYPE_LAMBDA_CAPTURE_UNSUPPORTED`) because an ordinary closure would observe a frozen copy, never the live cell. Capturing a value that reaches a `ThreadWorker` view — the view itself, or a record whose `RES` field holds it — is also `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`, because no `List` may hold one either (§14.4). (This is distinct from inner-block reassignment of an outer `MUT`, which is allowed because the scope is still live.) A resource **is** capturable: the closure holds a reference to it, exactly as a `List` element does, and the resource's cleanup scope follows the `List` rule (§14.4, §15.6). The one exception the compiler allows for `MUT` is a lambda passed directly into a **compiler-proven non-escaping callback position** — the `collections::forEach` action argument: such a lambda may capture an outer `MUT` by reference for the duration of the synchronous call. [[src/codegen/builtins/mod.rs:is_nonescaping_callback_arg]] This is an internal call-bound by-reference capture, not a general source-level capability.
- There is no source `NONESCAPING` annotation. The direct `collections::forEach` callback exception above is selected by the compiler. [[src/codegen/builtins/mod.rs:is_nonescaping_callback_arg]]
- **Effects are inferred, not annotated, in v1.** The compiler records fallible calls, resource use, thread use, filesystem/network/native access, and package permissions as audit metadata (see `./mfb spec tooling auditability`). There are no source-level effect or purity annotations.
- **Recursion** is allowed. Implementations are not required to perform tail-call optimization. Native calls consume the host thread stack. A call that would run past the thread's stack raises `ErrStackOverflow` (7-701-0002) at the entry of the function being called, before it binds anything: an ordinary failure that the caller's `TRAP` catches and that runs every unwound frame's cleanup — never a host fault. This holds on every thread, including workers and the app-mode program thread, for direct, mutual and function-value recursion. A function whose body makes no call (no `Call`/`CallResult`) skips the check: it cannot recurse, and its frame fits the reserve kept below the limit. Recursion inside native `LINK` code is outside the check. [[src/codegen/runtime/stack_limit.rs:function_needs_stack_check]] [[src/codegen/runtime/stack_limit.rs:emit_stack_check]]

  The limit is a per-thread word (`./mfb spec memory arenas`) set once when the thread starts, to its stack bottom plus a reserve of `min(512 KiB, stack / 4)` — the reserve covers the largest frame a function allocates before its check, and the raise. macOS (`pthread_get_stackaddr_np` / `pthread_get_stacksize_np`) and Windows (`GetCurrentThreadStackLimits`) report each thread's exact stack. On Linux a runtime-created worker has its known 8 MiB stack; the main thread reads `RLIMIT_STACK` through the raw `prlimit64` syscall and counts on the three quarters of it the kernel keeps below the entry stack pointer (arguments and environment take at most a quarter), capping an unlimited stack at 256 MiB; the app-mode thread asks `pthread_getattr_np`. [[src/codegen/runtime/stack_limit.rs:emit_limit_from_rlimit]]

## Function-value selection and calls

A callable type records ordered parameter types, a result type, and whether it
is isolated. Parameter names and default expressions are not part of that type.
A reference to a single ordinary function uses its declared callable type.
[[src/types.rs:ParameterType]] [[src/ir/lower.rs:function_types]]
A resource parameter's `STATE T` clause and a resource return's `STATE T` clause
are part of that type (`./mfb spec language resource-management`, §15.5).

**Overloaded functions as values.** Naming an overloaded user function as a
value selects the one declaration whose callable type — ordered parameter types,
return type and isolation — is the type the context expects. The expected type
comes from the value's position: the `AS` annotation of a `LET`/`MUT` (local or
top-level), the target of an assignment, the parameter of a single-declaration
function the value is passed to, the enclosing function's return type for a
`RETURN` operand, or the element, field, map-value or `WITH`-field type of a typed
initializer. Selection is exact: a parameter or return type that differs selects
nothing, and an `ISOLATED` member may stand where a plain `FUNC` is expected but a
plain member never satisfies an `ISOLATED FUNC` type. This applies to
parameter-type and return-type overload sets alike, and to an imported package's
exported overload set (`pkg::name`). Calling the selected value runs that
declaration. [[src/monomorph/lower.rs:resolve_function_reference]]

**Isolation widens in one direction, everywhere a callable type is matched.**
An `ISOLATED FUNC` is a `FUNC`; a `FUNC` is not an `ISOLATED FUNC`. Isolation is
an extra guarantee (no captured environment, safe to run as a thread entry
point), and dropping that guarantee is always safe, so a plain callable-type
position accepts either an isolated or a plain concrete, while an
`ISOLATED FUNC`-typed position (`thread::start`'s worker parameter) accepts only
an isolated one. This one rule governs every position a callable type is
matched against another: a `LET`/`MUT`/assignment/return/field/argument
(`TypeEnv::assignable`), overload selection above, and a built-in's own callback
parameter (`collections::forEach`/`transform`/`filter`/`reduce`/`sortBy`/…,
`json::parse`'s reviver) — the registry's structural unifier applies the same
directional check to a `FUNC`/`FUNC` pair. [[src/ir/verify/compat.rs:assignable]]
[[src/codegen/registry/mod.rs:unify_at]]

```basic
' Given overloads of pick for Integer and String:
LET f AS FUNC(Integer) AS Integer = pick     ' the Integer declaration
LET g AS FUNC(String) AS Integer = pick      ' the String declaration
```

A reference with no expected type (`LET f = pick`, or `pick` passed to an
overloaded, package or built-in callee) is `TYPE_OVERLOAD_AMBIGUOUS`; an expected
type that no declaration has, including a non-callable one, is
`TYPE_OVERLOAD_NO_MATCH`. Both name the declared overloads. A local binding with
the same name shadows the function: naming or calling it uses the local. A user
override of an overridable built-in name (`FUNC toString(p AS Point) AS String`)
is selected the same way, but only for a type the built-in itself rejects —
the rule a call to it follows; otherwise the name is the built-in.

A general built-in predicate named as a value (`isEven` where a
`FUNC(Integer) AS Boolean` is expected) is selected by a separate mechanism in IR
lowering, not by this rule. [[src/ir/lower.rs:builtin_predicate_ref_type]]

A call through a function value supplies every parameter **positionally**.
Declaration defaults are not retained, and missing arguments are
`TYPE_CALL_ARITY_MISMATCH`. Named arguments are `TYPE_CALL_ARGUMENT_MISMATCH`
because a callable type preserves no parameter names. Calling the value does
not perform another overload selection.
[[src/ir/shape.rs:check_call_shape]]

A declared native `LINK` member can be used as an MFBASIC function value and
passed to an MFBASIC callback parameter. Its callable type is the declared
MFBASIC wrapper signature; invocation routes through the generated marshaling
thunk, rather than treating a C address as a closure. This rule does not provide
an MFBASIC-to-C callback-pointer conversion. Native ABI declarations are
specified by `./mfb spec language native-libraries`.
[[src/ir/lower.rs:function_types]]
[[src/target/shared/nir/lower.rs:link_routing_imports]]

## See Also

* ./mfb spec architecture monomorphization — the authoritative overload-resolution algorithm (exact arity/positional matching) and `$`-mangled overload symbol naming
* ./mfb spec architecture type-inference — expected (contextual) type propagation and return-type-overload disambiguation
* ./mfb spec language resource-management — resource-parameter ownership and lexical drop
* ./mfb spec architecture escape-analysis — closure capture and non-escaping callback rules
