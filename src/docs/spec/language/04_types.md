# 4. Types

## 4.1 Primitives

| Type | Description |
|------|-------------|
| `Integer` | 64-bit signed |
| `Float` | 64-bit IEEE float |
| `Fixed` | 64-bit binary fixed-point, signed 32/32 split |
| `Boolean` | `TRUE` / `FALSE` |
| `String` | UTF-8, immutable |
| `Byte` | unsigned 8-bit |

`Fixed` is a binary fixed-point number with an integer part and a `1 / 2^32` fractional part (the 8-byte runtime storage layout is specified by `./mfb spec memory scalar-storage`). Its range is approximately `-2147483648.0` through `2147483647.9999999998`. Fixed-point arithmetic is deterministic across targets, but it is not exact decimal currency arithmetic because most decimal fractions are rounded to binary fixed-point values. Overflow produces an error result with code `77050010`; divide-by-zero and invalid numeric domains produce an error result with code `77050002`.

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

Numeric comparisons (`=`, `<>`, `<`, `>`, `<=`, `>=`) use the same operand promotion rules for comparison but always return `Boolean`. `=` and `<>` also accept any two compatible comparable operands. The ordering operators `<`, `>`, `<=`, and `>=` additionally accept two `String` operands, which are ordered lexicographically by Unicode scalar value (see §4.11); mixed `String`/numeric ordering is a type error.

Numeric edge cases:

- `Integer` arithmetic is checked. Overflow in `+`, `-`, `*`, unary `-`, exponentiation (`^`), and the minimum-integer `MOD -1` case fails with `ErrOverflow` (`77050010`). Integer operations never wrap.
- `Byte` arithmetic that returns `Byte` is checked. Results above `255` fail with `ErrOverflow` (`77050010`); results below `0` fail with `ErrUnderflow` (`77050011`). Byte operations never wrap.
- `/` uses the promoted result type from the table above. When `/` promotes to `Byte` or `Integer`, it truncates the quotient toward zero. `DIV` is fractional division and always returns `Float`. Division by zero in a `Float`-result `/` or `DIV` is not pre-checked: `x / 0` yields `±Inf` and `0.0 / 0.0` yields `NaN`, which are caught at the next observation boundary (`ErrFloatOverflow` (`77050015`) / `ErrFloatNaN` (`77050013`); see the `Float` finiteness rule below). Division by zero for a non-`Float` result fails with `ErrInvalidArgument` (`77050002`).
- `MOD` uses the promoted result type from the table above and is available for every numeric operand pairing in the table. `a MOD b` fails when `b = 0`, with `ErrFloatDomain` (`77050012`) for `Float` results and `ErrInvalidArgument` (`77050002`) otherwise. Otherwise the remainder has the same sign as `a`, and `a = (truncTowardZero(a / b) * b) + (a MOD b)` in the promoted numeric domain.
- `^` for `Integer` requires a non-negative integer exponent and fails with `ErrInvalidArgument` (`77050002`) for negative exponents. Overflow fails with `ErrOverflow` (`77050010`). `^` for a `Float` result requires a whole, non-negative exponent and fails with `ErrFloatDomain` (`77050012`) otherwise; overflow to infinity is caught at the observation boundary (below), not at the operator.
- `Float` follows IEEE 754 binary64 representation. MFBASIC guarantees that **no user-accessible `Float` is non-finite**, enforced at *observation boundaries* rather than after each operation: a finiteness check fires only where a `Float` becomes observable — bound to a named local/global, assigned, stored into a collection element or record field, returned, passed as an argument, or printed/converted. An anonymous intermediate expression result may be non-finite transiently and may recover to finite without trapping (for example `1.0 / (1e200 * 1e200)` evaluates to `+0.0`). At a boundary a `NaN` fails with `ErrFloatNaN` (`77050013`) and an infinity fails with `ErrFloatOverflow` (`77050015`, "arithmetic overflow to infinity"), stamped at that statement's location. Built-in math functions that have a genuine domain error still fail at the call: a negative `sqrt`, a non-positive `log`/`log10`, or an out-of-range `asin`/`acos` fail with `ErrFloatDomain` (`77050012`), and a math kernel that produces an infinity (such as `exp` overflow) fails with `ErrFloatInf` (`77050014`). Imported native `Float` values that are already NaN or infinity are rejected at the boundary with `ErrInvalidFormat` (`77050003`).
- Float comparisons follow IEEE 754, including for the non-finite intermediates the boundary rule permits (a comparison is not an observation boundary, so comparing a transient non-finite never traps). Any ordered comparison (`<`, `<=`, `>`, `>=`) involving `NaN` is `false`; `=` with a `NaN` operand is `false` (so `NaN = NaN` is `false`) and `<>` is `true`; `+Inf` is greater than every finite value and `-Inf` is less than every finite value. Value equality is IEEE, so `+0.0 = -0.0` is `true`. Map-key equality is a separate, **bitwise** comparison (`+0.0 ≠ -0.0`, `NaN = NaN`) and is unchanged by this rule.
- Converting `Float` or `Fixed` to `Integer` or `Byte` fails with `ErrOverflow` (`77050010`) when outside the destination range. Converting text to a numeric type fails with `ErrInvalidFormat` (`77050003`) when the text is malformed or names a non-finite value such as `NaN` or `Infinity`. [[src/target/shared/code/builder_codegen_primitives.rs:emit_float_domain_return]]

