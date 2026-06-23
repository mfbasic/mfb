# ⟪MFBASIC⟫ — Language Specification

## Modern Functional Basic (MFB)

A modern, functional dialect of BASIC. Immutable by default, no objects, package-level imports, and an implicit error model. Every function call **produces its value on success and fails with an `Error` on failure**; the value auto-unwraps and the `Error` auto-propagates. Errors auto-route to an inline `TRAP` on the failing expression, to a function-level `TRAP`, or propagate to the caller — no `TRY`, no `GOTO`, no exceptions. The language is designed for memory-safe implementation through owned values, explicit resource ownership, and lexical cleanup.

---

## 1. Design Principles

1. **Readable over terse** — English keywords, `END X` blocks, line-oriented.
2. **Functional, no OOP** — plain data (records/unions) + free functions. No classes, methods, `self`, or inheritance.
3. **Immutable by default** — `LET` binds, `MUT` opts into reassignment. No implicit globals, no hidden aliasing.
4. **Optional ceremony** — a 3-line script needs no module header; structure exists when you want it.
5. **Errors as values, invisibly plumbed** — every function call yields its value or fails with an `Error`; success auto-unwraps, errors auto-route to an inline `TRAP` on the failing expression, to a function-level `TRAP`, or propagate. No exceptions, no unwinding.
6. **Package-owned closed domains** — a package owns the unions it defines and the free functions that operate on them. Extension is package layering through explicit composition (`UNION ... INCLUDES ...`), not open inheritance, traits, or retroactive interface implementation.
7. **Predictable memory** — designed for memory-safe implementation through formal ownership, move, copy, freeze, resource, and lexical drop rules. No GC, no refcounting, no manual `free`.

---

## 2. Lexical Structure

- **Case-insensitive keywords**, case-sensitive identifiers. Convention: `camelCase` functions and built-in callable names, `CapitalCamelCase` types, `camelCase` bindings, `UPPERCASE` keywords.
- **Comments**: `'` to end of line, or `REM`.
- **Statement separator**: newline, or `:` for multiple statements on one line.
- **Line continuation**: trailing `_`.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`. Legacy sigils (`$ % # !`) are removed.
- Identifiers are ASCII-only in this version. If a future version allows non-ASCII identifiers, compilers and language servers must lint Unicode confusables and near-collisions after Unicode normalization and case folding.

```basic
LET total = 0 : LET count = 0     ' two statements, one line
LET msg = "hello " & _
          "world"                 ' continuation
' this is a comment
REM so is this
```

The `:` separator is legal, but formatters and language servers should lint dense security-sensitive lines, especially lines that combine fallible calls, resource operations, native calls, permissioned filesystem/network operations, an inline `TRAP`, or `TRAP` control flow.

Identifiers are case-sensitive, so `userId` and `userid` are distinct. Tooling should lint near-collisions that differ only by case or visually minor spelling differences within the same scope or imported namespace.

---

## 3. Templates

MFBASIC supports monomorphized templates, not runtime generics.

Template parameters may appear only on `TYPE`, `UNION`, `FUNC`, and `SUB` declarations. A template is not a runtime entity and is not emitted to binary representation as an open declaration. Every used instantiation is resolved during compilation into a concrete declaration before IR, binary representation, package metadata, or native lowering is produced.

Built-in type constructors such as `List`, `Map`, and `Thread` are compiler-owned templates. User code may define templates with the same `OF` syntax where allowed:

```basic
TYPE Stack OF T
  items AS List OF T
END TYPE

FUNC push OF T(s AS Stack OF T, value AS T) AS Stack OF T
  RETURN WITH s { items := append(s.items, value) }
END FUNC

SUB printValue OF T(value AS T)
  io::print(toString(value))
END SUB
```

Template arguments are inferred only from explicit argument, parameter, field, and expected result types by simple unification. There is no general inference engine, no trait system, no variance, no higher-kinded types, no boxing, and no runtime template dispatch.

Template predicates such as comparability, copyability, defaultability, or resource restrictions are checked against each concrete instantiation:

```basic
FUNC getOrDefault OF K, V(items AS Map OF K TO V, key AS K, defaultValue AS V) AS V
  IF hasKey(items, key) THEN
    RETURN get(items, key)
  END IF

  RETURN defaultValue
END FUNC
```

The `K` parameter above must be comparable because every concrete `Map` key type must be comparable. The requirement is checked when `getOrDefault` is instantiated, not through a separate bound or trait declaration.

Exported templates in source packages are instantiated by the importing compilation before binary representation is produced. A compiled `.mfp` package contains only concrete template instantiations; it does not expose templates for later instantiation unless a future package format explicitly adds signed template metadata.

---

## 4. Types

### 4.1 Primitives

| Type | Description |
|------|-------------|
| `Integer` | 64-bit signed |
| `Float` | 64-bit IEEE float |
| `Fixed` | 64-bit binary fixed-point, signed 32/32 split |
| `Boolean` | `TRUE` / `FALSE` |
| `String` | UTF-8, immutable |
| `Byte` | unsigned 8-bit |

`Fixed` is a binary fixed-point number stored as a signed 32-bit integer part and a 32-bit fractional part. Its range is approximately `-2147483648.0` through `2147483647.9999999998`, with a resolution of `1 / 2^32`. Fixed-point arithmetic is deterministic across targets, but it is not exact decimal currency arithmetic because most decimal fractions are rounded to binary fixed-point values. Overflow produces an error result with code `77050010`; divide-by-zero and invalid numeric domains produce an error result with code `77050002`.

The name `Fixed` is retained for deterministic binary fixed-point arithmetic. A future exact base-10 financial type, if added, should use a distinct name such as `Decimal` and must specify decimal scale, rounding, and overflow rules separately.

Numeric literals are initially untyped. Integer-looking literals default to `Integer` when there is no expected type. Decimal-looking literals default to `Float` when there is no expected type. When the expected type is `Fixed`, a decimal literal is rounded to the nearest representable `Fixed` value. There is no separate suffix syntax for `Fixed`; use an explicit annotation or conversion when needed:

```basic
LET x = 1.25             ' inferred Float
LET y AS Fixed = 1.25    ' Fixed
LET z = toFixed("1.25")  ' Fixed, fallible parse
```

`Byte` is an unsigned 8-bit integer with range `0` through `255`. Integer literals may initialize a `Byte` only when the literal is statically in range. Runtime conversion to `Byte` uses `toByte`; out-of-range conversion fails with `77050010`.

Fixed > Float > Integer > Byte

| Left operand | Right operand | `+`, `-`, `*`, `^`, `/`, `MOD` | `DIV`   |
|--------------|---------------|--------------------------------|---------|
| `Byte`       | `Byte`        | `Byte`                         | `Float` |
| `Byte`       | `Integer`     | `Integer`                      | `Float` |
| `Byte`       | `Fixed`       | `Fixed`                        | `Float` |
| `Byte`       | `Float`       | `Float`                        | `Float` |
| `Integer`    | `Byte`        | `Integer`                      | `Float` |
| `Integer`    | `Integer`     | `Integer`                      | `Float` |
| `Integer`    | `Fixed`       | `Fixed`                        | `Float` |
| `Integer`    | `Float`       | `Float`                        | `Float` |
| `Fixed`      | `Byte`        | `Fixed`                        | `Float` |
| `Fixed`      | `Integer`     | `Fixed`                        | `Float` |
| `Fixed`      | `Fixed`       | `Fixed`                        | `Float` |
| `Fixed`      | `Float`       | `Fixed`                        | `Float` |
| `Float`      | `Byte`        | `Float`                        | `Float` |
| `Float`      | `Integer`     | `Float`                        | `Float` |
| `Float`      | `Fixed`       | `Fixed`                        | `Float` |
| `Float`      | `Float`       | `Float`                        | `Float` |

Numeric comparisons (`=`, `<>`, `<`, `>`, `<=`, `>=`) use the same operand promotion rules for comparison but always return `Boolean`. `=` and `<>` also accept any two compatible comparable operands.

Numeric edge cases:

- `Integer` arithmetic is checked. Overflow in `+`, `-`, `*`, unary `-`, exponentiation (`^`), and the minimum-integer `MOD -1` case fails with `ErrOverflow` (`77050010`). Integer operations never wrap.
- `Byte` arithmetic that returns `Byte` is checked. Results above `255` fail with `ErrOverflow` (`77050010`); results below `0` fail with `ErrUnderflow` (`77050011`). Byte operations never wrap.
- `/` uses the promoted result type from the table above. When `/` promotes to `Byte` or `Integer`, it truncates the quotient toward zero. `DIV` is fractional division and always returns `Float`. Division by zero fails with `ErrFloatDomain` (`77050012`) for `Float` results and `ErrInvalidArgument` (`77050002`) otherwise.
- `MOD` uses the promoted result type from the table above and is available for every numeric operand pairing in the table. `a MOD b` fails when `b = 0`, with `ErrFloatDomain` (`77050012`) for `Float` results and `ErrInvalidArgument` (`77050002`) otherwise. Otherwise the remainder has the same sign as `a`, and `a = (truncTowardZero(a / b) * b) + (a MOD b)` in the promoted numeric domain.
- `^` for `Integer` requires a non-negative integer exponent and fails with `ErrInvalidArgument` (`77050002`) for negative exponents. Overflow fails with `ErrOverflow` (`77050010`).
- `Float` follows IEEE 754 binary64 representation, but MFBASIC does not expose successful non-finite arithmetic results. Floating-point domain failures, including division by zero and invalid `^` exponents, fail with `ErrFloatDomain` (`77050012`). Results that would be NaN fail with `ErrFloatNaN` (`77050013`). Results that would be infinity fail with `ErrFloatInf` (`77050014`), except arithmetic overflow to infinity fails with `ErrFloatOverflow` (`77050015`). Imported native `Float` values that are already NaN or infinity are rejected at the boundary with `ErrInvalidFormat` (`77050003`).
- Float comparisons are total over finite values only. Comparing a non-finite `Float` is not possible in ordinary MFBASIC source because non-finite values cannot be constructed or imported successfully.
- Converting `Float` or `Fixed` to `Integer` or `Byte` fails with `ErrOverflow` (`77050010`) when outside the destination range. Converting text to a numeric type fails with `ErrInvalidFormat` (`77050003`) when the text is malformed or names a non-finite value such as `NaN` or `Infinity`.

### 4.2 Records (product types)

Pure data, no attached behavior.

```basic
TYPE Vec3
  x AS Float
  y AS Float
  z AS Float
END TYPE

LET v  = Vec3[1.0, 2.0, 3.0]                 ' positional
LET w  = Vec3[x := 0.0, y := 1.0, z := 0.0]  ' by field
LET v2 = WITH v { x := 99.0 }                ' functional update, v unchanged
io::print(toString(v.x))                      ' field read
```

Field access uses `.`: `value.fieldName`. It is compile-time checked, and the right side is a field identifier, not a variable or string.

Record fields may have visibility. A public field can be read, constructed, and updated anywhere the record type is visible. A `PACKAGE` field can be used only by files in the declaring package. A `PRIVATE` field can be used only in the declaring source file. Outside code that cannot see a field also cannot set it in a constructor, read it with `.`, or update it with `WITH`; such records are opaque across that boundary and must be constructed or modified through exported package functions.

Constructors use square brackets: `TypeName[...]`. Brackets are never used for indexing, so constructor syntax does not conflict with collection access.

List literals use bare square brackets, such as `[1, 2, 3]`, and are parsed separately from constructors. A constructor always begins with a visible type name before the opening bracket.

`WITH value { field := expr, ... }` creates a copy of a record with the named fields replaced. The original value is unchanged.

A record type may not contain itself, directly or transitively, except through a `List`, `Map`, or `UNION`. A field is a mandatory owned value with no null or absent form, so a record whose field cycles back to the same record only through other plain records has no base case and can never be constructed. Such a declaration is rejected with `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`:

```basic
TYPE Node          ' rejected: `next` always demands another Node
  value AS Integer
  next  AS Node
END TYPE
```

Mutually recursive records with no intervening `List`, `Map`, or `UNION` are rejected for the same reason. Recursion is legal when every cycle passes through a `List`, `Map`, or `UNION`, because those supply a terminating base case — an empty collection, or a non-recursive union member:

```basic
TYPE Tree                       ' allowed: cycle passes through List
  value    AS Integer
  children AS List OF Tree
END TYPE
```

See §4.3 for the recursive-union form and §14.5 for the related value-cycle rule.

### 4.3 Unions (sum types)

User-defined unions are closed sums over existing concrete member types. A union declaration does not define payload fields inline; it names concrete `TYPE` declarations that already exist and groups them into one closed domain.

```basic
TYPE Circle
  radius AS Float
END TYPE

TYPE Rect
  w AS Float
  h AS Float
END TYPE

TYPE Point
END TYPE

UNION Shape
  Circle
  Rect
  Point
END UNION

LET s AS Shape = Circle[radius := 2.0]
```

A union may include the members of another concrete union:

```basic
IMPORT shape

TYPE Triangle
  a AS Float
  b AS Float
  c AS Float
END TYPE

UNION ExtraShape INCLUDES shape::Shape
  Triangle
END UNION

LET c AS ExtraShape = Circle[radius := 10.0]   ' member included from shape::Shape
LET t AS ExtraShape = Triangle[a := 3.0, b := 4.0, c := 5.0]
```

`INCLUDES` creates a new closed union whose member set is the included union's members plus the members declared locally. It does not modify the included union, does not create subtyping, and does not make `ExtraShape` accepted where `shape::Shape` is expected. This is package layering, not polymorphism: the new package owns the larger domain and the free functions that operate on it.

A package may extend a closed domain by defining a new union that includes another package's union members, then forwarding or wrapping the included domain's operations through its own package functions:

```basic
FUNC area(s AS ExtraShape) AS Float
  MATCH s
    CASE Triangle(t)
      RETURN triangleArea(t.a, t.b, t.c)

    CASE ELSE
      FAIL error(77050004, "shape member not handled")
  END MATCH
END FUNC
```

Included members are reintroduced as members of the new union for construction and matching. In the declaring package they are addressed like local types; importers address them through the declaring package namespace, such as `extras::Circle[radius := 10.0]`. Member name conflicts are compile-time errors.

Recursive concrete unions are allowed by defining recursive member types:

```basic
TYPE JsonNull
  value AS Nothing
END TYPE

TYPE JsonBool
  value AS Boolean
END TYPE

TYPE JsonNum
  value AS Float
END TYPE

TYPE JsonStr
  value AS String
END TYPE

TYPE JsonArr
  items AS List OF Json
END TYPE

TYPE JsonObj
  fields AS Map OF String TO Json
END TYPE

UNION Json
  JsonNull
  JsonBool
  JsonNum
  JsonStr
  JsonArr
  JsonObj
END UNION
```

A non-template user-defined union introduces one concrete type name, even when it includes another union.

