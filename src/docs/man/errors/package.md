# errors

The implicit-failure error model: Error, FAIL, and TRAP

## Synopsis

```
mfb man errors
```

## Imports

`errors` is a documentation topic, not an importable package. `Error` and
`ErrorLoc` are compiler-owned read-only records that are always in scope, the
`error` constructor is always available like `toString`, and `FAIL`, `TRAP`,
`PROPAGATE`, and `RECOVER` are language keywords — no `IMPORT` is needed.

## Description

MFBASIC has an implicit-failure error model: there is no `TRY`, no `GOTO`, and no
exceptions. Every function call either produces its value or fails with an
`Error`. On success the value is delivered directly (auto-unwrapped); on failure,
control immediately transfers to the enclosing `TRAP`, and if there is no `TRAP`
the function fails with that same `Error` to its caller. Propagation is the
default behavior of calling a function — a call auto-propagates unless an inline
`TRAP` is attached to its expression.

Failure always carries the single built-in `Error` record; there are no
per-function error types and no coercion. `Error` and its location record
`ErrorLoc` are read-only — a program may read their fields but may not construct
them with `[...]`, update them with `WITH`, or assign to them:

```
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

`error(code AS Integer, message AS String) AS Error` is the only way to build an
`Error` in source. The returned `Error.source` records the location of the
`error(...)` call and is not rewritten as the error propagates, so it always
points at the original failure site. See `mfb man general error`. There is no
built-in `Option`/`Maybe`; absence is represented by an error, conventionally
with a semantic code such as `errorCode::ErrNotFound`. `Result`, `Ok`, and `Err`
are the runtime's internal representation of a fallible outcome and are not types
a user may write or match.

## Result flow

A call in an ordinary expression auto-unwraps its success value:

```
LET value = toInt(input)
```

If `toInt` succeeds, `value` receives the `Integer`. If `toInt` fails, the rest
of the current expression is skipped and the `Error` routes to the nearest
`TRAP` or propagates out of the function. Function arguments are evaluated left
to right, so if an earlier argument fails, later arguments are not evaluated.
When an error path leaves a scope, any live resource bindings in that scope are
closed by lexical drop before the error reaches the enclosing `TRAP` or caller.

## FAIL

`FAIL` fails explicitly with an `Error`. Its operand must be `Error`-typed
(`TYPE_FAIL_REQUIRES_ERROR`):

```
IF n < 0 THEN FAIL error(77050002, "negative")
```

`FAIL e` routes to the enclosing `TRAP`; with no trap, the function fails to its
caller carrying `e`.

## Local handling with an inline TRAP

To handle an error at the call site instead of auto-propagating, attach a postfix
inline `TRAP(e)` to the expression. The success value auto-unwraps into the
binding exactly as a normal call; on error the handler runs with the bound
`Error` and must either `RECOVER` a value (bound into the binding, then continue
at the statement after `END TRAP`) or diverge with `RETURN`, `FAIL`, `PROPAGATE`,
or an `EXIT` form:

```
RES f = fs::openFile(path) TRAP(e)
  io::print("could not open: " & e.message)
  RECOVER fs::openFile(fallbackPath)   ' supply a File and continue
END TRAP
```

An inline `TRAP` is legal only as the value of a `LET`/`MUT` binding, an
assignment, or a bare expression statement, and traps exactly one expression.
Every path through the handler must `RECOVER` or diverge; falling through to
`END TRAP` is a compile error. For a value-less trapped call (a `SUB` or a
fallible effect-only built-in) `RECOVER` takes no operand. Use it for ordinary
absence too — `RECOVER` the recoverable case and bail on the rest:

```
IMPORT errorCode

LET user = getUser(id) TRAP(e)
  IF e.code = errorCode::ErrNotFound THEN RECOVER defaultUser
  FAIL e
END TRAP
```

## Function-level TRAP

Each `FUNC` or `SUB` may declare at most one function-level `TRAP(e)`, at the
bottom of the function after normal flow. It traps every error from the body and
is diverging-only — it has no `RECOVER`, because at function scope there is no
failing statement to resume into:

```
FUNC readAge(input AS String) AS Integer
  LET n = toInt(input)                 ' auto-propagates on failure
  IF n < 0 THEN FAIL error(77050002, "negative")
  RETURN n

  TRAP(err)
    io::print("Bad age: " & err.message)
    RETURN 0                           ' function succeeds with a default
  END TRAP
END FUNC
```

The function-level `TRAP` is reached only via `FAIL` in the body, an
auto-propagated failure from a call, or `FAIL`/`PROPAGATE` inside the trap —
never by fall-through. Every body path before it must end with `RETURN` (in a
`FUNC`) or `EXIT SUB` (in a `SUB`) or `FAIL`.

## Trap outcomes

| Statement | Meaning | Scope |
| --- | --- | --- |
| `RECOVER v` | bind `v` and continue after the trap | inline `TRAP` only |
| `RETURN v` | the function succeeds with value `v` | `FUNC` only |
| `EXIT SUB` | the sub succeeds with no value | `SUB` only |
| `PROPAGATE` | re-propagate the current error to the caller | both |
| `FAIL e2` | replace or wrap the error and fail to the caller | both |

`PROPAGATE` is valid only inside a `TRAP` handler; elsewhere it is a compile
error (`TYPE_PROPAGATE_REQUIRES_TRAP`). `RECOVER` is valid only inside an inline
`TRAP` (`TYPE_RECOVER_OUTSIDE_INLINE_TRAP`). Once control enters a function-level
`TRAP`, the failed expression is abandoned — there is no resume.

## MATCH versus TRAP

`MATCH` does not intercept call errors. A call used as a `MATCH` scrutinee
auto-unwraps like every other call site, and `MATCH` matches enum and union
values only; `CASE Ok`, `CASE Err`, and `CASE Error` are not valid arms
(`TYPE_RESULT_NOT_MATCHABLE`). A failure is never matched, only trapped. Use an
inline `TRAP` to handle one call's error locally, and a function-level `TRAP` for
one central policy over several calls.

## Program entry points

An executable entry point may handle errors with `TRAP` like any other function.
If an entry point returns an uncaught `Err`, the runtime writes
`Code: <err.code> Message: <err.message>` to stderr and exits with status 255.

## Errors

No errors.

## See also

- `mfb man general error`
- `mfb man flow match`
- `mfb man types`
- `mfb spec diagnostics error-codes`