## 4.2 Records (product types)

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

A record type may not contain itself, directly or transitively, except through a `List`, `Map`, or `UNION`. A field is a mandatory owned value with no null or absent form, so a record whose field cycles back to the same record only through other plain records has no base case and can never be constructed. Such a declaration is rejected with `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`: [[src/rules/table.rs:642]]

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

## 4.3 Unions (sum types)

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

## 4.4 `Error` and absence (built in)

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
`WITH`, or assign to their fields. [[src/rules/table.rs:480]] [[src/rules/table.rs:486]] User-authored errors are created with the
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

> **Implementation note.** Internally every fallible outcome is a private
> success-or-`Error` form that is not nameable, constructible, or matchable in
> user code; it exists only in compiler IR and is never observable in user
> syntax. The native register-level result ABI (success/error/exit tags) is
> specified by `./mfb spec memory fallible-call-abi`.

## 4.5 Enums

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

## 4.6 `Nothing`

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

## 4.7 Collections

```basic
List OF T                          ' owned sequence
Map OF K TO V                      ' owned map
MapEntry OF K TO V                 ' map iteration entry
Pair OF A, B                       ' two-value product
Partition OF T                     ' predicate split result
```

`List`, `Map`, `MapEntry`, `Pair`, and `Partition` are built-in templates. Each concrete use, such as `List OF Integer`, `Map OF String TO Float`, `MapEntry OF String TO Float`, `Pair OF Integer, String`, or `Partition OF Integer`, is monomorphized before binary representation generation. There is one sequence type, `List`. There are no fixed-size arrays and no `DIM`. See §12.

`MapEntry OF K TO V` is the compiler-owned record shape used when iterating a map. It has public read-only fields `key AS K` and `value AS V`.

`Pair OF A, B` and `Partition OF T` are compiler-owned, always-in-scope generic records (used by `collections::zip` and `collections::partition`). They are ordinary records — public, constructible (`Pair[a, b]`, `Partition[matched, unmatched]`), copyable when their members are copyable, and sendable across threads when their members are sendable.

```basic
TYPE Pair OF A, B
  first  AS A
  second AS B
END TYPE

TYPE Partition OF T
  matched   AS List OF T
  unmatched AS List OF T
END TYPE
```

Unlike `MapEntry`, `Pair` places no comparability constraint on `A` or `B`. `Partition OF T` is defaultable when `T` is (two empty lists); `Pair OF A, B` is defaultable when both `A` and `B` are. The names `Pair` and `Partition` are reserved: a user `TYPE` may not redeclare them.

Runtime collection storage is specified by the memory spec
(`./mfb spec memory collections`).

## 4.8 Threads

