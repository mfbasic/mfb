# ⟪MFBASIC⟫ — Language Specification

## Modern Functional Basic (MFB)

A modern, functional dialect of BASIC. Immutable by default, no objects, package-level imports, and a single-trap error model. Every function returns a `Result` that **auto-unwraps on success and auto-propagates on error** — no `TRY`, no `GOTO`, no exceptions. The language is designed for memory-safe implementation through owned values, explicit resource ownership, and lexical cleanup.

---

## 1. Design Principles

1. **Readable over terse** — English keywords, `END X` blocks, line-oriented.
2. **Functional, no OOP** — plain data (records/unions) + free functions. No classes, methods, `self`, or inheritance.
3. **Immutable by default** — `LET` binds, `MUT` opts into reassignment. No implicit globals, no hidden aliasing.
4. **Optional ceremony** — a 3-line script needs no module header; structure exists when you want it.
5. **Errors as values, invisibly plumbed** — every function returns `Result`; success auto-unwraps, errors auto-route to a single `TRAP` or propagate. No exceptions, no unwinding.
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

The `:` separator is legal, but formatters and language servers should lint dense security-sensitive lines, especially lines that combine fallible calls, resource operations, native calls, permissioned filesystem/network operations, or `TRAP` control flow.

Identifiers are case-sensitive, so `userId` and `userid` are distinct. Tooling should lint near-collisions that differ only by case or visually minor spelling differences within the same scope or imported namespace.

---

## 3. Templates

MFBASIC supports monomorphized templates, not runtime generics.

Template parameters may appear only on `TYPE`, `UNION`, `FUNC`, and `SUB` declarations. A template is not a runtime entity and is not emitted to bytecode as an open declaration. Every used instantiation is resolved during compilation into a concrete declaration before IR, bytecode, package metadata, or native lowering is produced.

Built-in type constructors such as `List`, `Map`, `Result`, and `Thread` are compiler-owned templates. User code may define templates with the same `OF` syntax where allowed:

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

Exported templates in source packages are instantiated by the importing compilation before bytecode is produced. A compiled `.mfp` package contains only concrete template instantiations; it does not expose templates for later instantiation unless a future package format explicitly adds signed template metadata.

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

`Fixed` is a binary fixed-point number stored as a signed 32-bit integer part and a 32-bit fractional part. Its range is approximately `-2147483648.0` through `2147483647.9999999998`, with a resolution of `1 / 2^32`. Fixed-point arithmetic is deterministic across targets, but it is not exact decimal currency arithmetic because most decimal fractions are rounded to binary fixed-point values. Overflow produces an error result carrying `Error[77050010, ...]`; divide-by-zero and invalid numeric domains produce an error result carrying `Error[77050002, ...]`.

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
      FAIL Error[77050004, "shape member not handled"]
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

### 4.4 `Result`, `Error`, and absence (built in)

The error model is built on two built-in public types and one compiler-owned private success member type:

```basic
TYPE Error
  code    AS Integer
  message AS String
END TYPE
```

`Result OF T` is compiler-owned notation for a built-in union with two members:

- private compiler-owned `Ok OF T`
- public `Error`

Users may construct `Error[...]`, but may not construct `Result` or `Ok` directly. `Result` concrete instantiations are produced by the compiler.

- **Every function returns `Result OF T`** where `T` is the declared return type.
- The error member is **always** the single public `Error` type — no per-function error types, no coercion.
- There is no built-in `Option`/`Maybe`. Absence is represented by `Error[code, message]` carried through `Result`; use semantic error-code constants such as `errorCode::ErrNotFound` for not found.
- `Result` is rarely written by hand; it is produced and consumed implicitly (see §8).

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

`Nothing` is the unit type. It has one value, written `NOTHING`, and is the success value returned by `SUB` (see §7).

```basic
SUB log(msg AS String)
  IF msg = "" THEN RETURN       ' Ok(NOTHING)
  io::print(msg)
END SUB
```

### 4.7 Collections

```basic
List OF T                          ' owned sequence
Map OF K TO V                      ' owned map
MapEntry OF K TO V                 ' map iteration entry
```

`List`, `Map`, and `MapEntry` are built-in templates. Each concrete use, such as `List OF Integer`, `Map OF String TO Float`, or `MapEntry OF String TO Float`, is monomorphized before bytecode generation. There is one sequence type, `List`. There are no fixed-size arrays and no `DIM`. See §12.

`MapEntry OF K TO V` is the compiler-owned record shape used when iterating a map. It has public read-only fields `key AS K` and `value AS V`.

Runtime collection storage is specified in `specifications/memory_layouts.md`.

### 4.8 Threads

```basic
Thread OF Msg TO Out                 ' isolated running or completed thread
ThreadWorker OF Msg TO Out           ' worker-side view of the same thread
```

`Thread` and `ThreadWorker` are built-in templates for opaque handles to the same underlying package worker. `Thread` is the parent-side handle. `ThreadWorker` is the worker-side handle passed into the thread entry function. `Msg` is the message type used by `thread::send` and `thread::receive`; `Out` is the thread entry function's success type. A completed parent `Thread` exposes `result AS Result OF Out` for field access as `t.result`; retrieving that result consumes and closes the parent `Thread` handle.

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

Defaultability is recursive and finite: nested lists, maps, and records are defaultable only when every transitively referenced element, key, value, and field type is also defaultable, and recursive record cycles (legal only through `List`, `Map`, or `UNION`; see §4.2) do not define a default value. Enums, unions, functions, lambdas, `Result`, threads, and resource handles do not have default values. A `MUT` binding of one of those types must have an initializer.

### 4.11 Comparable Types

Some standard functions require a type to be comparable. Comparable types are `Integer`, `Float`, `Fixed`, `Boolean`, `String`, `Byte`, `Nothing`, enum types, and records whose fields are all comparable. `List`, `Map`, unions, functions, lambdas, threads, and resource handles are not comparable.