### 4.4 `Error` and absence (built in)

The error model is built on two built-in read-only types, `Error` and
`ErrorLoc`:

```basic
TYPE ErrorLoc
  filename AS String
  line     AS Integer
  char     AS Integer
END TYPE

TYPE Error
  code    AS Integer
  message AS String
  source  AS ErrorLoc
END TYPE
```

Both are compiler/runtime-generated **read-only** record shapes. A program may
read their fields, but may **not** construct them with `[...]`, update them with
`WITH`, or assign to their fields. User-authored errors are created with the
`error` built-in function:

```basic
FUNC error(code AS Integer, message AS String) AS Error
```

`error(...)` is always in scope like `toString`. The returned `Error.source`
records the source location of the `error(...)` call expression. `Error.source`
records where the error *originated*; it is not rewritten as the error
propagates.

The mental model is simple: **a function call either produces its value or fails
with an `Error`.** On success the value is delivered directly (auto-unwrapped); on
failure the `Error` auto-propagates to the nearest `TRAP` or to the caller (see
§8). Users never write a wrapper type around the result — there is nothing to
name, construct, or match.

- **A function may fail.** `FUNC F(...) AS T` yields a `T` on success and an
  `Error` on failure; a `SUB` yields nothing on success and may still fail.
- Failure always carries the single public `Error` type — no per-function error
  types, no coercion. Users create errors with `error(code, message)`, read
  `e.code` / `e.message` / `e.source`, and bind the error in `TRAP(e)`.
- A runtime-generated error (divide-by-zero, overflow, a failing built-in or
  package helper) carries the source location of the failing expression. A
  propagated error keeps the original origin, not the propagation point. An error
  raised inside an imported package carries the package's own source location.
- There is no built-in `Option`/`Maybe`. Absence is represented by an
  `error(code, message)`; use semantic error-code constants such as
  `errorCode::ErrNotFound` for not found.

> **Implementation note.** Internally the runtime represents every fallible
> outcome as a two-member union (a private success member plus the public
> `Error`). That type is not nameable, constructible, or matchable in user code;
> it exists only in compiler IR and binary representation metadata, and is never observable in
> user syntax.

### 4.5 Enums

```basic
ENUM Color
  Red, Green, Blue
END ENUM
```

Enum members are addressed as `EnumType.Member`, not as package names and not as bare globals:

```basic
LET c = Color.Red

MATCH c
  CASE Color.Red   : io::print("red")
  CASE Color.Green : io::print("green")
  CASE Color.Blue  : io::print("blue")
END MATCH
```

The `.` token is used for both enum member access and record field access. `EnumType.Member` is resolved from a type name on the left; `value.field` is resolved from a value expression on the left.

### 4.6 `Nothing`

`Nothing` is the unit type. It has one value, written `NOTHING`. It is used for
marker union members and for the `FUNC(...) AS Nothing` callback bridge that lets
a `SUB` be passed where a function value is expected. A `SUB` is value-less — it
produces no success value (see §7) — so a `SUB` body does not name `Nothing`.

```basic
SUB log(msg AS String)
  IF msg = "" THEN RETURN       ' bare RETURN: early, value-less exit
  io::print(msg)
END SUB
```

### 4.7 Collections

```basic
List OF T                          ' owned sequence
Map OF K TO V                      ' owned map
MapEntry OF K TO V                 ' map iteration entry
```

`List`, `Map`, and `MapEntry` are built-in templates. Each concrete use, such as `List OF Integer`, `Map OF String TO Float`, or `MapEntry OF String TO Float`, is monomorphized before binary representation generation. There is one sequence type, `List`. There are no fixed-size arrays and no `DIM`. See §12.

`MapEntry OF K TO V` is the compiler-owned record shape used when iterating a map. It has public read-only fields `key AS K` and `value AS V`.

Runtime collection storage is specified in `specifications/memory_layouts.md`.

### 4.8 Threads

```basic
Thread OF Msg TO Out                 ' isolated running or completed thread
ThreadWorker OF Msg TO Out           ' worker-side view of the same thread
Thread OF Msg RES Res TO Out         ' with a resource plane (thread::transfer/accept)
Thread OF RES Res TO Out             ' resource plane only (message slot is Nothing)
```

`Thread` and `ThreadWorker` are built-in templates for opaque handles to the same underlying package worker. `Thread` is the parent-side handle. `ThreadWorker` is the worker-side handle passed into the thread entry function. `Msg` is the message type used by `thread::send` and `thread::receive`; `Out` is the thread entry function's success type. A completed parent `Thread`'s outcome is retrieved only through `thread::waitFor(t)`, which auto-unwraps the `Out` value or auto-propagates the `Error`; retrieving the outcome consumes and closes the parent `Thread` handle.

### 4.9 Type Inference

`LET` and `MUT` infer when initialized; explicit `AS` otherwise required.

```basic
LET name = "world"        ' inferred String
MUT i AS Integer          ' explicit, uninitialized (defaults 0)
```

### 4.10 Default Values

A `MUT` binding may omit its initializer only when its type has a defined default value.

| Type | Default |
|------|---------|
| `Integer`, `Byte` | `0` |
| `Float`, `Fixed` | `0.0` |
| `Boolean` | `FALSE` |
| `String` | `""` |
| `Nothing` | `NOTHING` |
| `List OF T` | `[]`, when `T` has a default value |
| `Map OF K TO V` | Empty map, when `K` and `V` have default values |
| Record type | A record with every field set to its default, if every field type has a default. |

Defaultability is recursive and finite: nested lists, maps, and records are defaultable only when every transitively referenced element, key, value, and field type is also defaultable, and recursive record cycles (legal only through `List`, `Map`, or `UNION`; see §4.2) do not define a default value. Enums, unions, functions, lambdas, threads, and resource handles do not have default values. A `MUT` binding of one of those types must have an initializer.

### 4.11 Comparable Types

Some standard functions require a type to be comparable. Comparable types are `Integer`, `Float`, `Fixed`, `Boolean`, `String`, `Byte`, `Nothing`, enum types, and records whose fields are all comparable. `List`, `Map`, unions, functions, lambdas, threads, and resource handles are not comparable.

`Map` keys must be comparable. List helpers such as `find`, `contains`, and `replace` require comparable element types.

Equality operators `=` and `<>` require either numeric operands or any two compatible comparable operands. Ordering operators `<`, `>`, `<=`, and `>=` remain numeric-only.

---

## 5. Bindings & Scope

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

- **No implicit declaration.** Using an undeclared name is a compile error.
- **Lexical, hierarchical scope.** Inner blocks may read and shadow bindings from enclosing scopes.
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

---

## 6. Functions

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

---

## 7. Subs

A `SUB` is **effect-only and value-less**: it produces no success value, and its
call is a statement, not an expression.

```basic
SUB logItem(x AS Integer)
  io::print(toString(x))
END SUB
```

A `SUB` still has an error channel — it can `FAIL`, auto-propagate, and drop
resources on the way out — but it produces nothing on success. `EXIT SUB` is the
value-less early success exit, and fall-through to `END SUB` succeeds. `RETURN`
and `RETURN NOTHING` are compile errors in a `SUB`; `RETURN` is for value-producing
`FUNC` bodies. A `SUB` call may not be used in value position: `LET x = aSub()`
is a compile error.

For first-class function typing, a `SUB(A, B, ...)` is compatible with `FUNC(A, B, ...) AS Nothing`. This lets effect-only callbacks work without wrapper functions:

```basic
SUB printItem(x AS Integer)
  io::print(toString(x))
END SUB

forEach(nums, printItem)
```

`Nothing` remains a normal concrete unit type — it is still needed for marker
union members and for the `FUNC(...) AS Nothing` callback bridge above — but a
`SUB` body never names it. A value-less call (a `SUB`, or a fallible effect-only
built-in such as `fs::writeAll`) participates in auto-propagation and inline
`TRAP` handling like any other call; its inline `TRAP` `RECOVER` takes no operand:

```basic
fs::writeAll(f, "done") TRAP(e)
  io::print(e.message)
  RECOVER            ' value-less: the call produces no value
END TRAP
io::print("saved")
```

Value-producing callbacks still require a value-producing `FUNC`. A `SUB` is valid for APIs such as `forEach` that expect `FUNC(T) AS Nothing`; it is not valid for APIs such as `transform` that infer and collect a result value.

---

## 8. Error Model — Implicit Failure, One `TRAP` per Function

### 8.1 The core rule

Every function call either **produces its value** or **fails with an `Error`**. On success the value is delivered directly (auto-unwrapped). On failure, control immediately transfers to the enclosing `TRAP`; if there is no `TRAP`, the function fails with that same `Error` to *its* caller.

```basic
LET x = toFloat(input)    ' on success: x = v
                            ' on failure: jump to TRAP, or fail to the caller carrying e
```

There is **no `TRY` keyword and no `GOTO`**. Propagation is the default behavior of calling a function; a call auto-propagates **unless** a postfix inline `TRAP` is attached to its expression (§8.4), which overrides the default for that one expression.

Function arguments are evaluated left to right. If any argument expression fails, later arguments are not evaluated and the error routes to the enclosing `TRAP` or propagates to the caller.

When an error path leaves a scope, any live resource bindings in that scope are closed by lexical drop (§14.7, §15) before the final error reaches the enclosing `TRAP` or caller.

### 8.2 Entering the error path

Use `FAIL` to fail explicitly with an `Error`:

```basic
IF n < 0 THEN FAIL error(77050002, "negative")
```

`FAIL e` routes to the enclosing `TRAP`; with no trap, the function fails to its caller carrying `e`.

### 8.3 The `TRAP` block — one keyword, two scopes

`TRAP(e)` traps errors in two scopes. A **function-level** `TRAP(e)` at the bottom of a `FUNC`/`SUB` traps every error from the body; an **inline** `TRAP(e)` attached postfix to a single expression traps just that expression (§8.4). Both bind an `Error` named by the parenthesized identifier, so no type annotation is needed.

```basic
FUNC readAge(input AS String) AS Integer
  LET n = toInt(input)                 ' auto-propagates on failure
  IF n < 0 THEN FAIL error(77050002, "negative")
  RETURN n

  TRAP(err)
    io::print("Bad age: " & err.message)
    RETURN 0                           ' function succeeds with default
  END TRAP
END FUNC
```

Each `FUNC`/`SUB` may declare **at most one** function-level `TRAP`, at the bottom, after normal flow.

Trap outcomes:

| Statement | Meaning | Produces | Scope |
|-----------|---------|----------|-------|
| `RECOVER v` | bind `v` and continue after the trap | binding gets `v` | inline only |
| `RETURN v` | function succeeds | success value `v` | `FUNC` only |
| `EXIT SUB` | sub succeeds | no value | `SUB` only |
| `PROPAGATE` | re-propagate the current `err` | failure carrying `err` | both |
| `FAIL e2` | replace/wrap the error | failure carrying `e2` | both |

The function-level `TRAP` is **diverging-only**: it has no `RECOVER`, because at function scope there is no failing statement to resume into. It may convert the error into the function's final success value with `RETURN` (in a `FUNC`) or `EXIT SUB` (in a `SUB`), rethrow the same error with `PROPAGATE`, or replace it with `FAIL`. Once control enters a function-level `TRAP`, the failed expression is abandoned.

```basic
TRAP(err)
  PROPAGATE                            ' bubble the same error
END TRAP
```

```basic
TRAP(err)
  FAIL error(77060001, "load failed: " & err.message)   ' wrap with context
END TRAP
```

### 8.4 Local error handling (inline `TRAP`)

To handle an error at the call site instead of auto-propagating, attach a **postfix inline `TRAP`** to the expression. The happy value auto-unwraps into the binding exactly as a normal call; on error the handler block runs with `e : Error` and must either **`RECOVER` a value** (bound into the binding, then continue at the statement after `END TRAP`) or **diverge** (`RETURN`, `FAIL`, `PROPAGATE`, or an `EXIT` form).

```basic
RES f = fs::openFile(path) TRAP(e)
  io::print("could not open: " & e.message)
  RECOVER fs::openFile(fallbackPath)   ' supply a File and continue
END TRAP
LET line = fs::readLine(f)
```

An inline `TRAP` is legal only as the value of a `LET`/`MUT` binding, an assignment, or a bare expression statement. It scopes to exactly **one** expression — to wrap several fallible calls, use the function-level `TRAP`. Every path through the handler must `RECOVER` or diverge; falling through to `END TRAP` is a compile error (there must be no path that leaves the binding unset). For a value-less trapped call (a `SUB`, or a fallible effect-only built-in), `RECOVER` takes no operand.

Use the same construct for ordinary absence — `RECOVER` the recoverable case, bail on the rest:

```basic
IMPORT errorCode

LET user = getUser(id) TRAP(e)
  IF e.code = errorCode::ErrNotFound THEN RECOVER defaultUser   ' use default, continue
  FAIL e                                                        ' any other error: bail
END TRAP
```

`MATCH` no longer intercepts call errors. A call used as a `MATCH` scrutinee auto-unwraps like every other call site; `MATCH` matches enum/union **values** only (§9).

### 8.5 `RETURN` semantics

`RETURN v` **always** means function success with the value `v`, whether it appears in the body or in the `TRAP`. It does not resume at the failed expression. `RETURN` is forbidden in a `SUB`; use `EXIT SUB` for a value-less early success exit. A `SUB` with no `TRAP` may fall through to `END SUB`, which succeeds. `RETURN` never produces an error. `FAIL` and `PROPAGATE` produce errors.

### 8.6 Rules

1. At most one function-level `TRAP` per function, at the bottom, after normal flow.
2. The trap payload is always `Error`; written `TRAP(err)` with no type. The same spelling is used for the inline and function-level forms.
3. The function-level trap block is reachable only via `FAIL` (in the body), an auto-propagated failure from a call, or `FAIL`/`PROPAGATE` inside the trap. It is never reached by fall-through.
4. `PROPAGATE` is valid inside a function-level `TRAP` or an inline `TRAP` handler (it refers to the current `err`). Elsewhere it is a compile error; use `FAIL e` instead.
5. With no enclosing `TRAP`, any failure (from `FAIL` or an auto-propagated call) becomes the function's failure to its caller.
6. Every function-level `TRAP` path must end in `RETURN` (for a `FUNC`), `EXIT SUB` (for a `SUB`), `PROPAGATE`, or `FAIL`. Trap fall-through is a compile error.
7. Every `FUNC` path must end in `RETURN value` or `FAIL error`. Function fall-through is a compile error.
8. A `SUB` with no `TRAP` may fall through to `END SUB`, which succeeds (value-less).
9. A `SUB` with a `TRAP` must end every normal path before the `TRAP` with `EXIT SUB` or `FAIL error`. Falling through from the normal body into the `TRAP` is a compile error.
10. An executable entry point's uncaught failure terminates the process as an unhandled runtime error: the process exits with code `255`, and stderr receives `Code: <err.code> Message: <err.message>`. Give the entry point a `TRAP` for graceful handling.
11. An inline `TRAP` is legal only as the value of a `LET`/`MUT` binding, an assignment, or a bare expression statement, and traps exactly one expression. The trapped expression must be a fallible call; trapping an expression that cannot fail is a compile error.
12. Every path through an inline `TRAP` handler must end in `RECOVER` or a diverging statement (`RETURN`, `FAIL`, `PROPAGATE`, or an `EXIT` form). Falling through to `END TRAP` is a compile error.
13. `RECOVER` is valid only inside an inline `TRAP` handler; it is a compile error in a function-level `TRAP` or anywhere else. `RECOVER`'s value must be assignable to the trapped expression's success type; it carries a value iff that type is not `Nothing`. The handler binding is scoped to the handler block only.

