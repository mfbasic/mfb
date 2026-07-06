# A Tour of MFBASIC

A one-page tour of the MFBASIC language

MFBASIC is a modern, functional dialect of BASIC built around value
ownership: every value has a single owner and is reclaimed deterministically
when its scope exits — no garbage collector, no reference counting, no
user-visible free. Bindings are immutable by default, there are no objects,
and errors propagate automatically. This page walks the whole language once;
every heading ends with where to read more.

## MFBASIC at a glance

- Immutable by default
- Value ownership with deterministic cleanup
- Checked arithmetic — overflow fails, never wraps
- Pattern matching over closed unions
- Automatic error propagation, built in
- No exceptions
- No garbage collector
- No null

## Hello, world

A program starts at `main`. Built-in packages are brought in with `IMPORT` and
called with the `::` separator; a package holds free functions and types —
there are no classes or methods. A few core functions (`len`, `toString`,
`error`, ...) are always in scope without an import.

```
IMPORT io

SUB main()
  io::print("Hello, world")
END SUB
```

## Bindings

Three binding forms on two axes: `LET` and `MUT` choose mutability, `RES`
chooses ownership (files, sockets, and other unique handles). Types are
inferred from the initializer; annotate with `AS` when there is none. There is
no implicit declaration and no shadowing, and bindings die at scope exit.

```
LET name = "world"                  ' immutable, inferred String
MUT total AS Float = 0.0            ' reassignable
total = total + 1.0
RES f = fs::openFile("data.csv")    ' owned resource, closed by scope exit
```

More: `mfb man types`.

## Data

Scalars are `Integer`, `Float`, `Fixed`, `Boolean`, `String`, and `Byte`.
Integer arithmetic is checked (overflow fails, never wraps) and an observed
`Float` is never NaN or infinity.

Data has no attached behavior: a record (`TYPE`) carries only fields; a
`UNION` groups existing record types into one closed sum; an `ENUM` names a
fixed set of members. Constructors use square brackets — brackets are never
used for indexing — and `WITH` builds an updated copy without touching the
original.

```
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

LET s AS Shape = Circle[radius := 2.0]
LET r = Rect[w := 3.0, h := 4.0]
LET wider = WITH r { w := 10.0 }     ' r is unchanged
```

Collections are the built-in templates `List OF T` and `Map OF K TO V`. List
literals use bare brackets: `[1, 2, 3]`. All collection access goes through
free functions — there is no index-bracket syntax.

More: `mfb man types`, `mfb man collections`.

## Control flow

The classic BASIC forms, structured: `IF`/`ELSEIF`/`ELSE`, counted `FOR`,
`FOR EACH` over lists and maps, `WHILE`/`WEND`, and `DO` loops. Loops leave
and skip with `EXIT FOR`/`EXIT WHILE`/`EXIT DO` and the matching `CONTINUE`
forms. There is no `GOTO` and no `SELECT CASE`.

```
FOR i = 1 TO 10 STEP 2
  io::print(toString(i))
NEXT

FOR EACH item IN items
  io::print(item)
NEXT
```

`MATCH` is the pattern form and the replacement for `SELECT CASE`: it
deconstructs unions by binding member payloads, matches enum members and
literals, and its exhaustiveness is checked at compile time.

```
MATCH s
  CASE Circle(c)
    io::print("circle, radius " & toString(c.radius))

  CASE Rect(r)
    io::print("rect " & toString(r.w) & " by " & toString(r.h))
END MATCH
```

More: `mfb man flow`.

## Functions

Only `FUNC` (returns a value) and `SUB` (effect-only) — no methods. Every
`FUNC` path ends in `RETURN` or `FAIL`; a `SUB` exits with `EXIT SUB` or by
reaching `END SUB`.

```
IMPORT math

FUNC area(s AS Shape) AS Float
  MATCH s
    CASE Circle(c)
      RETURN math::pi * c.radius ^ 2.0

    CASE Rect(r)
      RETURN r.w * r.h
  END MATCH
END FUNC
```

Functions are values: `LAMBDA` builds anonymous functions (parameter types are
written, not inferred), and the pipeline operator `|>` threads a value through
calls at the `_` placeholder.

```
LET total = nums |> collections::filter(_, positive) |> collections::sum(_)
```

More: `mfb man lambda`.

## Errors

Every call either produces its value or fails with an `Error`. A successful
call evaluates directly to its value; failure auto-propagates to the nearest
`TRAP` or to the caller. There is no TRY, no exceptions, no null, and no
Option/Maybe — absence is an `Error` with a semantic code. Fail explicitly
with `FAIL error(code, message)`.

A function-level `TRAP` at the bottom of a `FUNC`/`SUB` handles every error
from the body:

```
FUNC readAge(input AS String) AS Integer
  LET n = toInt(input)                 ' auto-propagates on failure
  IF n < 0 THEN FAIL error(77050002, "negative")
  RETURN n

  TRAP(err)
    io::print("Bad age: " & err.message)
    RETURN 0                           ' the function succeeds with 0
  END TRAP
END FUNC
```

An inline `TRAP` handles one call at the call site; it must `RECOVER` a
replacement value or diverge:

```
IMPORT errorCode

LET user = getUser(id) TRAP(e)
  IF e.code = errorCode::ErrNotFound THEN RECOVER defaultUser
  FAIL e
END TRAP
```

More: `mfb man errors`.

## Resources and cleanup

A resource is bound with `RES` and has exactly one live owner. It is closed
automatically by lexical drop on every exit path — normal scope exit,
`RETURN`, `FAIL`, propagated errors, and `TRAP` routing. Plain values follow
the same ownership model, so cleanup is deterministic everywhere: when a scope
exits, its bindings are dropped in reverse declaration order.

```
FUNC firstLine(path AS String) AS String
  RES f = fs::openFile(path)     ' auto-propagates if the open fails
  RETURN fs::readLine(f)         ' f is closed here, success or failure
END FUNC
```

More: `mfb man fs`.

## Threads

Threads are isolated workers started from exported `ISOLATED FUNC` entry
points. They share nothing with their parent — no lexical scope, no mutable
collections, no resources — and communicate over bounded, typed message
queues: `thread::start`, `thread::send`, `thread::receive`, and
`thread::waitFor` to collect the result.

More: `mfb man thread`.

## Where to go next

- Coming from another language? `mfb man tour c`, `java`, `go`, `typescript`,
  or `python` walk the same ideas in your language's terms.
- `mfb man` — the package index; `mfb man <package> <function>` for any
  built-in.
- `mfb spec language` — the full language specification, including a complete
  worked example (`mfb spec language worked-example`).
- `mfb spec architecture` — how `mfb build` turns source into a native
  executable or a signed `.mfp` package.