`Map` keys must be comparable. List helpers such as `find`, `contains`, and `replace` require comparable element types.

Equality operators `=` and `<>` require either numeric operands or any two compatible comparable operands. Ordering operators `<`, `>`, `<=`, and `>=` remain numeric-only.

---

## 5. Bindings & Scope

Only two binding forms:

- **`LET`** — immutable binding.
- **`MUT`** — reassignable binding.

```basic
LET x = 10
MUT total AS Float = 0.0
total = total + 1         ' OK
' x = 5                   ' ERROR: x is immutable
```

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

- **Every function returns `Result`.** `FUNC F(...) AS T` has effective type `Result OF T`. A `SUB` returns `Result OF Nothing` (see §7).
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

A `SUB` is the effect-only spelling of a function whose success type is `Nothing`.

```basic
SUB logItem(x AS Integer)
  io::print(toString(x))
END SUB
```

At the call boundary, it has effective type `Result OF Nothing`, with success represented as `Ok(NOTHING)`. A bare `RETURN`, `RETURN NOTHING`, and fall-through to `END SUB` all produce `Ok(NOTHING)` where fall-through is allowed.

For first-class function typing, a `SUB(A, B, ...)` is compatible with `FUNC(A, B, ...) AS Nothing`. This lets effect-only callbacks work without wrapper functions:

```basic
SUB printItem(x AS Integer)
  io::print(toString(x))
END SUB

forEach(nums, printItem)
```

`Nothing` is a normal concrete unit type, not a bottom type and not a non-returning marker. `Result OF Nothing` participates in auto-unwrapping, propagation, and direct `MATCH` handling exactly like any other `Result OF T`:

```basic
MATCH fs::writeAll(f, "done")
  CASE Ok(v)       : io::print("saved")
  CASE Error(e)    : io::print(e.message)
END MATCH
```

Value-producing callbacks still require a value-producing `FUNC`. A `SUB` is valid for APIs such as `forEach` that expect `FUNC(T) AS Nothing`; it is not valid for APIs such as `transform` that infer and collect a result value.

---

## 8. Error Model — Implicit `Result`, One `TRAP` per Function

### 8.1 The core rule

Every function returns `Result`. At a call site, a call **auto-unwraps** to its `Ok` value. If the call yields `Err`, control immediately transfers to the enclosing `TRAP`; if there is no `TRAP`, the function returns that `Err` to *its* caller.

```basic
LET x = toFloat(input)    ' if Ok(v): x = v
                            ' if Error(e): jump to TRAP, or return an error result carrying e
```

There is **no `TRY` keyword and no `GOTO`**. Propagation is the default behavior of calling a function.

Function arguments are evaluated left to right. If any argument expression returns `Err`, later arguments are not evaluated and the error routes to the enclosing `TRAP` or propagates to the caller.

When an error path leaves a scope, any live resource bindings in that scope are closed by lexical drop (§14.7, §15) before the final error reaches the enclosing `TRAP` or caller.

### 8.2 Entering the error path

Use `FAIL` to fail explicitly with an `Error`:

```basic
IF n < 0 THEN FAIL Error[77050002, "negative"]
```

`FAIL e` routes to the enclosing `TRAP`; with no trap, the function returns an error result carrying `e`.

### 8.3 The `TRAP` block

Each `FUNC`/`SUB` may declare **at most one** `TRAP`, at the bottom, after normal flow. The payload is always `Error`, so no type annotation is needed.

```basic
FUNC readAge(input AS String) AS Integer
  LET n = toInt(input)                 ' auto-propagates on Err
  IF n < 0 THEN FAIL Error[77050002, "negative"]
  RETURN n

  TRAP err
    io::print("Bad age: " & err.message)
    RETURN 0                           ' function succeeds with default
  END TRAP
END FUNC
```

Trap outcomes:

| Statement | Meaning | Produces |
|-----------|---------|----------|
| `RETURN v` | function succeeds | `Ok(v)` |
| `PROPAGATE` | re-propagate the current `err` | error result carrying `err` |
| `FAIL e2` | replace/wrap the error | error result carrying `e2` |

There is no trap resume operation. Once control enters a `TRAP`, the failed expression is abandoned. A trap may convert the error into the function's final success value with `RETURN`, rethrow the same error with `PROPAGATE`, or replace it with `FAIL`.

```basic
TRAP err
  PROPAGATE                            ' bubble the same error
END TRAP
```

```basic
TRAP err
  FAIL Error[77060001, "load failed: " & err.message]   ' wrap with context
END TRAP
```

### 8.4 Capturing a `Result` for local handling (`MATCH`)

To handle an error at the call site instead of auto-propagating, make the call the **direct scrutinee of a `MATCH`**. A matched call is *not* auto-unwrapped — you receive the `Result`.

```basic
MATCH fs::openFile(path)
  CASE Ok(f)     : LET line = fs::readLine(f)
  CASE Error(e)  : io::print("could not open: " & e.message)
END MATCH
```

This is the only way to intercept an error locally without a `TRAP`. Everywhere else, calls auto-propagate.

Use this same pattern for ordinary absence:

```basic
IMPORT errorCode

MATCH getUser(id)
  CASE Ok(user) : io::print("Found")
  CASE Error(e) WHEN e.code = errorCode::ErrNotFound : io::print("User does not exist")
  CASE Error(e) : FAIL e
END MATCH
```

### 8.5 `RETURN` semantics

`RETURN v` **always** means function success and produces the function's final `Ok(v)`, whether it appears in the body or in the `TRAP`. It does not resume at the failed expression. A bare `RETURN` in a `SUB` produces `Ok(NOTHING)`. `RETURN NOTHING` is also valid in a `SUB`. A `SUB` with no `TRAP` may fall through to `END SUB`, which implicitly returns `Ok(NOTHING)`. `RETURN` never produces an error. `FAIL` and `PROPAGATE` produce errors.