### 8.7 Program entry point

An executable program starts at the root-package function named by `project.json` `entry`, defaulting to `main`. The entry point may be any one of these source shapes; empty parentheses are optional for zero-argument entries:

```basic
SUB main
END SUB

SUB main(args AS List OF String)
END SUB

FUNC main AS Integer
END FUNC

FUNC main(args AS List OF String) AS Integer
END FUNC
```

The actual name is the manifest entry value, so `main` above is illustrative. The accepted entry signatures are closed: a `SUB` entry has success type `Nothing`, a `FUNC` entry must have success type `Integer`, and the only allowed parameter is one `List OF String` argument. Multiple matching entry declarations, a missing entry declaration in an executable, any other parameter list, or any non-`Integer` `FUNC` entry return type are compile-time errors.

When an entry declares `args AS List OF String`, the runtime passes the command-line argument vector as an owned immutable list. `get(args, 0)` is the program name as invoked by the host. Subsequent elements are user arguments in order.

Process result mapping:

| Entry outcome | Process behavior |
|---------------|------------------|
| `SUB` entry succeeds | Exit code `0`. |
| `FUNC ... AS Integer` succeeds with `n` | Exit code `n`. Implementations must reject or fail values outside the host process exit-code range. |
| `EXIT PROGRAM n` executes | Run stack-wide lexical cleanup, then exit with code `n`. |
| Entry fails with an uncaught error carrying `err` | Write `Code: <err.code> Message: <err.message>` to stderr and exit with code `255`. |

Environment access outside command-line arguments is outside the core language specification and may be provided by a future standard package.

### 8.8 Desugaring

