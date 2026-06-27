# 5. Bindings & Scope

Three binding forms on two axes — `LET`/`MUT` choose **mutability**, `RES`
chooses **ownership**:

- **`LET`** — immutable binding (copyable data).
- **`MUT`** — reassignable binding (copyable data).
- **`RES`** — a uniquely-owned resource (a `File`, `Socket`, `Listener`, …).
  A resource has no aliases, so mutability is moot and `RES` needs no
  immutable/mutable sub-distinction. See §15.

```basic
LET x = 10
MUT total AS Float = 0.0
total = total + 1         ' OK
' x = 5                   ' ERROR: x is immutable
RES f AS File = fs::open("app.db", "read")   ' a resource is bound with RES
```

The binding keyword is **required and enforced**; it *surfaces* a type property,
it does not choose it:

- A resource **must** be bound with `RES`; `LET`/`MUT` on a resource is an error
  (`TYPE_RESOURCE_REQUIRES_RES`).
- `RES` binds **only** resources; `RES` on copyable data is an error
  (`TYPE_RES_REQUIRES_RESOURCE`).
- A resource appears only in `RES` positions — binding, parameter (`RES f AS
  File`), and return (`AS RES File`) — and **never inside a data type**: a record
  field of a resource type is an error (`TYPE_RESOURCE_FIELD_FORBIDDEN`).
- A `RES` binding may carry a copyable, defaultable data `STATE` (§15):
  `RES f AS File STATE FileState = …`.

Rules:

- **No implicit declaration.** Using an undeclared name is a compile error
  (`SYMBOL_UNKNOWN_IDENTIFIER`).
- **Initialization.** A binding must have a type annotation or an initializer;
  neither is `TYPE_BINDING_REQUIRES_TYPE_OR_VALUE`. An immutable `LET` **must**
  have an initializer — `LET x AS T` with no value is `TYPE_LET_REQUIRES_VALUE`.
  A `MUT` *may* omit its initializer, but only when its declared type has a
  defined default value: `MUT x AS T` (no value) starts at `T`'s default, and a
  non-defaultable `T` is `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` (typecheck
  `is_defaultable_type`). `RES` resources are not defaultable and must be bound
  to a producing expression.
- **Lexical, hierarchical scope.** Inner blocks may read and use bindings from
  enclosing scopes, but may **not** re-declare (shadow) a name already in scope —
  that is a `SYMBOL_DUPLICATE_LOCAL` error (see the resolver scope model below).
- **Outer `MUT` reassignment.** An inner block may reassign an enclosing `MUT` (same live scope, same cell).
- **Collection representation follows the binding.** A collection bound with `LET` is an immutable, fixed snapshot. A collection bound with `MUT` is a locally mutable, growable buffer while it remains in that live binding. Binding a `MUT` collection to `LET`, such as `LET snap = pts`, creates an immutable snapshot; if `pts` is used afterward the snapshot is an independent copy, and if `pts` is not used afterward the compiler may freeze and move the buffer.
- **Bindings die at `END`/scope exit.**
- **Compile-time constants.** A `LET` bound to a constant expression *is* a constant expression (usable where one is required). There is no separate `CONST`.
- **Module-level state.** A top-level `MUT` is module state. There is no `GLOBAL` keyword; visibility (§13) governs sharing, and top-level `MUT` is discouraged.

```basic
LET x = 10
IF cond THEN
  LET y = x + 1           ' OK: inner sees outer x
END IF
' io::print(toString(y))       ' ERROR: y died at END IF

MUT total = 0
FOR i = 1 TO 10
  total = total + i       ' OK: reassigns enclosing MUT
NEXT
```

## Resolver scope model

The resolver tracks every in-scope name for a function in **one flat
per-function map** (`name -> Symbol`). Parameters and locals share that single
map — there is no separate parameter namespace and no nested chain of scope
frames. The map is seeded with the function's parameters and then threaded
through the body as statements are resolved.

What introduces a **local** into the map:

- `LET`/`MUT`/`RES` bindings (the `Let` statement form),
- the `FOR` counter variable,
- the `FOR EACH` element variable,
- a `MATCH` case's union-pattern binding (`MATCH … Type(binding)`),
- an inline-`TRAP` handler binding (`expr TRAP e ... END TRAP`),
- lambda parameters,
- the function-level `TRAP` binding.

<!-- [[src/resolver.rs:resolve_statement]] [[src/resolver.rs:resolve_expression]] -->


Function parameters are inserted first; a parameter whose name collides with an
earlier parameter is `SYMBOL_DUPLICATE_LOCAL`.
[[src/resolver.rs:resolve_function]]

### No shadowing of an in-scope local

Re-declaring a name that is **already live** in the function's flat map is
`SYMBOL_DUPLICATE_LOCAL` — the resolver detects the collision when the insert
returns a previous entry. This applies to a `LET`/`MUT`/`RES` that reuses a
parameter or earlier local name, and to a `FOR`/`FOR EACH` loop variable that
reuses a name already in scope. There is no shadowing: an inner binding may not
re-use a name still visible from an enclosing block.

```basic
LET x = 10
LET x = 20                ' ERROR: SYMBOL_DUPLICATE_LOCAL
FOR x = 1 TO 3            ' ERROR: x is already in scope
NEXT
```
[[src/resolver.rs:resolve_statement]]

### Straight-line locals persist to later siblings

Within one straight-line block the locals map is mutated in place, so a `LET`
declared earlier is visible to every later sibling statement in the same block:

```basic
LET a = 1
LET b = a + 1             ' OK: a is in scope for the rest of this block
```

### Nested blocks clone the local set

The bodies of `IF`/`ELSE`, `WHILE`, and `DO … UNTIL` are resolved as **nested
blocks**: the resolver clones the current locals map, resolves the body against
the clone, and discards it. Bindings introduced inside such a block therefore do
**not** leak out to following sibling statements. (This is why a name declared in
an `IF` body is gone after `END IF`, while still being unable to collide with —
and not shadow — an outer name, since the clone already contains it.)

`FOR` and `FOR EACH` bodies are likewise resolved against a clone that already
holds the loop variable, so the counter/element and any bindings inside the loop
body do not escape the loop. A `MATCH` case body is resolved against a clone that
holds the case's union binding (when present), so per-case bindings stay local to
the case.
[[src/resolver.rs:resolve_nested_block]]

### MATCH guards see the union binding

A `MATCH` case guard is resolved against its own clone of the locals map, and
when the case uses a union pattern the guard clone has the union binding inserted
**before** the guard expression is resolved. The guard can therefore reference
the bound payload, exactly as the case body can.
[[src/resolver.rs:resolve_statement]]

### The TRAP binding sits on top of the body's locals

A function-level `TRAP` handler is resolved against a **clone of the locals map
that existed after the function body was resolved**, with the trap's error
binding inserted on top. The handler thus sees the function's parameters (and any
function-level locals captured into that map) plus its own error binding, while
the trap binding itself is confined to the handler. An inline `expr TRAP e …`
handler works the same way at expression granularity: it clones the locals
visible at that point and adds the handler binding `e`.
[[src/resolver.rs:resolve_function]] [[src/resolver.rs:resolve_expression]]