### 8.6 Rules

1. At most one `TRAP` per function, at the bottom, after normal flow.
2. The trap payload is always `Error`; written `TRAP err` with no type.
3. The trap block is reachable only via `FAIL` (in the body), an auto-propagated `Err` from a call, or `FAIL`/`PROPAGATE` inside the trap. It is never reached by fall-through.
4. `PROPAGATE` is valid only inside a `TRAP` (it refers to the current `err`). Elsewhere it is a compile error; use `FAIL e` instead.
5. With no `TRAP`, any `Err` (from `FAIL` or an auto-propagated call) becomes the function's returned `Err`.
6. Every `TRAP` path must end in `RETURN`, `PROPAGATE`, or `FAIL`. Trap fall-through is a compile error.
7. Every `FUNC` path must end in `RETURN value` or `FAIL error`. Function fall-through is a compile error.
8. A `SUB` with no `TRAP` may fall through to `END SUB`, implicitly returning `Ok(NOTHING)`.
9. A `SUB` with a `TRAP` must end every normal path before the `TRAP` with `RETURN`, `RETURN NOTHING`, or `FAIL error`. Falling through from the normal body into the `TRAP` is a compile error.
10. An executable entry point's uncaught `Err` terminates the process as an unhandled runtime error: the process exits with code `255`, and stderr receives `Code: <err.code> Message: <err.message>`. Give the entry point a `TRAP` for graceful handling.

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
| `SUB` returns `Ok(NOTHING)` | Exit code `0`. |
| `FUNC ... AS Integer` returns `Ok(n)` | Exit code `n`. Implementations must reject or fail values outside the host process exit-code range. |
| Entry returns an uncaught error result carrying `err` | Write `Code: <err.code> Message: <err.message>` to stderr and exit with code `255`. |

Environment access outside command-line arguments is outside the core language specification and may be provided by a future standard package.

### 8.8 Desugaring

```text
FUNC f(a AS A) AS T            =>   FUNC f(a AS A) AS Result OF T

  call g(x)        =>  MATCH g(x)
                         CASE Ok(v)    : v
                         CASE Error(e) : bind err = e ; jump __trap
                                       (no trap => RETURN error result carrying e)
                       END MATCH

  FAIL e           =>  bind err = e ; jump __trap
                       (no trap => RETURN error result carrying e)

  RETURN v         =>  RETURN Ok(v)          (body or trap)

  __trap:
    PROPAGATE      =>  RETURN error result carrying err
    FAIL e2        =>  RETURN error result carrying e2
    RETURN v       =>  RETURN Ok(v)
```

A call used as a `MATCH` scrutinee is **not** rewritten — the raw `Result` is matched. No real exceptions, no stack unwinding — pure value flow.

---

## 9. Pattern Matching

`MATCH` binds concrete union member values, matches raw `Result` calls, and matches literals; exhaustiveness is checked at compile time.

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
- If the scrutinee is a direct call expression, `MATCH` sees the raw `Result`, so `CASE Ok(v)` and `CASE Error(e)` match the `Result` members.
- `CASE ELSE` is the catch-all fallback.
- **Exhaustiveness**: unions must cover all member types. Open types (`Integer`, `String`, etc.) require a `CASE ELSE` or it is a compile error. Guarded `CASE` arms do not contribute to compile-time coverage because the guard can fail; use an unguarded arm or `CASE ELSE` to cover the remaining values.
- A call as the scrutinee captures its `Result` (see §8.4).

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

There is **no `GOTO`** and **no `SELECT CASE`** (use `MATCH`).

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
- Checked numeric failures from operators are ordinary `Err` results and therefore auto-propagate unless handled by `MATCH` or `TRAP`.
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

- `get` returns `Result` (missing key / out-of-range index yields an error result carrying `Error`) and therefore auto-propagates. Use `getOr(coll, key, default)` for the common defaulted read.
- A collection bound with `LET` is an immutable snapshot. Update functions such as `append` and `set` may read it and produce a new collection value, but assigning back to the same `LET` binding or otherwise modifying it is a compile-time error.
- A collection bound with `MUT` is a locally mutable buffer. When the result of an update function is assigned back to the same `MUT` binding, such as `pts = append(pts, v)`, the compiler performs the update destructively in place instead of allocating a replacement collection.
- Update helpers are semantically pure functions. `append(pts, v)` by itself computes and discards a result; it has no lasting effect unless the result is assigned, returned, passed, or otherwise consumed. Destructive update is an optimization only for the assignment-back-to-the-same-`MUT` pattern.
- Update functions on `MUT` collections preserve ownership semantics at boundaries: passing or returning the collection freezes it into an immutable owned value (§14).
- Containers own their contents. Adding a value to a collection stores an owned value in the collection, never a borrowed reference to an external binding.
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

A package may import a source package or an `.mfp` package. Imported `.mfp` packages must have a compatible bytecode/package format version, compatible public API metadata, and an MFBASIC language version supported by the compiler.

Executable builds use a lockfile named `mfb.lock`. The lockfile records the exact selected package identity, version, source or registry alias, content hash, bytecode/package version, native dependency metadata hash, and transitive dependencies. Locked builds must use the lockfile selections exactly; a hash or version mismatch fails before compilation, bytecode merging, or native linking.

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

Native backends use one allocator-agnostic bytecode contract for heap-backed values. Bytecode names value operations; native lowering chooses whether a value is inline, static, stack-resident, or arena-backed.