This sketch is **compiler-internal**: it describes how source desugars into
**structured IR** (the same IR that is serialized as the package's Binary Representation).
`Result`, `Ok`, and `Err` below are the runtime's private representation of a
fallible outcome (§4.4), not types a user writes. The control flow is
structured: a function-level `TRAP` is a nested region with an explicit end, not
a label, and "propagate to the enclosing trap" is the structured `PROPAGATE` op,
not a jump to a program counter.

```text
FUNC f(a AS A) AS T            =>   FUNC f(a AS A) AS Result OF T

  call g(x)        =>  MATCH g(x)
                         CASE Ok(v)    : v
                         CASE Error(e) : PROPAGATE to enclosing TRAP region
                                       (no trap => RETURN error result carrying e)
                       END MATCH

  FAIL e           =>  PROPAGATE e to enclosing TRAP region
                       (no trap => RETURN error result carrying e)

  RETURN v         =>  RETURN Ok(v)          (body or trap)

  LET x = g(y) TRAP(e)  =>  MATCH g(y)        ' inline TRAP
    <handler>                CASE Ok(v)    : bind x = v ; continue after END TRAP
  END TRAP                   CASE Error(e) : <handler>
                           END MATCH
                           ' RECOVER w  =>  bind x = w ; continue after END TRAP
                           ' PROPAGATE  =>  PROPAGATE to enclosing TRAP (else RETURN error carrying e)
                           ' RETURN/FAIL diverge as above

  TRAP region (function-level bottom trap):
    PROPAGATE      =>  RETURN error result carrying the bound error
    FAIL e2        =>  RETURN error result carrying e2
    RETURN v       =>  RETURN Ok(v)
```

A call used as a `MATCH` scrutinee **is** rewritten like any other call (it auto-unwraps). No real exceptions, no stack unwinding, no jumps — pure structured value flow that serializes directly into the package's Binary Representation.

---

## 9. Pattern Matching

`MATCH` binds concrete union member values and matches literals; exhaustiveness is checked at compile time. A call scrutinee auto-unwraps (use an inline `TRAP` for local error handling, §8.4).

```basic
FUNC area(s AS Shape) AS Float
  MATCH s
    CASE Circle(c) : RETURN 3.14159 * c.radius * c.radius
    CASE Rect(r)   : RETURN r.w * r.h
    CASE Point(p)  : RETURN 0.0
  END MATCH
END FUNC
```

Also handles values (replacing the old `SELECT CASE`):

```basic
MATCH grade
  CASE "A"      : io::print("Great")
  CASE "B", "C" : io::print("OK")
  CASE ELSE     : io::print("?")
END MATCH
```

- If the scrutinee type is a union type, each non-`ELSE` union case must bind one local: `CASE MemberType(binding)`.
- The scrutinee keeps its declared union type. The bound case local has the concrete member type.
- Literal patterns and comma-separated literal lists.
- `NOTHING`, `TRUE`, `FALSE`, strings, and numbers are literal patterns.
- Enum matches use qualified enum member patterns such as `Color.Red`.
- Guards: `CASE Rect(r) WHEN r.w = r.h : ...`.
- `CASE ELSE` is the catch-all fallback.
- **Exhaustiveness**: unions must cover all member types. Open types (`Integer`, `String`, etc.) require a `CASE ELSE` or it is a compile error. Guarded `CASE` arms do not contribute to compile-time coverage because the guard can fail; use an unguarded arm or `CASE ELSE` to cover the remaining values.
- A call scrutinee auto-unwraps to its value; to handle its failure locally, use an inline `TRAP` (see §8.4). `CASE Ok`/`CASE Error` are not valid match arms — a failure is never matched, only trapped.

---

## 10. Control Flow

```basic
FOR i = 1 TO 10 STEP 2 : io::print(toString(i)) : NEXT
FOR EACH item IN lines : io::print(item) : NEXT
FOR EACH entry IN scores : io::print(entry.key & "=" & toString(entry.value)) : NEXT
WHILE x < 10 : x = x + 1 : WEND
DO : ... : LOOP UNTIL done
DO WHILE ready : ... : LOOP

IF x > 0 THEN io::print("pos") ELSE io::print("non-pos")

IF cond THEN
  ...
ELSEIF other THEN
  ...
ELSE
  ...
END IF
```

`FOR EACH` accepts `List OF T` and `Map OF K TO V` sources. A list loop binds the loop variable as `T`. A map loop binds the loop variable as `MapEntry OF K TO V`; `entry.key` has type `K` and `entry.value` has type `V`. Map loop order is implementation-defined but stable for a given unchanged map value, matching the order used by `keys` and `values`.

`EXIT FOR`, `EXIT DO`, and `EXIT WHILE` leave the innermost enclosing loop whose
kind matches the keyword and continue after that loop. `CONTINUE FOR`,
`CONTINUE DO`, and `CONTINUE WHILE` skip the rest of the current iteration of
the innermost enclosing matching loop. `FOR EACH` uses `EXIT FOR` and
`CONTINUE FOR`; both `DO ... LOOP UNTIL` and `DO WHILE ... LOOP` use `EXIT DO`
and `CONTINUE DO`. The named kind may target an outer matching loop through
inner loops of other kinds; it is a compile error when no matching loop encloses
the statement.

`EXIT SUB` leaves the enclosing `SUB` successfully with no value. `EXIT FUNC` is
always a compile error because a `FUNC` must `RETURN` a value.

`EXIT PROGRAM <integer>` terminates the process with the given exit code from
any call depth. It is not catchable by `TRAP`. Before termination, the runtime
runs lexical cleanup for all live scopes in the current call chain up to the
entry point. Constant exit codes outside the host process range are compile
errors; non-constant values follow the host convention.

There is **no `GOTO`** and **no `SELECT CASE`** (use `MATCH`). `EXIT` and
`CONTINUE` are structured, lexically scoped loop/routine exits, not arbitrary
jumps.

---

## 11. Operators

| Category | Operators |
|----------|-----------|
| Arithmetic | `+  -  *  /  DIV  MOD  ^` |
| Comparison | `=  <>  <  >  <=  >=` |
| Logical | `AND  OR  NOT  XOR` (`AND`/`OR` short-circuit; `XOR` always evaluates both sides) |
| String | `&` (concat) |
| Field access | `.` |
| Pipeline | `\|>` with `_` placeholder |

Precedence, highest to lowest:

| Precedence | Operators / forms |
|------------|-------------------|
| 1          | Primary expressions, calls, constructors, list/map literals, `WITH` |
| 2          | Field access and enum member access: `.` |
| 3          | Unary `-` |
| 4          | Exponentiation: `^` |
| 5          | Multiplication, division, modulo: `*`, `/`, `MOD`, `DIV` |
| 6          | Addition, subtraction: `+`, `-` |
| 7          | String concatenation: `&` |
| 8          | Comparisons: `=`, `<>`, `<`, `>`, `<=`, `>=` |
| 9          | `NOT` |
| 10         | `AND` |
| 11         | `OR`, `XOR` |
| 12         | Pipeline: `|>` |

`XOR` has the same precedence as `OR` and evaluates both operands.

Operator edge cases:

- `&` has lower precedence than `+` and `-`, so `a & b + c` parses as `a & (b + c)`.
- `^` is right-associative: `2 ^ 3 ^ 2` parses as `2 ^ (3 ^ 2)`.
- Unary `-` has higher precedence than `^` in MFBASIC, so `-2^2` parses as `(-2) ^ 2`. Write `-(2 ^ 2)` when the negation should apply after exponentiation.
- Checked numeric failures from operators are ordinary failures and therefore auto-propagate unless handled by a `TRAP`.
- `/` and `MOD` use the numeric promotion table in §4.1. `DIV` always returns `Float`.
- `MOD` is available for every numeric operand pairing and uses a truncation-toward-zero quotient to compute the remainder.

```basic
LET result = nums |> filter(_, isEven) |> transform(_, square) |> sum(_)
```

---

## 12. Collections (owned, binding-driven mutability)

All access is via free functions — **no indexing brackets, no key brackets**. Brackets construct values; functions read and update them.

List literals use the declared or otherwise expected `List OF T` element type when one is available; otherwise the element type is inferred from the first item. Every element must be compatible with that element type. This allows annotated lists of union members, such as `LET shapes AS List OF Shape = [Circle[5], Rect[2, 3]]`.

```basic
LET list  = [1, 2, 3]                          ' List OF Integer (literal)
LET first = get(list, 0)                        ' read (fallible -> auto-propagates)
LET list2 = append(list, 4)                     ' new immutable snapshot
LET safe  = getOr(list, 99, 0)                  ' read with default, never fails

LET m  = Map OF String TO Integer { "a" := 1, "b" := 2 }   ' literal
LET a  = get(m, "a")                            ' read (fallible)
LET m2 = set(m, "c", 3)                          ' new map
LET n  = len(list)

MUT pts AS List OF Vec3 = []
pts = append(pts, v)                            ' in-place append on the mutable buffer
```

- `get` can fail (missing key / out-of-range index fails with an `Error`) and therefore auto-propagates. Use `getOr(coll, key, default)` for the common defaulted read.
- A collection bound with `LET` is an immutable snapshot. Update functions such as `append` and `set` may read it and produce a new collection value, but assigning back to the same `LET` binding or otherwise modifying it is a compile-time error.
- A collection bound with `MUT` is a locally mutable buffer. When the result of an update function is assigned back to the same `MUT` binding, such as `pts = append(pts, v)`, the compiler performs the update destructively in place instead of allocating a replacement collection.
- Update helpers are semantically pure functions. `append(pts, v)` by itself computes and discards a result; it has no lasting effect unless the result is assigned, returned, passed, or otherwise consumed. Destructive update is an optimization only for the assignment-back-to-the-same-`MUT` pattern.
- Update functions on `MUT` collections preserve ownership semantics at boundaries: passing or returning the collection freezes it into an immutable owned value (§14).
- Containers own their contents. Adding a value to a collection stores an owned value in the collection, never a borrowed reference to an external binding. The one exception is a resource handle: a `List` element or `Map` value may hold a **borrow** of a resource (a copy of the handle pointer). The resource itself is owned by a *scope*, not by the collection; the collection closes nothing (§15.6).
- Immutability is deep for the contained value graph. A `LET` collection does not allow mutation of its elements through the collection, and no element can be observed as shared mutable state through another collection or binding.

Built-in collection helpers include `len`, `get`, `getOr`, `find`, `mid`, `replace`, `set`, `append`, `prepend`, `insert`, `removeAt`, `removeKey`, `keys`, `values`, `hasKey`, `contains`, `forEach`, `transform`, `filter`, `reduce`, and `sum`.

The native collection memory layout is specified in `specifications/memory_layouts.md`.

`FOR EACH` over `List OF T` visits items left to right. `FOR EACH` over `Map OF K TO V` visits `MapEntry OF K TO V` values in the map's implementation-defined stable iteration order.

---

## 13. Modules & Packages

**Project source = one package.** The `.mfb` files selected by the current
project's `project.json` together form that project's source package.
Directories inside a source root do not create package boundaries or package
namespaces. Additional packages are introduced only through the importing
project's `project.json` `packages` array.

Visibility:
- `PRIVATE` (default) — file-local.
- `PACKAGE` — visible to all files in the same package, hidden from importers.
- `EXPORT` — visible to importers.

Top-level `LET`, `MUT`, `FUNC`, `SUB`, `TYPE`, `UNION`, and `ENUM` may use `PRIVATE`, `PACKAGE`, or `EXPORT`. Fields in `TYPE` declarations may also use `PRIVATE`, `PACKAGE`, or `EXPORT`; omitted field visibility defaults to the containing type's visibility, capped at `PACKAGE` for non-exported types.

Only exported top-level `FUNC` declarations may use `ISOLATED`. Imported package constructors are addressed as `package::identifier` when constructing values, but constructors for records with hidden fields are callable only from scopes that can see every required field.

Exported top-level `MUT` is allowed only when written explicitly as `EXPORT MUT`; it is package state visible to importers and must be surfaced by audit tooling. A top-level `MUT` without `EXPORT` is private or package-local according to its visibility annotation and remains discouraged for shared state.

Double-colon notation is reserved for package access. Dot notation is reserved for field access into data values and enum members:

```basic
IMPORT shapes
IMPORT longPackageName AS shortName

LET s = shapes::Circle[2.0]
io::print(toString(shapes::area(s)))
io::print(toString(s.radius))
```

Rules:

- A package-qualified name has exactly two parts: `package::identifier`.
- Nested package qualifiers are illegal: `a::b::c` is a compile error.
- Record fields use `value.field`. Methods and object-style access do not exist.
- Imports are not transitive. A package cannot export an imported package or create re-export chains.
- `IMPORT packageName AS aliasName` binds the package to `aliasName` in the importing file. The original package name is not also introduced by that import; use a second import only if both names are needed.
- An import alias must not conflict with another imported package name or alias, a top-level declaration visible in the file, or a built-in package name such as `io`, `math`, `thread`, or `errorCode`.

```basic
' shapes package source
EXPORT FUNC area(s AS Shape) AS Float
EXPORT ISOLATED FUNC worker(path AS String) AS Integer
PRIVATE FUNC helper() AS Float
```

```basic
' main.mfb
IMPORT mathstuff
IMPORT shapes

io::print(toString(shapes::area(shapes::Circle[2.0])))
```

Import graph is resolved at compile time; cycles are an error.

`IMPORT packageName` resolves a package, not an arbitrary source file. The
compiler resolves the first identifier in the import using this order:

1. A built-in package supplied by the toolchain, such as `io`.
2. A package with the same `name` in the importing project's own
   `project.json` `packages` array. If no dependency is declared, resolution
   fails.
3. If the declared dependency has a `source` beginning with `local:///`, the
   rest of the value must be an absolute path. The compiler checks
   `/absolute/path/project.json`; the manifest `name` must match the import and
   `kind` must be `package`. `local://relative` and other non-absolute local
   forms are errors. A package that uses `local:///` cannot be released without
   replacing that dependency source.
4. Otherwise, the compiler checks `<project_root>/packages/packageName.mfp`.
5. If no `.mfp` exists, the compiler checks
   `<project_root>/packages/packageName/project.json`; the manifest `name` must
   match the import and `kind` must be `package`.
6. Otherwise, the declared package is missing from the package store and the
   import is a compile-time error.

`<project_root>/packages` is the resolved dependency store, similar in role to
`node_modules` in Node projects. It is managed by the package manager. The
compiler does not implicitly import undeclared packages from this directory.
Each package is responsible for declaring its own dependencies; dependency
declarations are not inherited from importers and imports are not transitive.

### 13.1 Package identity, versions, and manifests

A package has a stable identity independent of its local directory name. Source projects declare identity, source inputs, and dependencies in a project manifest file named `project.json` at the project root. Source selection follows the manifest's normalized file and directory roots, include and exclude globs, and in-project containment rules. Compiled packages embed the relevant manifest data in the `.mfp` file.

Required manifest fields:

- `name`: the package import name used by source code.
- `version`: a semantic version `MAJOR.MINOR.PATCH`.
- `mfb`: the minimum compatible MFBASIC language version.
- `sources`: source files and roots selected by the project.

Dependency fields:

- `packages`: package dependency entries with names, semantic-version constraints, and optional source locators.
- `native`: optional native dependency metadata for packages that expose `LINK` bindings.

Version constraints use semantic-version ranges such as exact `=1.2.3`, compatible `^1.2.0`, patch-compatible `~1.2.0`, inequalities such as `>=1.2.0 <2.0.0`, or wildcard `1.2.*`. A dependency's selected version must satisfy every constraint that reaches it through the import graph.

The package resolver produces one selected version for each package identity. If two constraints cannot be satisfied by the same version, resolution fails with a package-version diagnostic; the compiler does not load multiple versions of the same package identity into one program.

A package may import a source package or an `.mfp` package. Imported `.mfp` packages must have a compatible binary representation/package format version, compatible public API metadata, and an MFBASIC language version supported by the compiler.

Executable builds use a lockfile named `mfb.lock`. The lockfile records the exact selected package identity, version, source or registry alias, content hash, binary representation/package version, native dependency metadata hash, and transitive dependencies. Locked builds must use the lockfile selections exactly; a hash or version mismatch fails before compilation, IR merging, or native linking.

An **isolated function** is an exported top-level `FUNC` declared with `ISOLATED`. When an isolated function is used as a thread entry point, the runtime starts it in a fresh instance of its package. Starting isolated functions from the same package multiple times creates multiple independent instances; their top-level `MUT` bindings are not shared with each other or with the importing package.

---

## 14. Memory Semantics

MFBASIC values have lexical ownership. Each live value is owned by exactly one binding, container slot, temporary, closure environment, thread message, or return slot. Values are reclaimed by deterministic drop at the end of the owning scope. There is no tracing GC, no reference counting, and no user-visible `free`.

The compiler may choose stack storage, inline storage, heap allocation, or destructive update, but those choices cannot change the ownership behavior described here.

### 14.1 Copy, move, and freeze

- **Copy** creates an independent value with no shared mutable state. Mutating the destination cannot affect the source.
- **Move** transfers ownership from one place to another. After a move, the source binding is uninitialized and any later read, write, capture, comparison, print, return, or drop of that binding is a compile-time use-after-move error.
- **Freeze** converts a mutable collection buffer into an immutable owned collection value. The frozen value may be read and copied or moved according to its element type, but it cannot be mutated through the old mutable buffer.

Primitives, `String`, enums, `Nothing`, records whose fields are copyable, and unions whose active payload is copyable are copyable. `List` and `Map` are copyable only when their element/key/value types are copyable; copying a collection copies its contents. Functions and lambdas are copyable only when their captured environment is copyable. Threads and resource handles are not copyable.

The compiler may replace a semantic copy with a move when it proves the source is not used afterward. This is an optimization only; it must not change diagnostics or observable behavior except performance.

### 14.2 Assignment and initialization

`LET name = expr`, `MUT name = expr`, record construction, union construction, collection construction, and return-slot initialization all consume the expression result into the destination.

When the expression is a binding:

- If the value's type is copyable and the binding is used again, assignment copies it.
- If the value's type is copyable and the binding is not used again, the compiler may move it.
- If the value's type is not copyable, assignment moves it and the source binding becomes unusable.

Reassigning a `MUT` first drops the old value in the binding, then initializes the binding with the new value. If evaluating the right-hand side fails, the old value remains live.

### 14.3 Function calls and returns

Function arguments are owned values. Passing an argument follows the same copy-or-move rules as assignment. A call cannot observe or mutate a caller-owned value after the argument has been passed, except through a standard resource borrow described in §15.

Returning a value moves it into the caller's return slot. Returning a local collection is valid because ownership leaves the callee before local scope cleanup. Returning a `MUT` collection freezes the mutable buffer into an immutable owned collection value. Returning a non-copyable local value moves it; the callee does not drop that moved-from binding.

Default arguments are evaluated at the call site and then passed under the same rules as explicit arguments.

### 14.3.1 Native heap value contract

Native backends use one allocator-agnostic IR contract for heap-backed values. The IR names value operations; native lowering chooses whether a value is inline, static, stack-resident, or arena-backed.

This language specification defines the ownership, aliasing, copy, move, and return behavior of heap-backed values; it does not define a universal per-object header or a byte-for-byte native representation for every value kind. Concrete runtime layouts for strings, records, unions, collections, and any future heap-backed value category are specified in `specifications/memory_layouts.md` and in the corresponding package/native ABI specifications when values cross an ABI boundary. Native lowering must follow those layout contracts consistently for construction, field access, union wrapping and extraction, collection storage, helper calls, and package/native ABI interop.

Arena allocation is an implementation strategy for native backends. An arena allocator may maintain allocator-private block headers or bookkeeping, but those allocator structures are not part of the source-level value model and must not be treated as a required object prefix for all arena-backed values.

Copy and move of arena-backed immutable values may be represented by copying the native value handle used by the active layout, provided the ownership rules above remain observable. Drop may be a no-op for individual values when all owned arena blocks are released at package-instance shutdown. Returning a heap-backed value copies or moves the native value into caller-owned storage according to the active layout, so returned values never point into the callee stack frame or into an arena whose lifetime is shorter than the caller-visible value.

Each package instance owns one arena. Worker threads or future isolated package instances get distinct arenas. A value that crosses from one arena-owned execution context to another must be transferred into storage whose lifetime is valid for the receiver before the receiver observes it. The native representation must not expose a handle into the sender's arena as a receiver-owned value unless that arena is also kept alive by the transfer object for the full receiver-visible lifetime. A failed heap allocation surfaces as an ordinary language-level error — `ErrInvalidArgument` for an invalid request and `ErrOutOfMemory` on exhaustion — and auto-propagates like any other failure. The allocator mechanism is an implementation detail specified in `specifications/memory_layouts.md`.

### 14.4 Closures and first-class functions

Closures capture `LET` bindings by value when the closure is created. Capturing a copyable value copies it into the closure environment unless the compiler can move it without changing later validity.

Capturing `MUT` bindings is a compile-time error because closures do not capture live mutable cells. Capturing resource handles or any other non-copyable values is also a compile-time error in v1 unless a later non-escaping closure feature explicitly defines local borrowing or move rules.

A closure environment is owned by the function value. Dropping the function value drops its captured values in reverse capture order.

### 14.5 Recursive unions and allocation

Recursive concrete unions are represented through compiler-managed owned nodes. A recursive edge is an owned child value, not a shared pointer. The compiler rejects value cycles; a program cannot construct a `List`, `Map`, record, or union value that directly or indirectly owns itself.

Independently of this construction-time check, a record type whose fields cycle back to itself without passing through a `List`, `Map`, or `UNION` is rejected at declaration time with `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION` (see §4.2), because such a type has no base case and can never be constructed.

Because cycles are impossible and each edge has one owner, dropping a recursive value recursively drops its owned children without GC or refcounting. Implementations may use iterative drop internally to avoid stack overflow on deeply nested values.

### 14.6 Containers and aliasing

`List` and `Map` own every stored element, key, and value. Inserting into a container copies or moves the inserted value into the container; it never stores a borrowed alias to an external binding. Removing from a container moves the removed value out when the API returns it, or drops it when the API discards it.

Ordinary containers cannot store thread handles, and cannot store resource handles as a `Map` *key* (handles are not comparable, §4.10). A `List` element or `Map` *value*, however, may hold a **borrow** of a resource — a copy of the one handle pointer (§15.6). Such a borrow is never an owner: the resource stays owned by a scope, which closes it exactly once on exit; the collection closes nothing, and copying or dropping a collection only copies or discards borrows. Containers can store functions only when the function value is copyable or movable under the closure rules above.

No two live mutable bindings may refer to the same collection buffer. A `MUT` collection buffer may be destructively updated only while it is owned by that single live `MUT` binding. Reads produce owned values, not aliases into the buffer.

### 14.7 Drop order

At normal scope exit, `RETURN`, `EXIT FOR`/`EXIT DO`/`EXIT WHILE`, `EXIT SUB`,
`CONTINUE FOR`/`CONTINUE DO`/`CONTINUE WHILE`, `FAIL`, `PROPAGATE`, or
auto-propagated errors, live bindings are dropped in reverse declaration order
within each scope. `EXIT PROGRAM` is a stack-wide drop edge that unwinds every
live scope up to the entry point before process termination. Nested scopes drop
before enclosing scopes continue. Record fields drop in declaration order. Union
member values drop according to the active member's record layout. List elements
drop from highest index to lowest. Map entries drop in implementation-defined
storage order; programs must not depend on map drop order.

Moved-from bindings are not dropped. Frozen buffers are dropped as immutable collection values by their final owner.

### 14.8 Diagnostics

The compiler must diagnose:

- Use after move.
- Copy attempts for non-copyable types.
- Cyclic value construction.
- Capturing `MUT` bindings in closures.
- Capturing resource handles in ordinary closures.
- Capturing other non-copyable values in ordinary closures.
- Storing thread handles in ordinary collections, or using a resource handle as a `Map` key.
- Binding a borrowed collection element of resource type with `RES` (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`), or otherwise treating such a borrow as an owner.
- Any control-flow path that could drop the same resource or owned value more than once.

`.mfp` packages must preserve enough ownership metadata for import-time type checking and Binary Representation verification (§21).
At minimum, exported type shape metadata must remain sufficient to reconstruct copyability, resource/thread containment, and drop-sensitive ownership checks when imported packages participate in move analysis.

---

## 15. Resource Management

Resource values, such as files and sockets, are unique handles. At any point in the program, exactly one live owner is responsible for each open handle. A resource is bound with the **`RES`** keyword (§5) — the ownership axis — and never with `LET`/`MUT`. Resources are closed automatically by lexical drop (§14.7) when their owning binding leaves scope, on every exit path: normal scope exit, `RETURN`, `EXIT`/`CONTINUE`, `FAIL`, `PROPAGATE`, an auto-propagated failure, and `TRAP` routing. `EXIT PROGRAM` performs the same cleanup across every live caller frame before terminating. There is no user-visible lifetime construct; a resource is released by the same ownership and drop rules as any other owned value.

```basic
FUNC readFirstLine(path AS String) AS String
  RES f AS File = fs::openFile(path)   ' auto-propagates on failure
  LET line = fs::readLine(f)           ' if this fails, f is still closed on the error exit
  RETURN line                          ' f is dropped (closed) here, on the success exit
END FUNC
```

A resource is closed exactly once. **Ordinary calls borrow.** Passing a `RES` binding to an ordinary function creates an exclusive, call-scoped borrow: the callee may use the handle and mutate its `STATE`, but does not take ownership, and the caller's binding stays live after the call. A `RES` binding is invalidated **only** by this fixed set of events, all visible at the call site:

1. the resource's **registered close op** (e.g. `fs::close(f)`) and its re-export aliases;
2. **`thread::transfer`** of the resource (§16);
3. **`RETURN`** of the resource (move out to the caller);
4. **scope-drop** at the end of the binding's lexical scope (auto-close).

A borrow grants *use* but never the right to *invalidate*: a callee that only borrowed a resource cannot close it, `RETURN` it, or `thread::transfer` it (`TYPE_RESOURCE_BORROW_INVALIDATE`) — those require ownership. There is no per-function borrow/consume inference and no `BORROW`/`MOVE` annotations: a call either is one of the four events or it borrows. A resource handle cannot be printed, compared, serialized, or captured by a lambda or ordinary closure. Its pointer may be copied only as a **borrow** into a `List` element or `Map` value (§15.6) — never duplicating the resource, and never as a `Map` key. A concrete resource handle may be sent to a thread only when that resource type is thread-sendable.

```basic
RES f AS File = fs::open("app.db", "read")
exec(f, "...")        ' borrow — f still live
exec(f, "...")        ' borrow — f still live
fs::close(f)          ' registered close → f invalidated
' exec(f, "...")      ' COMPILE ERROR: f used after close
```

A resource value may be passed only to a function whose parameter is declared `RES` and explicitly names that concrete resource type, such as `RES f AS File`, `RES s AS Socket`, or a `LINK`-declared resource. A function returns a resource with an explicit `AS RES <Type>` return. There is no generic resource supertype, no structural matching of handles, and no implicit conversion between resource types.

**Resources are atomic — records never hold them.** A record (product type) may never contain a resource field, directly or transitively (`TYPE_RESOURCE_FIELD_FORBIDDEN`): a resource field would either trap copyable data behind move-only semantics or let one value own several resources at once. Data that belongs *with* a resource travels in the resource's `STATE`, and to work with several resources you hold several `RES` bindings.

**`STATE` — data carried by a resource.** A `RES` binding may attach an associated data value with `STATE T`:

```basic
TYPE FileState        ' an ordinary, copyable data record
  pos AS Integer
  len AS Integer
END TYPE

RES s AS File STATE FileState = fs::open("app.db", "read")   ' state default-initialized
LET here = s.state.pos                                       ' read a state field
s.state.pos = 10                                             ' update one field in place
s.state = WITH s.state { pos := 10 }                         ' or replace the whole state
```

`T` must be an ordinary **copyable, defaultable data type** (`TYPE_STATE_INVALID` otherwise); since no data type may contain a resource, `T` is automatically resource-free. The state is owned by the resource, default-initializes when the resource is produced, rides through `RES` signatures (`RES s AS File STATE FileState`), and is freed when the resource drops or is closed. `STATE` is optional.

`s.state` reads the state record. It is updated either by assigning a single field in place (`s.state.field = value`) or by assigning a whole-state `WITH` update (`s.state = WITH s.state { field := value }`); the former is shorthand for the latter. These are the only member-target assignments in the language. Because a resource value is a shared handle, a state update made through a borrowed `RES` parameter is visible to the owner after the call.

**Resource unions.** A union whose every variant is a resource type is itself a resource — a *resource union* — and is `RES`-bound like any other resource:

```basic
UNION Stream            ' every variant is a resource → Stream is a resource
  File
  Socket
END UNION

RES s AS Stream = fs::open("app.db", "read")   ' a File wraps into the union
MATCH s
  CASE File(f)
    LET line = fs::readLine(f)
  CASE Socket(sock)
    LET data = net::read(sock, 1024)
END MATCH
' scope end → drop closes the active variant via its registered close op
```

A resource union owns exactly one resource at a time (the active variant), so it is atomic — a *choice* among resources, not a bundle. **Drop is tag-dispatched**: cleanup reads the union tag and calls the active variant's registered close op. Matching a resource union *borrows* the active variant (the union retains ownership and closes it on drop). A union may **not mix** data and resource variants (`TYPE_MIXED_RESOURCE_UNION`), and a resource union carries no `STATE`.

To release a resource earlier than the end of its scope, or to observe a close failure, call the resource's explicit close operation (such as `fs::close(f)`). That operation consumes the handle and auto-propagates a close failure like any other call, so the close failure is directly observable. After an explicit close the binding is moved and is not closed again by lexical drop.

A close that runs as part of an implicit lexical drop cannot inject an error into program flow, because a drop has no source-level result to route. If such a drop-close fails, the failure is emitted as diagnostic/audit metadata associated with the failed cleanup; it does not replace, wrap, or raise a source-level `Error`. Programs that must observe a close failure use the explicit close operation instead.

This rule does not change the built-in `Error` shape: A secondary close failure is not directly inspectable by ordinary source code unless a future diagnostics API exposes cleanup metadata.

Compiled cleanup metadata must preserve enough information for runtime and audit tooling to report a drop-close failure. Package audit output should identify cleanup regions that retain this failure metadata.

### 15.6 Resources in collections

A resource is owned by a **scope** — never by a binding or a collection. A `RES` binding, a borrowed `RES` parameter, and a collection slot (a `List` element or `Map` value) all hold a **borrow**: a copy of the one handle pointer. Copying the pointer is a borrow, never a duplication of the resource, and a collection slot is a borrow, not a resource binding. None of these close the resource; the owning scope closes it exactly once on exit, on every path.

A resource appearing as a collection element carries the **`RES` ownership-axis marker**, exactly as a binding (`RES f`), a parameter (`RES f AS File`), or a return (`AS RES File`) does. The only spelling for a list of files is `List OF RES File` (and `Map OF String TO RES File` for a map value); a bare `List OF File` is rejected just like `LET f AS File` (`TYPE_RESOURCE_REQUIRES_RES`), and `RES` on a non-resource element is rejected like `RES x AS Integer` (`TYPE_RES_REQUIRES_RESOURCE`). The marker is an ownership annotation only — the collection is still an ordinary copyable collection of borrows and owns nothing.

By default the owning scope is the scope where the resource is produced. The single rule that governs collections is **ownership floats up**:

> Adding a borrow of a resource to a collection migrates the resource's owning scope up to the collection's scope when that scope outlives the current owner. Ownership always floats to the **outermost** scope that references the resource; it never moves down. If a referencing collection escapes the function (it is `RETURN`ed), ownership moves out to the caller, exactly like `RETURN`ing the resource itself.

Consequences:

- A borrow added to a **higher-scope** collection raises the owning scope to that collection's scope; the resource closes once when that outer scope exits, and every borrow (the original binding and the collection elements) is within that scope, so none dangles.
- A borrow added to a **same- or lower-scope** collection leaves ownership unchanged; the collection just holds a borrow.
- A binding whose ownership has floated to an outer scope becomes a plain **borrow**: still usable, but it no longer closes at its own scope exit and may not close, `RETURN`, or `thread::transfer` the resource (`TYPE_RESOURCE_BORROW_INVALIDATE`).

Because all references are within the owning scope, `get` and `FOR EACH` of a resource element yield a **borrow**, statically safe with no runtime dependence on the closed flag (the flag is only a backstop that keeps the single close idempotent when a handle is reachable by more than one path). Such a borrow is not an owner: binding it with `RES`, or closing/returning/transferring it, is an error (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`). Collections of resources are ordinary copyable collections of pointers — no move-only or linearity — and the helpers that require a comparable element (`find`, `contains`, `replace`) remain unavailable because handles are not comparable, the same reason resources cannot be `Map` keys.

A resource element placed into a collection must be a named `RES` binding (the owner); a temporary or a borrowed element is not an owner and cannot be stored (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`).

**Returning a resource collection transfers scope-ownership to the caller**, exactly as `AS RES File` does for a single resource. A function returning `AS List OF RES File` releases the close obligations for the referenced resources — it does not close them — and the caller's binding scope **adopts** them, closing each once at its own exit. (A bare `List OF File` return is rejected for the missing `RES` marker.) On an error exit *before* the return, the resources are still closed by the function's scope, because they ride its owned-list until the `RETURN` transfers it. A resource collection may also be passed to a function, where the callee borrows its elements (and may not close them). The resources must be added to the collection at or after the collection's own binding so the obligation rides the collection. Sharing a resource collection across threads remains out of scope.

---

## 16. Threads

Threads are isolated execution contexts created from `ISOLATED FUNC` entry points. They do not share lexical scope, package state, mutable collections, or resources with their parent thread or with each other.

```basic
IMPORT workers
IMPORT thread

' workers/jobs.mfb
' EXPORT ISOLATED FUNC parseFile(worker AS ThreadWorker OF String TO Integer, path AS String) AS Integer

LET t = thread::start(workers::parseFile, "data.csv")

WHILE thread::isRunning(t)
  IF thread::poll(t, 10) THEN
    LET message = thread::receive(t)
    io::print(message)
  END IF
WEND

LET count = thread::waitFor(t)
io::print("Parsed " & toString(count) & " records")
```

Rules:

- A thread entry point must have type `ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out`. The worker handle is passed as the first argument by the runtime when the worker starts.
- A thread entry point must be an exported `ISOLATED FUNC` from an imported package. Starting a function from the current package is a compile error.
- A thread entry point must not be a `SUB`.
- A thread entry point must not be a closure or lambda. It must be a named package function.
- Each started thread receives its own fresh instance of the entry function's package, including a distinct worker arena. Starting isolated functions from the same package more than once creates independent package state for each thread.
- Thread arguments and messages are copied, moved, or frozen when they enter a thread. Values read from a thread are copied, moved, or frozen when they leave the thread. No sender and receiver can observe or mutate the same live value. Heap-backed boundary values are materialized in receiver-valid storage before user code observes them.
- Thread boundary types must be thread-sendable. Primitive owned values, `String`, `Nothing`, records, unions, and immutable containers are sendable when every contained field, payload, element, key, or value type is sendable. Functions, lambdas, `Thread`, `ThreadWorker`, and opaque resource handles are not sendable by default. (A worker outcome — internally a fallible result — is sendable when its success type is.)
- Concrete resource types opt in to thread sendability. Standard `File`, `Socket`, and `UdpSocket` handles are sendable. `Listener` is not sendable. A successful send of a non-copyable sendable resource moves ownership to the destination side immediately; a failed send leaves ownership with the sender.
- A thread's top-level `MUT` state is private to that thread's package instance.
- If the thread entry function succeeds with `v`, the thread's stored outcome carries the success value `v`. If it fails with `Error(e)`, including through auto-propagation, the stored outcome carries `e`. If the stored outcome still references worker-arena storage internally, the runtime keeps that worker arena live through the `Thread` outcome owner and materializes a receiver-owned copy before `thread::waitFor(t)` exposes the value to user code.
- The `Thread` value owns the completed outcome after the thread ends until it is retrieved. `thread::waitFor(t)` waits until completion, retrieves the outcome, auto-unwraps the `Out` value or auto-propagates the `Error` like any other function call, and consumes/closes the parent `Thread` handle. After retrieval, any further use of the same `Thread` handle fails with `ErrResourceClosed`.

The `thread` package exposes:

```basic
thread::start OF In, Msg, Out(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In, inboundLimit AS Integer = 64, outboundLimit AS Integer = 64) AS Thread OF Msg TO Out
thread::isRunning OF Msg, Out(t AS Thread OF Msg TO Out) AS Boolean
thread::waitFor OF Msg, Out(t AS Thread OF Msg TO Out) AS Out
thread::cancel OF Msg, Out(t AS Thread OF Msg TO Out) AS Nothing
thread::send OF Msg, Out(t AS Thread OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing
thread::poll OF Msg, Out(t AS Thread OF Msg TO Out, ms AS Integer) AS Boolean
thread::receive OF Msg, Out(t AS Thread OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg
thread::send OF Msg, Out(t AS ThreadWorker OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing
thread::receive OF Msg, Out(t AS ThreadWorker OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg
thread::isCancelled OF Msg, Out(t AS ThreadWorker OF Msg TO Out) AS Boolean
thread::transfer OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer = 0) AS Nothing
thread::accept OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, timeoutMs AS Integer = 0) AS RES Res
thread::transfer OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer = 0) AS Nothing
thread::accept OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, timeoutMs AS Integer = 0) AS RES Res
```

**Two planes across a thread boundary.** A thread type carries an optional resource plane: `Thread OF Msg RES Res TO Out` (and `ThreadWorker OF …`), where `RES Res` is the resource channel and may be omitted for a data-only thread (`Thread OF Msg TO Out`). A thread with only a resource channel is spelled `Thread OF RES Res TO Out` (the message slot defaults to `Nothing`). The two planes use **separate per-thread queues**, so a thread may carry both at once. The message channel (`thread::send` / `thread::receive` / `thread::poll`) carries **copyable, resource-free data**: a resource in the `Msg` slot is rejected (`TYPE_THREAD_NOT_SENDABLE` — declare it on the `RES` plane). Resources cross on the **resource plane** (`thread::transfer` / `thread::accept`), typed by `Res`. `thread::transfer(t, res)` **moves** `res` to `t` (invalidation event #2, §15): the sender binding is consumed, with ownership returned to the sender on failure (a `TRAP` handler may reuse it). `thread::accept(t)` receives a transferred resource and binds it with `RES`; a resource's `STATE` is declared on that binding and moves with the resource. Only thread-sendable resource types may cross.

Thread functions are ordinary built-in templates. Their `Msg` and `Out` parameters are resolved by the template rules in §3 from argument types and expected result types. `thread::start` gets `Msg` and `Out` from the started function's first `ThreadWorker OF Msg TO Out` parameter, and gets `In` from the started function's second parameter and the `data` argument. If a thread does not exchange messages, `Msg` may be `Nothing`.

Each thread has a bounded inbound queue and bounded outbound queue. Queue entries own transferred values in receiver-valid storage or runtime transfer storage, never as a bare dependency on the sender's arena. `thread::start` rejects limits less than `1` with `ErrInvalidArgument`. `thread::send(Thread, ...)` sends a value to the worker inbound queue. `thread::receive(ThreadWorker, ...)` reads from that inbound queue and is valid only inside the running worker. `thread::send(ThreadWorker, ...)` sends to the parent-visible outbound queue. `thread::poll` waits up to `ms` milliseconds for an outbound message from the worker and returns `TRUE` when `thread::receive(Thread, ...)` can read without blocking. `thread::receive(Thread, ...)` reads the next outbound message. Reading with no available message fails with `ErrNotFound`.

For queue operations, `timeoutMs = 0` means do not wait. A positive timeout waits up to that many milliseconds for space or data. Sending to a full queue or receiving from an empty queue after the timeout fails with `ErrTimeout`. Negative timeouts are invalid except where a specific overload documents an indefinite worker-side wait, such as `thread::receive(ThreadWorker, -1)`.

`thread::cancel` requests cooperative cancellation. It does not kill the worker immediately. The worker observes cancellation with `thread::isCancelled(t)` and should return or fail promptly. After cancellation is requested, new parent-side `thread::send` calls fail with `ErrInterrupted`; unread inbound messages may be discarded. Outbound messages already sent by the worker remain readable until drained. Runtime-managed worker queue cancellation points, including `thread::receive(ThreadWorker, ...)` and `thread::send(ThreadWorker, ...)`, wake and fail with `ErrInterrupted` when cancellation is requested. Other blocking built-ins that are implemented as runtime-managed waits, such as terminal input, blocking file reads, or network waits, must use the same cooperative error-return model when cancellation integration is provided. Cancellation points do not asynchronously kill the worker or interrupt arbitrary user/native code.

When a thread ends, its inbound queue is closed and further parent-side sends fail. Its outbound queue remains readable until drained; after it is empty, `thread::poll` returns `FALSE` and `thread::receive(Thread, ...)` fails with `ErrNotFound`. `thread::waitFor` may be used before or after draining messages; it retrieves the stored outcome exactly once and closes the parent `Thread` handle. Closing the handle drops any remaining queued outbound messages. The worker arena may be released only after the worker result has been transferred out of that arena or the runtime has otherwise kept that arena live through result retrieval, and any outbound messages have either been transferred to queue-owned storage or dropped. Dropping a completed `Thread` handle releases all remaining queued messages. Dropping a running `Thread` handle requests cancellation and detaches the worker; the runtime must reclaim the worker when it exits, preventing zombie threads.

`Thread` values are non-copyable owned handles and participate in lexical cleanup. Scope exit, `RETURN`, `FAIL`, `PROPAGATE`, auto-propagated errors, and trap routing drop live parent `Thread` handles in reverse declaration order together with other owned values. Reassigning a `MUT Thread` evaluates the right-hand side first; if that succeeds, the old handle is dropped before the binding stores the new handle. A `Thread` binding that has moved out through return or another consuming operation is not dropped by the source scope. `thread::waitFor(t)` closes the underlying handle but does not make the source binding syntactically moved; later user-visible operations fail with `ErrResourceClosed`, while compiler-generated lexical cleanup is idempotent for an already closed handle.

---

## 17. Native Libraries

Native libraries are host dynamic libraries loaded through reusable `.mfp` binding packages. MFBASIC code cannot call arbitrary C symbols directly. A binding package introduces its **native resource types at package scope** (`RESOURCE … CLOSE BY …`) and declares a `LINK` block holding the library name, its package-like namespace, and the typed native wrapper functions visible to MFBASIC code. Compiling that package emits a normal `.mfp` (structured Binary Representation) plus native binding metadata.

Application packages do not repeat a dependency's `LINK` block. They import the binding package normally with `IMPORT`, call its exported wrapper functions, and use its resource types through ordinary ownership and lexical-drop behavior. Final executable builds collect native dependencies from all imported `.mfp` packages, resolve them once for the target platform, validate their manifests, and link or load the declared native libraries before `main`. Each library is opened and every declared symbol resolved by a generated load-time initializer that runs before `main`; if a library or symbol cannot be loaded the program aborts at startup with `ErrNativeBindingUnavailable` (`77030007`) rather than continuing with an unbound symbol.

* Native ABI details do not leak across package boundaries unless explicitly part of the binding package's public API.
* Application code importing a binding package sees ordinary MFBASIC types, functions, resources, failure/auto-propagation behavior, and lexical-drop cleanup behavior.
* A source package that declares `LINK` is a binding package. It may also include ordinary MFBASIC wrapper code, validation, and higher-level helpers around the native symbols.

```basic
' The native resource type is declared at PACKAGE scope. `EXPORT` makes it
' nameable by importers as `sqlite::Db`; `CLOSE BY` names its registered close
' op — a native LINK function declared below.
EXPORT RESOURCE Db CLOSE BY sqlite::close

LINK "sqlite3" AS sqlite
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

' Re-export the registered close op under the package name (see below).
EXPORT FUNC close AS sqlite::close
```

`LINK "sqlite3" AS sqlite` creates the namespace `sqlite`, so native wrapper functions are called like package functions. A producer wrapper returns `AS RES Db`, so its result is bound with `RES`:

```basic
RES db AS Db = sqlite::open("app.db")
' borrow db through wrapper calls...
' db is closed by lexical drop when its scope ends, or by an explicit sqlite::close(db)
```

**Declaring the native resource (`[visibility] RESOURCE Name CLOSE BY closeFn`).** A native resource is declared at **package scope, not inside the `LINK` block** — it is named and exported exactly like any other type, so package wrapper code resolves the bare name `Db` and `EXPORT` lets importers write `sqlite::Db`. Omitting `EXPORT` keeps it package-private. The declaration may forward-reference a `LINK` function defined later in the file. `Db` is an opaque unique native handle whose hidden representation is a `CPtr`; source code cannot inspect, cast, compare, serialize, print, copy, capture in a lambda, store in an ordinary collection, do arithmetic on it, or name its `CPtr`. A resource may be passed only to functions whose signatures explicitly accept that resource type in a `RES` position. Resources are not thread-sendable unless the declaration opts in with a trailing `THREAD_SENDABLE`.

`CLOSE BY <closeFn>` names the resource's **registered close op** — a native `LINK` function whose single `RES` parameter is this resource type (overhaul invalidation event #1). It runs automatically when the resource binding is dropped at scope exit, including on error exits, and may also be called explicitly to release early or to observe a close failure. `closeFn` must be a native `LINK` function; naming an ordinary MFBASIC function is rejected (`RESOURCE_CLOSE_NOT_NATIVE`), and a close op that does not consume exactly one `RES` parameter of its resource is rejected (`RESOURCE_CLOSE_SIGNATURE`). Calling a native wrapper with a closed resource fails with `ErrResourceClosed`.

**Re-exporting the close op (`[visibility] FUNC alias AS qualified::func`).** A binding publishes its close op under the package name with a transparent **function alias**, so importers can close explicitly through `sqlite::close(db)`. The alias is the *same* registered close op: calling it consumes its `RES` argument exactly as the native close op does. A hand-written wrapper `FUNC close(RES db AS Db) … sqlite::close(db)` cannot replace it, because its parameter is a borrow and a borrow may never invalidate (§15) — there is no `MOVE` annotation. The alias form is required for any close op importers should be able to call.

`SYMBOL "sqlite3_open"` gives the exact native symbol name to look up in the loaded library. The MFBASIC function name is the public wrapper name; it does not have to match the native symbol name.

`ABI (...) AS ...` gives the native C-facing call shape. The `FUNC` signature is the MFBASIC-facing wrapper type; the `ABI` signature is the host-library symbol's argument and return representation. Each ABI slot is `name type` in native C argument order, and slots bind to wrapper parameters **by name** (so `path` in the ABI matches the `path` parameter). One slot may be named `return` to mark the wrapper's result (an `OUT` slot for a produced handle/value, or the native return slot after `AS`). Every wrapper parameter must map to an ABI slot of the same name, and every ABI slot must be satisfied by exactly one of: a wrapper parameter, the `return` result marker, or a `CONST` pin — otherwise `NATIVE_ABI_UNBOUND_PARAM` / `NATIVE_ABI_UNBOUND_SLOT`.

Native ABI types are separate from MFBASIC source types:

| Type | Meaning |
|------|---------|
| `CInt8`, `CInt16`, `CInt32`, `CInt64` | Signed fixed-width C integer values. |
| `CUInt8`, `CUInt16`, `CUInt32`, `CUInt64` | Unsigned fixed-width C integer values. |
| `CBool` | C `_Bool` / `bool` value. |
| `CFloat32`, `CFloat64` | 32-bit and 64-bit C floating-point values. |
| `CIntPtr`, `CUIntPtr` | Signed and unsigned integer values with pointer width. |
| `CSize` | Unsigned C size value, equivalent to `size_t`. |
| `CString` | Null-terminated UTF-8 string pointer created from a MFBASIC `String` for the duration of the call. Embedded NUL bytes are rejected before the native call with `ErrInvalidArgument` (`77050002`). |
| `CPtr` | Opaque native pointer value used only inside native bindings. It cannot be inspected, manipulated, stored, returned, or named by ordinary MFBASIC code except as the hidden representation of a declared `RESOURCE`. |
| `CVoid` | Native `void` return. Valid only as an ABI return type. Use MFBASIC `Nothing` for the wrapper's source-level return type. |

The fixed-width names are preferred over C spellings such as `int` or `long`, because those spellings vary by platform. Bindings should map the platform header's actual ABI to one of the fixed or pointer-sized types.

The marshaling boundary validates values rather than silently corrupting them: an `Integer` argument that does not fit a narrower signed C integer fails with `ErrOverflow` (`77050010`) instead of truncating; a C floating-point **return** that is NaN or infinite is rejected with `ErrFloatNaN` (`77050013`) / `ErrFloatInf` (`77050014`), since an MFBASIC `Float` is always finite (§3); and the bytes of a returned C string are validated as UTF-8, failing with `ErrEncoding` (`77020004`) when malformed.

ABI parameters may use direction modifiers:

| Form | Meaning |
|------|---------|
| `REF T` | Pass a pointer to a temporary native value initialized from the MFBASIC argument. The pointer lifetime ends when the native call returns. |
| `OUT T` | Pass a pointer to uninitialized native storage and copy the result back after the call. The pointer lifetime ends when the native call returns. |
| `CPtr` | Pass a resource handle or opaque pointer as-is inside the binding boundary. |

**Pinning constant and NULL arguments (`CONST slot = value`).** The `ABI (...)` line always states the true native signature — every C argument in C order. Some of those arguments are fixed values the caller never supplies (a `-1` length, a NULL callback, a sentinel destructor). `CONST <slot> = <value>` pins one ABI slot to a fixed value and removes it from the wrapper's parameter list. The value is checked against the slot's declared ABI type. `NOTHING` pins a C NULL on a pointer slot; a pointer-sized integer literal pins a sentinel pointer (e.g. `-1` for SQLite's `SQLITE_TRANSIENT`). A `CONST` slot is input-only — marking it `OUT` or as the result is rejected (`NATIVE_CONST_OUT`), and pinning an unknown slot is `NATIVE_CONST_UNKNOWN_SLOT`. A pin is call metadata baked into the native frame; it never materializes as a source value, so it cannot forge or leak a `CPtr`.

```basic
FUNC bindText(RES stmt AS Stmt, iCol AS Integer, zVal AS String) AS Nothing
  SYMBOL "sqlite3_bind_text"
  ABI (stmt CPtr, iCol CInt32, zVal CString, nByte CInt32, destructor CPtr) AS status CInt32
  CONST nByte = -1            ' bind up to the terminating NUL
  CONST destructor = -1       ' SQLITE_TRANSIENT (void*)-1: copy the bytes now
  SUCCESS_ON status = 0
END FUNC
```

**Success gating (`SUCCESS_ON` / `ERROR_ON`).** When the native return is a status code rather than the result, a Boolean expression over the named slots decides success:

| Form | Meaning |
|------|---------|
| `SUCCESS_ON <expr>` | The wrapper succeeds when `<expr>` is true; any other outcome auto-propagates as an `Error`. |
| `ERROR_ON <expr>` | The De Morgan complement of `SUCCESS_ON`; the wrapper fails when `<expr>` is true. A wrapper states one, not both. |

`<expr>` is any Boolean expression over slot names, including compound conditions: `SUCCESS_ON status = 0`, `SUCCESS_ON status >= 0`, or `SUCCESS_ON status = 100 OR status = 101`. Comparisons bind tighter than `AND`/`OR` (§11), so the compound form needs no parentheses. `SUCCESS_ON status = 0` is common for libraries such as SQLite; `ERROR_ON status = -1` is common for POSIX-style APIs. When the gate fails, the wrapper produces `ErrNativeBindingCallFailed` (`77030008`), which auto-propagates like any other call failure.

**Result value mapping (`RESULT <expr>`).** When the wrapper's result is *derived from* the status (rather than passed straight through or produced via `OUT`), `RESULT <expr>` supplies it. For example SQLite's `sqlite3_step` returns `SQLITE_ROW` (100) or `SQLITE_DONE` (101); the wrapper returns `AS Boolean` meaning "a row is ready":

```basic
FUNC step(RES stmt AS Stmt) AS Boolean
  SYMBOL "sqlite3_step"
  ABI (stmt CPtr) AS status CInt32
  SUCCESS_ON status = 100 OR status = 101   ' both are non-errors
  RESULT status = 100                       ' TRUE iff a row is ready
END FUNC
```

A plain value-returning call needs neither gate nor mapping: name the native return slot `return` and the C return becomes the wrapper's result (e.g. `ABI (stmt CPtr, name CString) AS return CInt32`). A value-producing wrapper that marks no result (`return` / `RESULT`) is rejected (`NATIVE_ABI_NO_RESULT`).

**Multiple outputs (`RETURN_OUT`).** When an ABI signature has more than one `OUT` slot, `RETURN_OUT` defines how those outputs become the success value, referencing slots by name. A single `OUT` slot named `return` is returned implicitly.

```basic
TYPE DivModResult
  quotient AS Integer
  remainder AS Integer
END TYPE

LINK "mylib" AS mylib
  FUNC divmod(a AS Integer, b AS Integer) AS DivModResult
    SYMBOL "divmod"
    ABI (a CInt32, b CInt32, quotient OUT CInt32, remainder OUT CInt32) AS CVoid
    RETURN_OUT DivModResult[quotient, remainder]
  END FUNC
END LINK
```

`RETURN_OUT DivModResult[quotient, remainder]` means: after the native call succeeds, read the named `OUT` slots and succeed with `DivModResult[quotient, remainder]`.

**Freeing a caller-owned return (`FREE`).** A `CPtr` result mapped to an owned MFBASIC value (such as `AS String`) is **copied** out of the native buffer and the source pointer is then left untouched — *copy-and-leave*. That is correct when the native library **owns** the buffer and keeps it valid (a transient or static pointer), as with `sqlite3_column_text`. When the call instead returns a buffer the **caller owns and must release** — `sqlite3_expanded_sql`, `sqlite3_mprintf`, `strdup` — copy-and-leave would leak it. A `FREE` block names the produced slot and the deallocator that releases it:

```basic
LINK "sqlite3" AS sqlite
  FUNC expandedSql(RES stmt AS Stmt) AS String
    SYMBOL "sqlite3_expanded_sql"
    ABI (stmt CPtr) AS return CPtr
    FREE return
      SYMBOL "sqlite3_free"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK
```

`FREE return` means: after the wrapper has copied the `return` slot into its owned MFBASIC result, pass the **original** native pointer to the named deallocator. The nested `SYMBOL`/`ABI` declare that deallocator — exactly one pointer parameter and a `CVoid` return. The freed slot is the produced pointer: the C `return`, or a named `OUT` slot. The deallocator runs **once, after the copy, on the success path only**; if the wrapper fails before the value is produced (a failed `SUCCESS_ON` gate, a marshaling error), nothing is freed. A NULL produced pointer is passed to the deallocator unchanged, because deallocators such as `sqlite3_free` define NULL as a no-op. The original pointer is never surfaced as an MFBASIC value, so `FREE` is the only sanctioned way to release a caller-owned native return — a raw `CPtr` cannot be handed back to source code to free by hand (`NATIVE_CPTR_ESCAPE`). A binding with more than one caller-owned pointer (for example several `OUT` buffers) states one `FREE` block per slot.

Rules:

- `LINK` names and all declared `SYMBOL` names are resolved before `main` starts. Native libraries are not lazy-loaded.
- If a required native library or symbol cannot be loaded before `main`, the program terminates before entering `main`. The diagnostic is written to stderr and the process exits with `55000001` (`ErrLinkFailed`). This startup failure is outside the error/`TRAP` model because no MFBASIC function is running yet.
- Linked names occupy a package-like namespace. A package-qualified name such as `sqlite::open` follows the same two-part rule as package access.
- A native call may resolve only the symbols declared by `SYMBOL` entries in the binding package. Dynamic lookup by source strings or computed names is not available to ordinary MFBASIC code.
- Native functions expose ordinary MFBASIC signatures. At call sites they auto-unwrap, auto-propagate, and participate in `MATCH` like any other fallible function.
- Native functions may accept and return MFBASIC primitive values, strings, byte lists, and declared resource types through an explicit `ABI` mapping. Other conversions are implementation-defined unless specified by the binding.
- If a native function has more than one `OUT` parameter and its MFBASIC return type is not `Nothing`, it must declare `RETURN_OUT`.
- A `FREE` block must name a `CPtr`-typed produced slot — the `return` slot or a declared `OUT` slot — and its deallocator must declare exactly one pointer parameter and a `CVoid` return. The deallocator is called once on the success path, after the produced value is copied into the wrapper's owned MFBASIC result, with the original (possibly NULL) native pointer; it is not called on a failed call. Without a `FREE` block a `CPtr` result is copied and the source pointer is left untouched (copy-and-leave), which leaks a caller-owned buffer — `FREE` is the only way to release one.
- `RESOURCE` is a declaration form for concrete opaque unique-handle types; it is not an inheritance base type and cannot be used as a generic catch-all type.
- Native resource ownership is declared at package scope with `RESOURCE <Name> CLOSE BY <closeFn>`. Raw C ABI types (`CPtr`, `CString`, `CInt32`, …) may appear only inside `ABI (...)` slots, never in a wrapper's MFBASIC-facing signature; a `CPtr` exists solely as the hidden representation of a declared resource and must not escape into an ordinary API (`NATIVE_CPTR_ESCAPE`).
- `REF` and `OUT` native pointer values are temporary call-frame values. Native code must not retain them after return; if a binding needs retained native storage, it must model that storage as a declared `RESOURCE`.
- Native `LINK` resources slot into the resource model of §15 unchanged: bound with `RES`, borrowed at ordinary calls, auto-closed by lexical drop through the registered close op, never copied/stored/field-accessed, and thread-sendable only with `THREAD_SENDABLE`. Diagnostics specific to native bindings are listed in `specifications/error_codes.md` (`1-102-0008`…`0009`, `2-203-0089`…`0098`).
- Native libraries are platform-specific dependencies. A `.mfp` package may declare that it needs a native library, including version, search policy, platform constraints, and content/hash requirements, but the native library itself is not portable binary representation.

**Example:**

```

EXPORT RESOURCE Db CLOSE BY sqliteLink::close
RESOURCE Stmt CLOSE BY sqliteLink::finalize

LINK "sqlite3" AS sqliteLink
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, return OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC openV2(path AS String, flags AS Integer) AS RES Db
    SYMBOL "sqlite3_open_v2"
    ABI (path CString, return OUT CPtr, flags CInt32, zVfs CPtr) AS status CInt32
    CONST zVfs = NOTHING         ' NULL: use the default VFS
    SUCCESS_ON status = 0
  END FUNC

  FUNC close(RES db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC busyTimeout(RES db AS Db, ms AS Integer) AS Nothing
    SYMBOL "sqlite3_busy_timeout"
    ABI (db CPtr, ms CInt32) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC exec(RES db AS Db, sql AS String) AS Nothing
    SYMBOL "sqlite3_exec"
    ABI (db CPtr, sql CString, callback CPtr, arg CPtr, errmsg CPtr) AS status CInt32
    CONST callback = NOTHING     ' NULL: no per-row callback
    CONST arg = NOTHING          ' NULL: no callback argument
    CONST errmsg = NOTHING       ' NULL: report failures through status, not a buffer
    SUCCESS_ON status = 0
  END FUNC

  FUNC prepare(RES db AS Db, sql AS String) AS RES Stmt
    SYMBOL "sqlite3_prepare_v2"
    ABI (db CPtr, sql CString, nByte CInt32, return OUT CPtr, pzTail CPtr) AS status CInt32
    CONST nByte = -1             ' read sql up to the terminating NUL
    CONST pzTail = NOTHING       ' NULL: discard the trailing-SQL pointer
    SUCCESS_ON status = 0
  END FUNC

  FUNC bindText(RES stmt AS Stmt, iCol AS Integer, zVal AS String) AS Nothing
    SYMBOL "sqlite3_bind_text"
    ABI (stmt CPtr, iCol CInt32, zVal CString, nByte CInt32, destructor CPtr) AS status CInt32
    CONST nByte = -1             ' bind up to the terminating NUL
    CONST destructor = -1        ' SQLITE_TRANSIENT (void*)-1: copy bytes now, do not alias
    SUCCESS_ON status = 0
  END FUNC

  FUNC bindParameterIndex(RES stmt AS Stmt, name AS String) AS Integer
    SYMBOL "sqlite3_bind_parameter_index"
    ABI (stmt CPtr, name CString) AS return CInt32
  END FUNC

  FUNC step(RES stmt AS Stmt) AS Boolean
    SYMBOL "sqlite3_step"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 100 OR status = 101
    RESULT status = 100
  END FUNC

  FUNC columnText(RES stmt AS Stmt, col AS Integer) AS String
    SYMBOL "sqlite3_column_text"
    ABI (stmt CPtr, col CInt32) AS return CPtr
  END FUNC

  FUNC columnType(RES stmt AS Stmt, col AS Integer) AS Integer
    SYMBOL "sqlite3_column_type"
    ABI (stmt CPtr, col CInt32) AS return CInt32
  END FUNC

  FUNC columnInt(RES stmt AS Stmt, col AS Integer) AS Integer
    SYMBOL "sqlite3_column_int64"
    ABI (stmt CPtr, col CInt32) AS return CInt64
  END FUNC

  FUNC columnDouble(RES stmt AS Stmt, col AS Integer) AS Float
    SYMBOL "sqlite3_column_double"
    ABI (stmt CPtr, col CInt32) AS return CDouble
  END FUNC

  FUNC columnCount(RES stmt AS Stmt) AS Integer
    SYMBOL "sqlite3_column_count"
    ABI (stmt CPtr) AS return CInt32
  END FUNC

  FUNC finalize(RES stmt AS Stmt) AS Nothing
    SYMBOL "sqlite3_finalize"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC expandedSql(RES stmt AS Stmt) AS String
    SYMBOL "sqlite3_expanded_sql"
    ABI (stmt CPtr) AS return CPtr
    FREE return
      SYMBOL "sqlite3_free"
      ABI (return CPtr) AS CVoid
    END FREE
  END FUNC

  FUNC errmsg(RES db AS Db) AS String
    SYMBOL "sqlite3_errmsg"
    ABI (db CPtr) AS return CPtr
  END FUNC

  FUNC extendedErrcode(RES db AS Db) AS Integer
    SYMBOL "sqlite3_extended_errcode"
    ABI (db CPtr) AS return CInt32
  END FUNC

  FUNC errstr(code AS Integer) AS String
    SYMBOL "sqlite3_errstr"
    ABI (code CInt32) AS return CPtr
  END FUNC
END LINK
```

---

## 18. Built-in Functions

Terminal and standard-stream I/O: `io::print`, `io::write`, `io::printError`, `io::writeError`, `io::flush`, `io::flushError`, `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, `io::isInputTerminal`, `io::isOutputTerminal`, `io::isErrorTerminal`.
Structured terminal / TUI control (`term::`, see `specifications/plan-01-term.md`): `term::on`, `term::off`, `term::isOn`, `term::setForeground`, `term::setBackground`, `term::setBold`, `term::setUnderline`, `term::showCursor`, `term::hideCursor`, `term::clear`, `term::moveTo`, `term::getForeground`, `term::getBackground`, `term::getBold`, `term::getUnderline`, `term::terminalSize`.
Filesystem and file I/O: `fs::fileExists`, `fs::directoryExists`, `fs::exists`, `fs::readBytes`, `fs::readText`, `fs::writeBytes`, `fs::writeText`, `fs::writeBytesAtomic`, `fs::writeTextAtomic`, `fs::appendBytes`, `fs::appendText`, `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::createTempFile`, `fs::tempDirectory`, `fs::readLine`, `fs::readAll`, `fs::readAllBytes`, `fs::writeAll`, `fs::writeAllBytes`, `fs::close`, `fs::eof`, `fs::canonicalPath`, `fs::isWithin`, `fs::pathJoin`, `fs::pathDirName`, `fs::pathBaseName`, `fs::pathExtension`, `fs::pathNormalize`, `fs::deleteFile`, `fs::createDirectory`, `fs::createDirectories`, `fs::deleteDirectory`, `fs::listDirectory`, `fs::currentDirectory`, `fs::setCurrentDirectory`.
Network: `net::lookup`, `net::connectTcp`, `net::listenTcp`, `net::accept`, `net::bindUdp`, `net::receiveFrom`, `net::receiveTextFrom`, `net::sendTo`, `net::sendTextTo`, `net::poll`, `net::read`, `net::readText`, `net::write`, `net::writeText`, `net::close`, `net::localAddress`, `net::remoteAddress`, `net::setReadTimeout`, `net::setWriteTimeout`, `tls::connect`, `tls::wrap`, `tls::close`.
Strings: `len`, `find`, `mid`, `replace`, `strings::trim`, `strings::trimStart`, `strings::trimEnd`, `strings::upper`, `strings::lower`, `strings::caseFold`, `strings::normalizeNfc`, `strings::graphemes`, `strings::startsWith`, `strings::endsWith`, `strings::contains`, `strings::split`, `strings::join`, `strings::byteLen`, `toString`, `toInt`, `toFloat`, `toFixed`, `toByte`, `isNumeric`, `&`.
Regex: `regex::match`, `regex::find`, `regex::replace`.
Collections: `forEach`, `transform`, `filter`, `reduce`, `sum`, `get`, `getOr`, `find`, `mid`, `replace`, `set`, `append`, `prepend`, `insert`, `removeAt`, `removeKey`, `keys`, `values`, `hasKey`, `contains`, `len`.
Threads: `thread::start`, `thread::isRunning`, `thread::waitFor`, `thread::cancel`, `thread::send`, `thread::poll`, `thread::receive`, `thread::isCancelled`.
Math: `math::pi`, `math::piFixed`, `math::e`, `math::eFixed`, `math::abs`, `math::min`, `math::max`, `math::clamp`, `math::floor`, `math::ceil`, `math::round`, `math::sqrt`, `math::pow`, `math::exp`, `math::log`, `math::log10`, `math::sin`, `math::cos`, `math::tan`, `math::asin`, `math::acos`, `math::atan`, `math::atan2`.
JSON: `json::parse`, `json::stringify`, `json::get`, `json::getOr`.
Error codes: `errorCode::ErrInvalidArgument`, `errorCode::ErrNotFound`, and the other constants listed in the built-in error-code registry.

Fallible built-ins (`fs::openFile`, `toInt`, `get`, …) can fail and auto-propagate like any call.

---

## 19. Grammar (EBNF, abridged)

```ebnf
program        = { import | linkDecl } { declaration } ;

import         = "IMPORT" ident [ "AS" ident ] ;
qualifiedName  = ident "::" ident ;
resourceDecl   = declVis "RESOURCE" ident "CLOSE" "BY" qualifiedName
                   [ "THREAD_SENDABLE" ] ;
funcAlias      = declVis "FUNC" ident "AS" qualifiedName ;
linkDecl       = "LINK" string "AS" ident { nativeFuncDecl } "END" "LINK" ;
nativeFuncDecl = "FUNC" ident "(" [ params ] ")" [ "AS" [ "RES" ] type ]
                   nativeFuncBody "END" "FUNC" ;
nativeFuncBody = "SYMBOL" string
                   "ABI" "(" [ abiSlotList ] ")" "AS" abiSlot
                   { constPin }
                   [ nativeReturnRule ]
                   [ "RESULT" expr ]
                   [ returnOut ]
                   { nativeFree } ;
constPin       = "CONST" ident "=" expr ;
nativeReturnRule = "SUCCESS_ON" expr | "ERROR_ON" expr ;
returnOut       = "RETURN_OUT" ident "[" ident { "," ident } "]" ;
nativeFree     = "FREE" ( ident | "return" )
                   "SYMBOL" string
                   "ABI" "(" abiSlot ")" "AS" nativeType
                   "END" "FREE" ;
abiSlotList    = abiSlot { "," abiSlot } ;
abiSlot        = ( ident | "return" ) [ "OUT" ] nativeType ;
nativeType     = "CInt8" | "CInt16" | "CInt32" | "CInt64"
                | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64"
                | "CBool" | "CFloat32" | "CFloat64"
                | "CIntPtr" | "CUIntPtr" | "CSize"
                | "CString" | "CPtr" | "CVoid" ;

declaration    = topLetDecl | topMutDecl
               | funcDecl | subDecl | typeDecl | unionDecl | enumDecl
               | resourceDecl | funcAlias | linkDecl ;

declVis        = [ "EXPORT" | "PACKAGE" | "PRIVATE" ] ;
funcIso        = [ "ISOLATED" ] ;

topLetDecl     = declVis "LET" ident [ "AS" type ] "=" expr ;
topMutDecl     = declVis "MUT" ident [ "AS" type ] [ "=" expr ] ;

funcDecl       = declVis funcIso "FUNC" ident [ templateParams ] "(" [ params ] ")" "AS" type
                   block [ trap ] "END" "FUNC" ;
subDecl        = declVis "SUB" ident [ templateParams ] "(" [ params ] ")"
                   block [ trap ] "END" "SUB" ;
trap           = "TRAP" ident block "END" "TRAP" ;

templateParams = "OF" ident { "," ident } ;
params         = param { "," param } ;
param          = ident "AS" type [ "=" expr ] ;
type           = templateType | funcType | ident | qualifiedIdent ;
typeList       = type { "," type } ;
templateType
               = "Map" "OF" type "TO" type
               | "Thread" "OF" type "TO" type
               | (ident | qualifiedIdent) "OF" type { "," type } ;
funcType       = [ "ISOLATED" ] "FUNC" "(" [ typeList ] ")" "AS" type ;

typeDecl       = declVis "TYPE" ident [ templateParams ] { field } "END" "TYPE" ;
field          = declVis ident "AS" type ;
unionDecl      = declVis "UNION" ident [ templateParams ] [ unionIncludes ] { unionMember } "END" "UNION" ;
unionIncludes  = "INCLUDES" unionName { "," unionName } ;
unionName      = ident | qualifiedIdent ;
unionMember    = ident | qualifiedIdent ;
enumDecl       = declVis "ENUM" ident identlist "END" "ENUM" ;
identlist      = ident { "," ident } ;

block          = { statement } ;
statement      = letStmt | mutStmt | assignStmt
               | ifStmt | forStmt | foreachStmt | whileStmt
               | doStmt | matchStmt
               | failStmt | propagateStmt | returnStmt
               | exprStmt | "REM" ... ;

letStmt        = "LET" ident [ "AS" type ] "=" expr ;
mutStmt        = "MUT" ident [ "AS" type ] [ "=" expr ] ;
assignStmt     = ident "=" expr ;

(* Semantic rule: MUT without an initializer requires an explicit type
   with a defined default value. *)

ifStmt         = inlineIfStmt | blockIfStmt ;
inlineIfStmt   = "IF" expr "THEN" simpleStmt [ "ELSE" simpleStmt ] ;
blockIfStmt    = "IF" expr "THEN" block
                   { "ELSEIF" expr "THEN" block }
                   [ "ELSE" block ]
                   "END" "IF" ;
simpleStmt     = letStmt | mutStmt | assignStmt | failStmt | propagateStmt
               | returnStmt | exitStmt | continueStmt | exprStmt ;
forStmt        = "FOR" ident "=" expr "TO" expr [ "STEP" expr ]
                   block "NEXT" ;
foreachStmt    = "FOR" "EACH" ident "IN" expr block "NEXT" ;
whileStmt      = "WHILE" expr block "WEND" ;
doStmt         = "DO" block "LOOP" "UNTIL" expr
               | "DO" "WHILE" expr block "LOOP" ;

failStmt       = "FAIL" expr ;
propagateStmt  = "PROPAGATE" ;
returnStmt     = "RETURN" [ expr ] ;
exitStmt       = "EXIT" loopKind | "EXIT" "SUB" | "EXIT" "FUNC"
               | "EXIT" "PROGRAM" expr ;
continueStmt   = "CONTINUE" loopKind ;
loopKind       = "FOR" | "DO" | "WHILE" ;
exprStmt       = expr ;

matchStmt      = "MATCH" expr { caseClause } "END" "MATCH" ;
caseClause     = "CASE" patternList [ "WHEN" expr ] ":" block
               | "CASE" "ELSE" ":" block ;
patternList    = pattern { "," pattern } ;
pattern        = enumMember | unionPattern | literal ;
unionPattern   = (ident | qualifiedIdent) "(" ident ")" ;

expr           = orExpr { "|>" pipeTail } ;
pipeTail       = (ident | qualifiedIdent) "(" [ pipeArgList ] ")" ;
pipeArgList    = pipeArg { "," pipeArg } ;
pipeArg        = [ ident ":=" ] ( expr | "_" ) ;
orExpr         = andExpr { ("OR" | "XOR") andExpr } ;
andExpr        = notExpr { "AND" notExpr } ;
notExpr        = [ "NOT" ] cmpExpr ;
cmpExpr        = addExpr { cmpOp addExpr } ;
cmpOp          = "=" | "<>" | "<" | ">" | "<=" | ">=" ;
addExpr        = mulExpr { ("+"|"-"|"&") mulExpr } ;
mulExpr        = powExpr { ("*"|"/"|"DIV"|"MOD") powExpr } ;
powExpr        = unary [ "^" powExpr ] ;       (* right-associative *)
unary          = [ "-" ] fieldAccess ;
fieldAccess    = primary { "." ident } ;
primary        = literal | ident | qualifiedIdent | call | lambda
               | enumMember | constructor | withExpr | listLit | mapLit
               | "(" expr ")" ;
literal        = integer | decimal | string | "TRUE" | "FALSE" | "NOTHING" ;

qualifiedIdent = ident "::" ident ;         (* package::identifier only *)
enumMember     = ident "." ident ;         (* EnumType.Member *)
                                                (* Name resolution disambiguates
                                                   ident.ident: type name on
                                                   left => enum member; value on
                                                   left => field access. *)
call           = (ident | qualifiedIdent) "(" [ callArgList ] ")" ;
callArgList    = callArg { "," callArg } ;
callArg        = [ ident ":=" ] expr ;
lambda         = "LAMBDA" "(" [ params ] ")" "->" expr ;
withExpr       = "WITH" expr "{" fieldAssigns "}" ;
fieldAssigns   = fieldAssign { "," fieldAssign } ;
fieldAssign    = ident ":=" expr ;
constructor    = (ident | qualifiedIdent) "[" [ callArgList ] "]" ;
listLit        = "[" [ exprList ] "]" ;
exprList       = expr { "," expr } ;
mapLit         = "Map" "OF" type "TO" type "{" [ mapEntries ] "}" ;
mapEntries     = mapEntry { "," mapEntry } ;
mapEntry       = expr ":=" expr ;
```

---

## 20. Worked Example

```basic
IMPORT io
IMPORT strings

TYPE Vec3
  x AS Float
  y AS Float
  z AS Float
END TYPE

FUNC parseLine(line AS String) AS Vec3
  LET parts = strings::split(line, ",")
  IF len(parts) <> 3 THEN FAIL error(77050002, "expected 3 fields")

  LET x = toFloat(strings::trim(get(parts, 0)))   ' auto-propagates on failure
  LET y = toFloat(strings::trim(get(parts, 1)))
  LET z = toFloat(strings::trim(get(parts, 2)))
  RETURN Vec3[x, y, z]
END FUNC

FUNC loadPoints(path AS String) AS List OF Vec3
  MUT pts AS List OF Vec3 = []
  RES f = fs::openFile(path)                ' auto-propagates on failure
  WHILE NOT fs::eof(f)
    LET v = parseLine(fs::readLine(f))      ' auto-propagates to TRAP below on bad input
    pts = append(pts, v)                   ' optimized in place for MUT
  WEND
  RETURN pts                               ' f closed by lexical drop here; pts freezes automatically

  TRAP(err)
    io::print("Load failed: " & err.message)
    RETURN []                              ' use empty list as the function result
  END TRAP
END FUNC

SUB main()
  LET pts   = loadPoints("data.csv")
  io::print("Loaded " & toString(len(pts)) & " points")
  LET total = pts |> transform(_, LAMBDA(p) -> p.x) |> sum(_)
  io::print("Sum of x: " & toString(total))
  RETURN

  TRAP(err)
    io::print("Fatal: " & err.message)   ' otherwise exits with err.code
    RETURN
  END TRAP
END SUB
```

### 20.1 Package Layering Example

Package layering lets one package define a larger closed domain from another package's union without subtyping or open dispatch.

```basic
' shape/shapes.mfb
IMPORT math

TYPE Circle
  radius AS Float
END TYPE

TYPE Rect
  w AS Float
  h AS Float
END TYPE

UNION Shape
  Circle
  Rect
END UNION

EXPORT FUNC area(s AS Shape) AS Float
  MATCH s
    CASE Circle(c)
      RETURN math::pi * c.radius * c.radius

    CASE Rect(r)
      RETURN r.w * r.h
  END MATCH
END FUNC
```

```basic
' extraShape/shapes.mfb
IMPORT math
IMPORT shape

TYPE Triangle
  a AS Float
  b AS Float
  c AS Float
END TYPE

UNION ExtraShape INCLUDES shape::Shape
  Triangle
END UNION

EXPORT FUNC area(s AS ExtraShape) AS Float
  MATCH s
    CASE Triangle(t)
      RETURN triangleArea(t.a, t.b, t.c)

    CASE ELSE
      FAIL error(77050004, "shape member not handled")
  END MATCH
END FUNC

PRIVATE FUNC triangleArea(a AS Float, b AS Float, c AS Float) AS Float
  LET p = (a + b + c) / 2.0
  RETURN math::sqrt(p * (p - a) * (p - b) * (p - c))
END FUNC
```

Users import the larger package when they want the larger domain:

```basic
IMPORT extraShape
IMPORT io

SUB main()
  LET s AS extraShape::ExtraShape = extraShape::Circle[radius := 10.0]
  LET t AS extraShape::ExtraShape = extraShape::Triangle[a := 3.0, b := 4.0, c := 5.0]

  io::print(toString(extraShape::area(s)))
  io::print(toString(extraShape::area(t)))
END SUB
```

The resulting model is `extraShape = shape + extraShape additions`, but the type system remains closed and explicit: `shape::Shape` is not a base class, `ExtraShape` is not a subtype, there is no virtual dispatch, and no package can retroactively add member types to another package's union.

---

## 21. Build Artifacts

MFBASIC uses source files for authoring, portable binary representation packages, and native binaries for executables.

| Artifact | Extension | Purpose |
|----------|-----------|---------|
| Source file | `.mfb` | Human-authored source code. The `.mfb` files selected by a project's `project.json` together form that project's source package (§13). |
| Package | `.mfp` | Architecture-neutral binary representation package. Its payload is **structured Binary Representation** (a faithful, versioned serialization of the compiler's IR) plus an embedded package manifest, public API metadata, dependency metadata, and optional native-link metadata. A compiled package can be built on one platform and imported on any platform that supports the same MFB binary representation/package version. |
| Executable | platform-native | Final application binary for the target OS/CPU. Executables compile application code plus imported `.mfp` packages to native code. |

The backend pipeline is:

```text
.mfb source
  -> IR (typed, structured program representation)
  -> Binary Representation (.mfp package)
       or
  -> NIR -> native executable
```

A package build stops at Binary Representation: `IR -> Binary Representation (.mfp)`, a faithful structured serialization with no flattening or structure loss. An executable build lowers the project's own IR through `IR -> NIR -> native`. Consuming a package **decodes** its Binary Representation back into IR functions, merges them into the project IR, and lowers everything through that same single `IR -> NIR -> native` path — there is no separate package binary representation-to-native bridge.

Package compilation emits `.mfp` packages containing portable Binary Representation plus the embedded package manifest, dependency metadata, native-link metadata, and public API metadata needed for import, type checking, IR merging, and verification. This metadata includes each exported type and function's ownership properties: copyability, movability, resource-handle status, closure-capture requirements, thread-sendability, drop requirements, and collection element constraints. A package containing `LINK` declarations emits a reusable native binding `.mfp`: importers consume the package API and do not repeat the `LINK` declarations.

Executable compilation consumes `.mfb` application source, the resolved `mfb.lock`, and imported `.mfp` packages. The compiler decodes each imported package's Binary Representation and merges its IR functions into the project IR under the package's identity prefix, resolving package-qualified MFBASIC calls to functions in the merged IR. After the IR merge, the native backend lowers everything through `IR -> NIR -> native`, resolves all native dependencies declared by the merged packages, performs target OS/native linking as needed, and emits a native binary for the selected target platform.

### 21.1 `.mfp` Binary Representation verification

Every `.mfp` package is verified before its Binary Representation can be decoded, merged into the project IR, or lowered. Verification operates on the **decoded IR**, not a flat opcode stream, and is deterministic: it must reject malformed packages before any package code runs. Because the Binary Representation is structured (nested regions with explicit ends), structure is explicit — there is no control-flow graph to reconstruct and no "jump into a trap or cleanup region" to reject — and most invariants reuse the compiler's existing IR-level passes.

The verifier must check:

- Package metadata is well-formed, uses a supported binary representation/package (Binary Representation) version, satisfies the resolved manifest and lockfile entries, and matches the Binary Representation body.
- The package signature, hash, or trust record is valid when the build mode requires signed or locked dependencies.
- Public API metadata is consistent with the IR definitions, including exported names, type shapes, function signatures, ownership properties, and native-link declarations.
- Every IR node is type-correct. Operand types, result types, call signatures, record fields, union member types, collection element types, and `Result` handling (`CallResult`, `ResultIsOk`/`ResultValue`/`ResultError`) must match the typed metadata.
- Every binding, local, and result value is definitely defined before read and is not read after move.
- Resource ownership is linear. A resource handle has one owner, is not copied, is not stored in ordinary collections, is sent to threads only when its concrete type is thread-sendable, and is closed or moved exactly once on every control-flow path.
- Drop and cleanup paths are valid. The verifier rejects double-drop, missing-drop, and use-after-drop paths. Because resource regions are nested in the IR, every exit path is bounded by the region's end.
- Every `MATCH` is exhaustive (covers every value or has an `ELSE`).
- All normal and error paths satisfy the function's declared return type and `Result` behavior (declared return/effect agreement).
- There is at most one function-level bottom `TRAP`; error routing uses the structured `Result`/`TRAP`/`FAIL`/`PROPAGATE` form, never exception-like unwinding.
- Native-link manifests are valid: every linked library and symbol referenced by the IR is declared in metadata, every resource close function exists and has the correct resource-consuming signature, and every ABI mapping uses supported native types.

Verification failure rejects the package with a toolchain diagnostic. It is not recoverable by program `TRAP` code because no package code has started running.

A future VM is not foreclosed: it would either interpret the structured, typed Binary Representation directly or lower it through the same `IR -> NIR -> native` path. The artifact contract remains: packages are portable `.mfp` Binary Representation packages; executables are native platform binaries.

---

## 22. Tooling And Auditability

The compiler and language server must make fallible control flow visible even though ordinary calls auto-unwrap and auto-propagate.

Required diagnostics and tooling metadata:

- Mark every fallible call site in editor diagnostics or semantic tokens, including calls hidden inside expressions and argument lists.
- Show each auto-propagation edge from a fallible call to the enclosing `TRAP` or function return.
- Show each `TRAP` recovery path, including whether it `RETURN`s, `PROPAGATE`s, or replaces the error with `FAIL`.
- Report scopes that hold live resource bindings across fallible calls, and surface the lexical drop-close edges that release those resources on each exit path, together with the drop-close failure metadata rule from §15.
- Surface all native binding packages, linked native libraries, declared symbols, ABI mappings, and native resource close functions used by a build.
- Surface package permissions and host capabilities when a standard or native package requires filesystem, network, process, environment, clock, randomness, or native-library access.
- Lint dense or security-sensitive code for confusing identifier similarity. In the current ASCII-only identifier set this includes case-only near-collisions; if non-ASCII identifiers are ever enabled, it also includes Unicode normalization, case-fold, script-mixing, and confusable-character collisions.
- Include fallible-call, propagation, `TRAP`, permission, native-link, and resource-cleanup metadata in `.mfp` packages when exported APIs contain or expose those behaviors.

The toolchain must provide an audit command:

```text
mfb audit [--format text|json] [--locked] [path]
```

`mfb audit` reports fallible call sites, auto-propagation paths, `TRAP` recovery paths, resource cleanup behavior, native links, package permissions, dependency versions, lockfile mismatches, and verifier status. `--locked` requires the resolved dependency graph to match `mfb.lock`.

Additional required tooling commands:

```text
mfb fmt [--check] [path]
mfb test [--filter pattern] [--locked] [path]
mfb lsp
```

`mfb fmt` applies the standard formatter. `--check` exits with a toolchain diagnostic when formatting would change files.

`mfb test` discovers exported or private zero-argument `SUB` declarations whose names start with `test` in files included by the `project.json` test source entries. A test succeeds when it completes without failing and fails when it produces an error. Test builds use the same package resolver, verifier, resource rules, and audit metadata as executable builds.

`mfb lsp` starts the language-server protocol implementation. It must expose diagnostics for fallible calls, auto-propagation paths, `TRAP` recovery, resource moves/use-after-move, unsafe or invalid native links, permissions, package-version conflicts, lockfile mismatches, dense security-sensitive lines, and identifier near-collisions.