```basic
Thread OF Msg TO Out                 ' isolated running or completed thread
ThreadWorker OF Msg TO Out           ' worker-side view of the same thread
Thread OF Msg RES Res TO Out         ' with a resource plane (thread::transfer/accept)
Thread OF RES Res TO Out             ' resource plane only (message slot is Nothing)
```

`Thread` and `ThreadWorker` are built-in templates for opaque handles to the same underlying package worker. `Thread` is the parent-side handle. `ThreadWorker` is the worker-side handle passed into the thread entry function. `Msg` is the message type used by `thread::send` and `thread::receive`; `Out` is the thread entry function's success type. A completed parent `Thread`'s outcome is retrieved only through `thread::waitFor(t)`, which auto-unwraps the `Out` value or auto-propagates the `Error`; retrieving the outcome consumes and closes the parent `Thread` handle.

## 4.9 Type Inference

`LET` and `MUT` infer when initialized; explicit `AS` otherwise required. The full inference, coercion, and assignability rules — including how untyped numeric literals acquire a type from the expected type — are specified by `./mfb spec language type-inference`. The canonical text spelling of each type used in diagnostics is specified by `./mfb spec language type-name-encoding`.

```basic
LET name = "world"        ' inferred String
MUT i AS Integer          ' explicit, uninitialized (defaults 0)
```

## 4.10 Default Values

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

The exact defaultability predicate is defined as follows. A type is defaultable when it is one of the scalars `Integer`, `Byte`, `Float`, `Fixed`, `Boolean`, the built-in record shapes `Error` and `ErrorLoc`, `String`, `Nothing`, or `Unknown` (the last is treated as defaultable so a prior type error does not cascade). A `List OF T` is defaultable when `T` is defaultable; a `Map OF K TO V` is defaultable when both `K` and `V` are defaultable. A user record `TYPE` is defaultable when every one of its fields is defaultable. Everything else is **not** defaultable: function/lambda types, the internal fallible-result type, resource-plane types (`RES`), `Thread`, `ThreadWorker`, any `TYPE` wrapped as a resource handle, and any `ENUM` or `UNION`. Defaultability is computed with a recursion guard keyed by type name: when a record type is re-entered while still being evaluated, the re-entered occurrence is treated as non-defaultable, which gives recursive record cycles no base case and therefore no default value. This predicate is enforced on the IR by `ir::verify` (`is_defaultable`). [[src/ir/verify/mod.rs:is_defaultable]]

## 4.11 Comparable and Orderable Types

Some standard functions require a type to be comparable. Comparable types are `Integer`, `Float`, `Fixed`, `Boolean`, `String`, `Byte`, `Nothing`, enum types, and records whose fields are all comparable. [[src/syntaxcheck/types.rs:is_comparable]] The built-in `Error` and `ErrorLoc` record shapes are comparable (their fields are all comparable). `List`, `Map`, unions, functions, lambdas, threads, resource handles, and the internal fallible-result type are not comparable. Record comparability is computed structurally with a recursion guard: a record that reaches itself only through non-comparable members (a cycle through `List`/`Map`/`UNION`) is not comparable, and a resource-wrapped `TYPE` is never comparable.

`Map` keys must be comparable. List helpers such as `find`, `contains`, and `replace` require comparable element types.

A narrower set of types is **orderable** — for these a total order is defined and the ordering operators apply. Orderable types are `Integer`, `Float`, `Fixed`, `Byte`, and `String`. `Boolean`, `Nothing`, enums, unions, and records are comparable but not orderable. Helpers such as `collections::sort` and `collections::sortBy` require an orderable element or key type.

| Property | Types |
|----------|-------|
| Comparable (`=`, `<>`) | `Integer`, `Float`, `Fixed`, `Boolean`, `String`, `Byte`, `Nothing`, enums, records of comparable fields |
| Orderable (`<`, `>`, `<=`, `>=`) | `Integer`, `Float`, `Fixed`, `Byte`, `String` |