This language specification defines the ownership, aliasing, copy, move, and return behavior of heap-backed values; it does not define a universal per-object header or a byte-for-byte native representation for every value kind. Concrete runtime layouts for strings, records, unions, collections, and any future heap-backed value category are specified in `specifications/memory_layouts.md` and in the corresponding package/native ABI specifications when values cross an ABI boundary. Native lowering must follow those layout contracts consistently for construction, field access, union wrapping and extraction, collection storage, helper calls, and package/native ABI interop.

Arena allocation is an implementation strategy for native backends. An arena allocator may maintain allocator-private block headers or bookkeeping, but those allocator structures are not part of the source-level value model and must not be treated as a required object prefix for all arena-backed values.

Copy and move of arena-backed immutable values may be represented by copying the native value handle used by the active layout, provided the ownership rules above remain observable. Drop may be a no-op for individual values when all owned arena blocks are released at package-instance shutdown. Returning a heap-backed value copies or moves the native value into caller-owned storage according to the active layout, so returned values never point into the callee stack frame or into an arena whose lifetime is shorter than the caller-visible value.

Each package instance owns one arena. Worker threads or future isolated package instances get distinct arenas. A value that crosses from one arena-owned execution context to another must be transferred into storage whose lifetime is valid for the receiver before the receiver observes it. The native representation must not expose a handle into the sender's arena as a receiver-owned value unless that arena is also kept alive by the transfer object for the full receiver-visible lifetime. `arena_alloc(size, align)` validates that alignment is a non-zero power of two, treats zero-size allocations as one byte, rounds addresses with checked arithmetic, grows chained blocks when needed, uses a large-allocation block path for oversized requests, and reports `ErrInvalidArgument` or `ErrOutOfMemory` through ordinary language-level `Result` propagation.

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

Ordinary containers cannot store resource handles or thread handles. They can store functions only when the function value is copyable or movable under the closure rules above.

No two live mutable bindings may refer to the same collection buffer. A `MUT` collection buffer may be destructively updated only while it is owned by that single live `MUT` binding. Reads produce owned values, not aliases into the buffer.

### 14.7 Drop order

At normal scope exit, `RETURN`, `FAIL`, `PROPAGATE`, or auto-propagated errors, live bindings are dropped in reverse declaration order within each scope. Nested scopes drop before enclosing scopes continue. Record fields drop in declaration order. Union member values drop according to the active member's record layout. List elements drop from highest index to lowest. Map entries drop in implementation-defined storage order; programs must not depend on map drop order.

Moved-from bindings are not dropped. Frozen buffers are dropped as immutable collection values by their final owner.

### 14.8 Diagnostics

The compiler must diagnose:

- Use after move.
- Copy attempts for non-copyable types.
- Cyclic value construction.
- Capturing `MUT` bindings in closures.
- Capturing resource handles in ordinary closures.
- Capturing other non-copyable values in ordinary closures.
- Storing resources or thread handles in ordinary collections.
- Any control-flow path that could drop the same resource or owned value more than once.

`.mfp` packages must preserve enough ownership metadata for import-time type checking and bytecode verification (§21).
At minimum, exported type shape metadata must remain sufficient to reconstruct copyability, resource/thread containment, and drop-sensitive ownership checks when imported packages participate in move analysis.

---

## 15. Resource Management

`RESOURCE` values, such as files and sockets, are unique handles. At any point in the program, exactly one live owner is responsible for each open handle. Resource handles are non-copyable owned values with additional close rules. They are closed automatically by lexical drop (§14.7) when their owning binding leaves scope, on every exit path: normal scope exit, `RETURN`, `FAIL`, `PROPAGATE`, an auto-propagated `Err`, and `TRAP` routing. There is no user-visible lifetime construct; a resource is released by the same ownership and drop rules as any other owned value.

```basic
FUNC readFirstLine(path AS String) AS String
  LET f = fs::openFile(path)   ' auto-propagates on Err
  LET line = fs::readLine(f)   ' if this fails, f is still closed on the error exit
  RETURN line                  ' f is dropped (closed) here, on the success exit
END FUNC
```

A resource is closed exactly once. Standard resource operations borrow the handle for the duration of the call without transferring ownership. The explicit close operation consumes the handle, so the source binding is moved and cannot be used afterward; an already-moved binding is not dropped again. A resource handle cannot be copied, stored in an ordinary collection, printed, compared, serialized, or captured by a lambda or ordinary closure. A concrete resource handle may be sent to a thread only when that resource type is thread-sendable.

A `RESOURCE` value may be passed only to a function whose signature explicitly names that concrete resource type, such as `File`, `Socket`, or a `LINK`-declared `RESOURCE`. There is no generic `RESOURCE` supertype, no structural matching of handles, and no implicit conversion between resource types.

Borrow and consume are compiler rules inferred from the resource operation. They are not source-level annotations; MFBASIC does not add `BORROW`, `MOVE`, or similar parameter syntax for ordinary resource use.

To release a resource earlier than the end of its scope, or to observe a close failure, call the resource's explicit close operation (such as `fs::close(f)`). That operation consumes the handle, returns a `Result`, and auto-propagates a close `Err` like any other call, so the close failure is directly observable. After an explicit close the binding is moved and is not closed again by lexical drop. Reassigning a `MUT` resource binding evaluates the right-hand side first; if that succeeds, the old handle is dropped (closed) before the binding stores the new handle.

A close that runs as part of an implicit lexical drop cannot inject an error into program flow, because a drop has no source-level result to route. If such a drop-close fails, the failure is emitted as diagnostic/audit metadata associated with the failed cleanup; it does not replace, wrap, or raise a source-level `Error`. Programs that must observe a close failure use the explicit close operation instead.

This rule does not change the built-in `Error` shape: A secondary close failure is not directly inspectable by ordinary source code unless a future diagnostics API exposes cleanup metadata.