For the purpose of these operators, "numeric" means `Integer`, `Float`, `Fixed`, `Byte`, or `Unknown` — `Unknown` is admitted as numeric so a prior type error does not cascade into a second one. [[src/syntaxcheck/types.rs:is_numeric]] Equality operators `=` and `<>` accept any two numeric operands directly, with **no** compatibility requirement between them: any cross-numeric pairing such as `Integer = Float`, `Byte <> Fixed`, or `Float = Fixed` is accepted and returns `Boolean`. Equality also accepts any two compatible comparable operands. Ordering operators `<`, `>`, `<=`, and `>=` require either two numeric operands or two `String` operands. Two `String` operands are ordered **lexicographically by Unicode scalar value**: the strings are compared scalar by scalar, the first differing position decides, and if one string is a prefix of the other the shorter compares less. This order is deterministic and identical across all targets — it does not depend on host locale, collation, or libc. It is not a locale or human collation and is not grapheme-cluster aware; callers needing locale-aware or case-insensitive ordering normalize or `strings::caseFold` first and sort the result. Mixed `String`/numeric ordering is a compile-time type error.

## 4.12 Compile-time numeric-literal range checks

Numeric **literals** are range-checked statically, as a separate phase from the runtime numeric conversions described in §4.1. This static phase rejects a literal whose value cannot be represented in the type it is being stored into, before any code runs; it is distinct from `toByte`/`toInteger`/`toFixed` runtime conversions, which fail with runtime error codes. A static literal-range violation is a compile error in the `TYPE_*_LITERAL_*` family, not a runtime `Err*`. The check applies to a bare numeric literal and to a literal under a unary `-`; it does not constant-fold larger expressions. It runs on the typed IR in the semantic checker (relocated from the source checker in plan-20), so it guards decoded package IR too. [[src/ir/verify/mod.rs:check_literal_range]] [[src/ir/verify/mod.rs:check_const_literal]]

**Integer.** An integer-looking literal (no `.`) that does not parse as `i64` is rejected with `TYPE_INTEGER_LITERAL_OVERFLOW`. A negated integer literal `-N` is accepted when `N` parses as `u64` and `N <= i64::MAX + 1`; that is, `-9223372036854775808` (the most-negative `Integer`) is accepted even though `9223372036854775808` on its own overflows, because the minus sign is folded into the range check. A negated literal outside that bound is also `TYPE_INTEGER_LITERAL_OVERFLOW`. [[src/ir/verify/mod.rs:check_const_literal]] [[src/ir/verify/mod.rs:check_negated_const_literal]]

**Byte.** An integer literal stored into a `Byte` is checked against `0..=255`: a value above `255` (or one too large to parse as `u16`) is `TYPE_BYTE_LITERAL_OVERFLOW`, and any nonzero negated literal is `TYPE_BYTE_LITERAL_UNDERFLOW`. Decimal-looking literals are not range-checked here (they are not valid `Byte` literals).

**Float.** A literal stored into a `Float` is parsed as `f64` and checked for finiteness: a value that parses to a non-finite `f64` is rejected. A non-finite negated literal is `TYPE_FLOAT_LITERAL_UNDERFLOW`; a non-finite positive literal is `TYPE_FLOAT_LITERAL_OVERFLOW`. A finite `f64` always passes. [[src/ir/verify/mod.rs:check_const_literal]]

**Fixed.** A literal stored into a `Fixed` is parsed as `f64`, the sign applied, and checked against the static binary bound: `value < -2147483648.0` is `TYPE_FIXED_LITERAL_UNDERFLOW` and `value >= 2147483648.0` is `TYPE_FIXED_LITERAL_OVERFLOW`. Values inside `[-2147483648.0, 2147483648.0)` pass. [[src/ir/verify/mod.rs:check_const_literal]]

## See Also

* ./mfb spec memory scalar-storage — runtime scalar payload sizes
* ./mfb spec memory fallible-call-abi — native result register ABI
* ./mfb spec memory collections — runtime `List`/`Map` storage
* ./mfb spec language collections — collection operations and semantics
* ./mfb spec language type-inference — inference, coercion, and assignability rules
* ./mfb spec language type-name-encoding — canonical text spelling of types in diagnostics
* ./mfb man types — type-related built-in help