Compiled cleanup metadata must preserve enough information for runtime and audit tooling to report a drop-close failure. Package audit output should identify cleanup regions that retain this failure metadata.

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
- Thread boundary types must be thread-sendable. Primitive owned values, `String`, `Nothing`, records, unions, `Result`, and immutable containers are sendable when every contained field, payload, element, key, or value type is sendable. Functions, lambdas, `Thread`, `ThreadWorker`, and opaque resource handles are not sendable by default.
- Concrete resource types opt in to thread sendability. Standard `File`, `Socket`, and `UdpSocket` handles are sendable. `Listener` is not sendable. A successful send of a non-copyable sendable resource moves ownership to the destination side immediately; a failed send leaves ownership with the sender.
- A thread's top-level `MUT` state is private to that thread's package instance.
- If the thread entry function returns `Ok(v)`, the thread's stored result becomes the success member with payload `v`. If it fails with `Error(e)`, including through auto-propagation, the thread's stored result becomes the error member carrying `e`. If the stored result still references worker-arena storage internally, the runtime keeps that worker arena live through the `Thread` result owner and materializes a receiver-owned copy before `t.result` or `thread::waitFor(t)` exposes the value to user code.
- The `Thread` value owns the completed result after the thread ends until that result is retrieved. Field access `t.result` waits until completion, returns the stored `Result OF Out`, and consumes/closes the parent `Thread` handle. `thread::waitFor(t)` waits until completion, retrieves the same result, auto-unwraps or auto-propagates it like any other function call, and consumes/closes the parent `Thread` handle. After either retrieval path, any further use of the same `Thread` handle fails with `ErrResourceClosed`.

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
```

Thread functions are ordinary built-in templates. Their `Msg` and `Out` parameters are resolved by the template rules in §3 from argument types and expected result types. `thread::start` gets `Msg` and `Out` from the started function's first `ThreadWorker OF Msg TO Out` parameter, and gets `In` from the started function's second parameter and the `data` argument. If a thread does not exchange messages, `Msg` may be `Nothing`.

Each thread has a bounded inbound queue and bounded outbound queue. Queue entries own transferred values in receiver-valid storage or runtime transfer storage, never as a bare dependency on the sender's arena. `thread::start` rejects limits less than `1` with `ErrInvalidArgument`. `thread::send(Thread, ...)` sends a value to the worker inbound queue. `thread::receive(ThreadWorker, ...)` reads from that inbound queue and is valid only inside the running worker. `thread::send(ThreadWorker, ...)` sends to the parent-visible outbound queue. `thread::poll` waits up to `ms` milliseconds for an outbound message from the worker and returns `TRUE` when `thread::receive(Thread, ...)` can read without blocking. `thread::receive(Thread, ...)` reads the next outbound message. Reading with no available message fails with `ErrNotFound`.

For queue operations, `timeoutMs = 0` means do not wait. A positive timeout waits up to that many milliseconds for space or data. Sending to a full queue or receiving from an empty queue after the timeout fails with `ErrTimeout`. Negative timeouts are invalid except where a specific overload documents an indefinite worker-side wait, such as `thread::receive(ThreadWorker, -1)`.

`thread::cancel` requests cooperative cancellation. It does not kill the worker immediately. The worker observes cancellation with `thread::isCancelled(t)` and should return or fail promptly. After cancellation is requested, new parent-side `thread::send` calls fail with `ErrInterrupted`; unread inbound messages may be discarded. Outbound messages already sent by the worker remain readable until drained. Runtime-managed worker queue cancellation points, including `thread::receive(ThreadWorker, ...)` and `thread::send(ThreadWorker, ...)`, wake and fail with `ErrInterrupted` when cancellation is requested. Other blocking built-ins that are implemented as runtime-managed waits, such as terminal input, blocking file reads, or network waits, must use the same cooperative error-return model when cancellation integration is provided. Cancellation points do not asynchronously kill the worker or interrupt arbitrary user/native code.

When a thread ends, its inbound queue is closed and further parent-side sends fail. Its outbound queue remains readable until drained; after it is empty, `thread::poll` returns `FALSE` and `thread::receive(Thread, ...)` fails with `ErrNotFound`. `thread::waitFor` and `t.result` may be used before or after draining messages, but either operation retrieves the stored result exactly once and closes the parent `Thread` handle. Closing the handle drops any remaining queued outbound messages. The worker arena may be released only after the worker result has been transferred out of that arena or the runtime has otherwise kept that arena live through result retrieval, and any outbound messages have either been transferred to queue-owned storage or dropped. Dropping a completed `Thread` handle releases all remaining queued messages. Dropping a running `Thread` handle requests cancellation and detaches the worker; the runtime must reclaim the worker when it exits, preventing zombie threads.

`Thread` values are non-copyable owned handles and participate in lexical cleanup. Scope exit, `RETURN`, `FAIL`, `PROPAGATE`, auto-propagated errors, and trap routing drop live parent `Thread` handles in reverse declaration order together with other owned values. Reassigning a `MUT Thread` evaluates the right-hand side first; if that succeeds, the old handle is dropped before the binding stores the new handle. A `Thread` binding that has moved out through return or another consuming operation is not dropped by the source scope. `thread::waitFor(t)` and `t.result` close the underlying handle but do not make the source binding syntactically moved; later user-visible operations fail with `ErrResourceClosed`, while compiler-generated lexical cleanup is idempotent for an already closed handle.

---

## 17. Native Libraries

Native libraries are host dynamic libraries loaded through reusable `.mfp` binding packages. MFBASIC code cannot call arbitrary C symbols directly. A package that contains a `LINK` block declares the library name, its package-like namespace, opaque resource types, and the typed wrapper functions that are visible to MFBASIC code. Compiling that package emits normal `.mfp` bytecode plus native binding metadata.

Application packages do not repeat a dependency's `LINK` block. They import the binding package normally with `IMPORT`, call its exported wrapper functions, and use its resource types through ordinary ownership and lexical-drop behavior. Final executable builds collect native dependencies from all imported `.mfp` packages, resolve them once for the target platform, validate their manifests, and link or load the declared native libraries before `main`.

* Native ABI details do not leak across package boundaries unless explicitly part of the binding package's public API.
* Application code importing a binding package sees ordinary MFBASIC types, functions, resources, `Result` behavior, and lexical-drop cleanup behavior.
* A source package that declares `LINK` is a binding package. It may also include ordinary MFBASIC wrapper code, validation, and higher-level helpers around the native symbols.

```basic
LINK "sqlite3" AS sqlite
  TYPE Db AS RESOURCE
    CLOSE close
  END TYPE

  FUNC open(path AS String) AS Db
    SYMBOL "sqlite3_open"
    ABI (CString, OUT CPtr) AS CInt32
    SUCCESS_ON 0
  END FUNC

  FUNC close(db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (CPtr) AS CInt32
    SUCCESS_ON 0
  END FUNC
END LINK
```

`LINK "sqlite3" AS sqlite` creates the namespace `sqlite`, so native wrapper functions are called like package functions:

```basic
LET db = sqlite::open("app.db")
' use db
' db is closed by lexical drop when its scope ends, or by an explicit sqlite::close(db)
```

`TYPE Db AS RESOURCE` declares an opaque unique native handle. For a C library this is usually represented by a pointer or host handle internally, but source code cannot inspect, cast, compare, serialize, print, copy, capture in a lambda, store in an ordinary collection, send to a thread unless the concrete resource type is declared thread-sendable, or do arithmetic on it. A resource may be passed only to functions whose signatures explicitly accept that resource type. Resource handles are not sendable to threads unless the concrete resource type opts in.

`CLOSE close` names the native wrapper function that releases the resource. The close function runs automatically when the resource binding is dropped at scope exit, including on error exits, and may also be called explicitly to release the resource early or to observe a close failure. Calling a native wrapper function with a closed resource fails with `ErrResourceClosed`.

`SYMBOL "sqlite3_open"` gives the exact native symbol name to look up in the loaded library. The MFBASIC function name is the public wrapper name; it does not have to match the native symbol name.

`ABI (...) AS ...` gives the native C-facing call shape: The `FUNC` signature is the MFBASIC-facing wrapper type; the `ABI` signature is the host-library symbol's argument and return representation.

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

ABI parameters may use direction modifiers:

| Form | Meaning |
|------|---------|
| `REF T` | Pass a pointer to a temporary native value initialized from the MFBASIC argument. The pointer lifetime ends when the native call returns. |
| `OUT T` | Pass a pointer to uninitialized native storage and copy the result back after the call. The pointer lifetime ends when the native call returns. |
| `CPtr` | Pass a resource handle or opaque pointer as-is inside the binding boundary. |

Native return handling can be declared when the C return value is not simply the MFBASIC result:

| Form | Meaning |
|------|---------|
| `SUCCESS_ON value` | The native return is a status code. `value` means success; any other native return is an error. The MFBASIC success value must come from an `OUT` parameter or be `Nothing`. |
| `ERROR_ON value` | The native return is the result value except for one sentinel. `value` means error; any other native return is converted to the MFBASIC return value. |

`SUCCESS_ON 0` is common for libraries such as SQLite, where `0` means success and nonzero values are error codes. `ERROR_ON -1` is common for POSIX-style APIs, where `-1` means failure and any other returned value is valid.

```basic
FUNC openFd(path AS String, flags AS Integer) AS Integer
  SYMBOL "open"
  ABI (CString, CInt32) AS CInt32
  ERROR_ON -1
END FUNC
```

When an ABI signature has `OUT` parameters, `RETURN_OUT` can define how those output values become the MFBASIC success value. A single `OUT` may be returned implicitly when the wrapper return type is not `Nothing`. Multiple `OUT` parameters require `RETURN_OUT`.

For multiple outputs, define a normal record type and construct it from the `OUT` positions:

```basic
TYPE DivModResult
  quotient AS Integer
  remainder AS Integer
END TYPE

LINK "mylib" AS mylib
  FUNC divmod(a AS Integer, b AS Integer) AS DivModResult
    SYMBOL "divmod"
    ABI (CInt32, CInt32, OUT CInt32, OUT CInt32) AS CVoid
    RETURN_OUT DivModResult[3, 4]
  END FUNC
END LINK
```

`RETURN_OUT DivModResult[3, 4]` means: after the native call succeeds, convert the third and fourth ABI arguments from their output storage and return `Ok(DivModResult[out3, out4])`.

Rules:

- `LINK` names and all declared `SYMBOL` names are resolved before `main` starts. Native libraries are not lazy-loaded.
- If a required native library or symbol cannot be loaded before `main`, the program terminates before entering `main`. The diagnostic is written to stderr and the process exits with `55000001` (`ErrLinkFailed`). This startup failure is outside the `Result`/`TRAP` model because no MFBASIC function is running yet.
- Linked names occupy a package-like namespace. A package-qualified name such as `sqlite::open` follows the same two-part rule as package access.
- A native call may resolve only the symbols declared by `SYMBOL` entries in the binding package. Dynamic lookup by source strings or computed names is not available to ordinary MFBASIC code.
- Native functions expose ordinary MFBASIC signatures. At call sites they auto-unwrap, auto-propagate, and participate in `MATCH` like any other fallible function.
- Native functions may accept and return MFBASIC primitive values, strings, byte lists, and declared resource types through an explicit `ABI` mapping. Other conversions are implementation-defined unless specified by the binding.
- If a native function has more than one `OUT` parameter and its MFBASIC return type is not `Nothing`, it must declare `RETURN_OUT`.
- `RESOURCE` is a declaration form for concrete opaque unique-handle types; it is not an inheritance base type and cannot be used as a generic catch-all type.
- Native resource ownership must be declared with `TYPE ... AS RESOURCE` and `CLOSE`; raw `CPtr` values must not escape into ordinary MFBASIC APIs.
- `REF` and `OUT` native pointer values are temporary call-frame values. Native code must not retain them after return; if a binding needs retained native storage, it must model that storage as a declared `RESOURCE`.
- Native libraries are platform-specific dependencies. A `.mfp` package may declare that it needs a native library, including version, search policy, platform constraints, and content/hash requirements, but the native library itself is not portable bytecode.

---

## 18. Built-in Functions

Terminal and standard-stream I/O: `io::print`, `io::write`, `io::printError`, `io::writeError`, `io::flush`, `io::flushError`, `io::input`, `io::readLine`, `io::readChar`, `io::readByte`, `io::isInputTerminal`, `io::isOutputTerminal`, `io::isErrorTerminal`, `io::terminalSize`.
Filesystem and file I/O: `fs::fileExists`, `fs::directoryExists`, `fs::exists`, `fs::readBytes`, `fs::readText`, `fs::writeBytes`, `fs::writeText`, `fs::writeBytesAtomic`, `fs::writeTextAtomic`, `fs::appendBytes`, `fs::appendText`, `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::createTempFile`, `fs::tempDirectory`, `fs::readLine`, `fs::readAll`, `fs::readAllBytes`, `fs::writeAll`, `fs::writeAllBytes`, `fs::close`, `fs::eof`, `fs::canonicalPath`, `fs::isWithin`, `fs::pathJoin`, `fs::pathDirName`, `fs::pathBaseName`, `fs::pathExtension`, `fs::pathNormalize`, `fs::deleteFile`, `fs::createDirectory`, `fs::createDirectories`, `fs::deleteDirectory`, `fs::listDirectory`, `fs::currentDirectory`, `fs::setCurrentDirectory`.
Network: `net::lookup`, `net::connectTcp`, `net::listenTcp`, `net::accept`, `net::bindUdp`, `net::receiveFrom`, `net::receiveTextFrom`, `net::sendTo`, `net::sendTextTo`, `net::poll`, `net::read`, `net::readText`, `net::write`, `net::writeText`, `net::close`, `net::localAddress`, `net::remoteAddress`, `net::setReadTimeout`, `net::setWriteTimeout`, `tls::connect`, `tls::wrap`, `tls::close`.
Strings: `len`, `find`, `mid`, `replace`, `strings::trim`, `strings::trimStart`, `strings::trimEnd`, `strings::upper`, `strings::lower`, `strings::caseFold`, `strings::normalizeNfc`, `strings::graphemes`, `strings::startsWith`, `strings::endsWith`, `strings::contains`, `strings::split`, `strings::join`, `strings::byteLen`, `toString`, `toInt`, `toFloat`, `toFixed`, `toByte`, `isNumeric`, `&`.
Regex: `regex::match`, `regex::find`, `regex::replace`.
Collections: `forEach`, `transform`, `filter`, `reduce`, `sum`, `get`, `getOr`, `find`, `mid`, `replace`, `set`, `append`, `prepend`, `insert`, `removeAt`, `removeKey`, `keys`, `values`, `hasKey`, `contains`, `len`.
Threads: `thread::start`, `thread::isRunning`, `thread::waitFor`, `thread::cancel`, `thread::send`, `thread::poll`, `thread::receive`, `thread::isCancelled`.
Math: `math::pi`, `math::piFixed`, `math::e`, `math::eFixed`, `math::abs`, `math::min`, `math::max`, `math::clamp`, `math::floor`, `math::ceil`, `math::round`, `math::sqrt`, `math::pow`, `math::exp`, `math::log`, `math::log10`, `math::sin`, `math::cos`, `math::tan`, `math::asin`, `math::acos`, `math::atan`, `math::atan2`.
JSON: `json::parse`, `json::stringify`, `json::get`, `json::getOr`.
Error codes: `errorCode::ErrInvalidArgument`, `errorCode::ErrNotFound`, and the other constants listed in the built-in error-code registry.

Fallible built-ins (`fs::openFile`, `toInt`, `get`, …) return `Result` and auto-propagate like any call.

---

## 19. Grammar (EBNF, abridged)

```ebnf
program        = { import | linkDecl } { declaration } ;

import         = "IMPORT" ident [ "AS" ident ] ;
linkDecl       = "LINK" string "AS" ident { linkItem } "END" "LINK" ;
linkItem       = nativeTypeDecl | nativeFuncDecl ;
nativeTypeDecl = "TYPE" ident "AS" "RESOURCE"
                   [ "CLOSE" ident ] "END" "TYPE" ;
nativeFuncDecl = "FUNC" ident "(" [ params ] ")" "AS" type
                   nativeFuncBody "END" "FUNC" ;
nativeFuncBody = "SYMBOL" string
                   [ "ABI" "(" [ nativeParamList ] ")" "AS" nativeType ]
                   [ nativeReturnRule ]
                   [ returnOut ] ;
nativeReturnRule = "SUCCESS_ON" literal | "ERROR_ON" literal ;
returnOut       = "RETURN_OUT" returnOutExpr ;
returnOutExpr   = integer | constructor ;
nativeParamList = nativeParam { "," nativeParam } ;
nativeParam     = [ "REF" | "OUT" ] nativeType ;
nativeType      = "CInt8" | "CInt16" | "CInt32" | "CInt64"
                | "CUInt8" | "CUInt16" | "CUInt32" | "CUInt64"
                | "CBool" | "CFloat32" | "CFloat64"
                | "CIntPtr" | "CUIntPtr" | "CSize"
                | "CString" | "CPtr" | "CVoid" ;

declaration    = topLetDecl | topMutDecl
               | funcDecl | subDecl | typeDecl | unionDecl | enumDecl ;

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
               | returnStmt | exprStmt ;
forStmt        = "FOR" ident "=" expr "TO" expr [ "STEP" expr ]
                   block "NEXT" ;
foreachStmt    = "FOR" "EACH" ident "IN" expr block "NEXT" ;
whileStmt      = "WHILE" expr block "WEND" ;
doStmt         = "DO" block "LOOP" "UNTIL" expr
               | "DO" "WHILE" expr block "LOOP" ;

failStmt       = "FAIL" expr ;
propagateStmt  = "PROPAGATE" ;
returnStmt     = "RETURN" [ expr ] ;
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
  IF len(parts) <> 3 THEN FAIL Error[77050002, "expected 3 fields"]

  LET x = toFloat(strings::trim(get(parts, 0)))   ' auto-propagates on Err
  LET y = toFloat(strings::trim(get(parts, 1)))
  LET z = toFloat(strings::trim(get(parts, 2)))
  RETURN Vec3[x, y, z]
END FUNC

FUNC loadPoints(path AS String) AS List OF Vec3
  MUT pts AS List OF Vec3 = []
  LET f = fs::openFile(path)                ' auto-propagates on Err
  WHILE NOT fs::eof(f)
    LET v = parseLine(fs::readLine(f))      ' auto-propagates to TRAP below on bad input
    pts = append(pts, v)                   ' optimized in place for MUT
  WEND
  RETURN pts                               ' f closed by lexical drop here; pts freezes automatically

  TRAP err
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

  TRAP err
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
      FAIL Error[77050004, "shape member not handled"]
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

MFBASIC uses source files for authoring, portable bytecode packages, and native binaries for executables.

| Artifact | Extension | Purpose |
|----------|-----------|---------|
| Source file | `.mfb` | Human-authored source code. The `.mfb` files selected by a project's `project.json` together form that project's source package (§13). |
| Package | `.mfp` | Architecture-neutral bytecode package with embedded package manifest, public API metadata, dependency metadata, and optional native-link metadata. A compiled package can be built on one platform and imported on any platform that supports the same MFB bytecode/package version. |
| Executable | platform-native | Final application binary for the target OS/CPU. Executables compile application code plus imported `.mfp` packages to native code. |

The backend pipeline is:

```text
.mfb source
  -> typed program representation
  -> register bytecode
  -> .mfp package or native executable
```

Package compilation emits `.mfp` packages containing portable bytecode plus the embedded package manifest, dependency metadata, native-link metadata, and public API metadata needed for import, type checking, bytecode merging, and verification. This metadata includes each exported type and function's ownership properties: copyability, movability, resource-handle status, closure-capture requirements, thread-sendability, drop requirements, and collection element constraints. A package containing `LINK` declarations emits a reusable native binding `.mfp`: importers consume the package API and do not repeat the `LINK` declarations.

Executable compilation consumes `.mfb` application source, the resolved `mfb.lock`, and imported `.mfp` packages. The compiler statically merges imported package bytecode into the project bytecode, resolving package-qualified MFBASIC calls to functions in the merged bytecode image. After bytecode merging, the native backend resolves all native dependencies declared by the merged packages, performs target OS/native linking as needed, and emits a native binary for the selected target platform.

### 21.1 `.mfp` bytecode verification

Every `.mfp` package is verified before its bytecode can be imported, merged into project bytecode, or executed by a VM. Verification is deterministic and must reject malformed packages before any package code runs.

The verifier must check:

- Package metadata is well-formed, uses a supported bytecode/package version, satisfies the resolved manifest and lockfile entries, and matches the bytecode body.
- The package signature, hash, or trust record is valid when the build mode requires signed or locked dependencies.
- Public API metadata is consistent with bytecode definitions, including exported names, type shapes, function signatures, ownership properties, and native-link declarations.
- Bytecode instructions are type-correct at every program point. Operand types, result types, call signatures, record fields, union member types, collection element types, and `Result` handling must match the typed metadata.
- Stack slots, registers, temporaries, locals, and return slots are definitely initialized before read and are not read after move.
- Resource ownership is linear. A resource handle has one owner, is not copied, is not stored in ordinary collections, is sent to threads only when its concrete type is thread-sendable, and is closed or moved exactly once on every control-flow path.
- Drop and cleanup paths are valid. The verifier rejects double-drop, missing-drop, and use-after-drop paths.
- Control-flow targets are valid instruction boundaries inside the same function. Branches cannot jump into another function, into the middle of an instruction, into a `TRAP` body except through the error-routing edge, or into cleanup/finalizer code except through compiler-emitted cleanup edges.
- All normal and error paths satisfy the function's declared return type and `Result` behavior.
- Exception-like unwinding opcodes do not exist; error routing must use the specified `Result`/`TRAP` control-flow form.
- Native-link manifests are valid: every linked library and symbol referenced by bytecode is declared in metadata, every resource close function exists and has the correct resource-consuming signature, and every ABI mapping uses supported native types.

Verification failure rejects the package with a toolchain diagnostic. It is not recoverable by program `TRAP` code because no package code has started running.

An implementation may start with a tree-walk interpreter, then add a register bytecode VM, then add native code generation. The artifact contract remains: packages are portable `.mfp` bytecode packages; executables are native platform binaries.

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

`mfb test` discovers exported or private zero-argument `SUB` declarations whose names start with `test` in files included by the `project.json` test source entries. A test succeeds when it returns `Ok(NOTHING)` and fails when it returns an error result. Test builds use the same package resolver, verifier, resource rules, and audit metadata as executable builds.

`mfb lsp` starts the language-server protocol implementation. It must expose diagnostics for fallible calls, auto-propagation paths, `TRAP` recovery, resource moves/use-after-move, unsafe or invalid native links, permissions, package-version conflicts, lockfile mismatches, dense security-sensitive lines, and identifier near-collisions.
