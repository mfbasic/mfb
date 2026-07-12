# 8. Error Model — Implicit Failure, One `TRAP` per Function

## 8.1 The core rule

Every function call either **produces its value** or **fails with an `Error`**. On success the value is delivered directly (auto-unwrapped). On failure, control immediately transfers to the enclosing `TRAP`; if there is no `TRAP`, the function fails with that same `Error` to *its* caller.

```basic
LET x = toFloat(input)    ' on success: x = v
                            ' on failure: jump to TRAP, or fail to the caller carrying e
```

There is **no `TRY` keyword and no `GOTO`**. Propagation is the default behavior of calling a function; a call auto-propagates **unless** a postfix inline `TRAP` is attached to its expression (§8.4), which overrides the default for that one expression.

Function arguments are evaluated left to right. If any argument expression fails, later arguments are not evaluated and the error routes to the enclosing `TRAP` or propagates to the caller.

When an error path leaves a scope, any live resource bindings in that scope are closed by lexical drop (§14.7, §15) before the final error reaches the enclosing `TRAP` or caller.

## 8.2 Entering the error path

Use `FAIL` to fail explicitly with an `Error`:

```basic
IF n < 0 THEN FAIL error(77050002, "negative")
```

`FAIL e` routes to the enclosing `TRAP`; with no trap, the function fails to its caller carrying `e`.

## 8.3 The `TRAP` block — one keyword, two scopes

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

## 8.4 Local error handling (inline `TRAP`)

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

## 8.5 `RETURN` semantics

`RETURN v` **always** means function success with the value `v`, whether it appears in the body or in the `TRAP`. It does not resume at the failed expression. `RETURN` is forbidden in a `SUB`; use `EXIT SUB` for a value-less early success exit. A `SUB` with no `TRAP` may fall through to `END SUB`, which succeeds. `RETURN` never produces an error. `FAIL` and `PROPAGATE` produce errors.

## 8.5a `Error` and `ErrorLoc` record shapes

The trap payload is always the built-in read-only record `Error`:

| Type | Field | Field type |
|------|-------|------------|
| `Error` | `code` | `Integer` |
| `Error` | `message` | `String` |
| `Error` | `source` | `ErrorLoc` |
| `ErrorLoc` | `filename` | `String` |
| `ErrorLoc` | `line` | `Integer` |
| `ErrorLoc` | `char` | `Integer` |

Both are read-only: an `Error`/`ErrorLoc` cannot be user-constructed (`TYPE_READ_ONLY_RECORD_CONSTRUCTOR`) or `WITH`-updated (`TYPE_READ_ONLY_RECORD_UPDATE`), and accessing any other field is a compile error (`TYPE_UNKNOWN_FIELD`). `Error.source` is stamped at the origin where the error is created (by `error(...)`, a trapping built-in, or a failing call) and is **not** rewritten as the error propagates, so it always points at the original failure site. `error(code AS Integer, message AS String) AS Error` is the only way to build an `Error` in source.

## 8.5b Reserved internal type names

`Result` and its success member `Ok` are the runtime's private representation of a fallible outcome (§8.8) and are not types a user may write. The resolver rejects them in any type position: naming `Result`, `Ok`, or a parameterized `Result OF ...` is reported as `TYPE_RESULT_NOT_USER_VISIBLE` ("`Result` is an internal type; declare the success type directly"), rather than falling through to the generic unknown-type error. The same resolver also treats the internal placeholder `Unknown` — and any currently active template parameter — as always resolved, so neither produces an unknown-type diagnostic. [[src/resolver/resolution.rs:resolve_type_name]]

Because these names still appear in compiler-internal positions, two resolution paths skip them deliberately so they are never re-checked as user types:

* Constructor resolution special-cases `Error`, `Ok`, and `Err`: when a constructor's type name is one of these, the type-name resolution step is skipped (only the arguments are resolved). [[src/resolver/resolution.rs:resolve_expression]]
* `MATCH`-pattern resolution skips the `Ok` union type: a union pattern whose type name is `Ok` is not resolved as a user type (the desugaring in §8.8 introduces `Ok` arms internally). [[src/resolver/resolution.rs:resolve_match_pattern]]

## 8.6 Rules

1. At most one function-level `TRAP` per function, at the bottom, after normal flow.
2. The trap payload is always `Error`; written `TRAP(err)` with no type. The same spelling is used for the inline and function-level forms.
3. The function-level trap block is reachable only via `FAIL` (in the body), an auto-propagated failure from a call, or `FAIL`/`PROPAGATE` inside the trap. It is never reached by fall-through.
4. `PROPAGATE` is valid inside a function-level `TRAP` or an inline `TRAP` handler (it refers to the current `err`). Elsewhere it is a compile error (`TYPE_PROPAGATE_REQUIRES_TRAP`); use `FAIL e` instead. `FAIL`'s operand must be `Error`-typed (`TYPE_FAIL_REQUIRES_ERROR`). [[src/rules/table.rs:312]] [[src/rules/table.rs:318]]
5. With no enclosing `TRAP`, any failure (from `FAIL` or an auto-propagated call) becomes the function's failure to its caller.
6. Every function-level `TRAP` path must end in `RETURN` (for a `FUNC`), `EXIT SUB` (for a `SUB`), `PROPAGATE`, or `FAIL`. Trap fall-through is a compile error.
7. Every `FUNC` path must end in `RETURN value` or `FAIL error`. Function fall-through is a compile error.
8. A `SUB` with no `TRAP` may fall through to `END SUB`, which succeeds (value-less).
9. A `SUB` with a `TRAP` must end every normal path before the `TRAP` with `EXIT SUB` or `FAIL error`. Falling through from the normal body into the `TRAP` is a compile error.
10. An executable entry point's uncaught failure terminates the process as an unhandled runtime error: the process exits with code `255`, and stderr receives `Code: <err.code> Message: <err.message>`. Give the entry point a `TRAP` for graceful handling.
11. An inline `TRAP` is legal only as the value of a `LET`/`MUT` binding, an assignment, or a bare expression statement, and traps exactly one expression. A `TRAP` is legal on **any call** — a built-in call is just a call. The only rejection is a scrutinee with no runtime call to trap: a non-call expression, or a **package constant** (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`). Trapping a provably-**infallible** inline-lowered built-in — `len`, `toString`, `typeName`, every total `bits::*` op (all except the variable shifts `sl`/`sr`/`sra`, which trap `ErrInvalidArgument` on an out-of-range count), and the pure-query / default-returning / growth-only members `collections::contains`/`hasKey`/`keys`/`values`/`sum`/`getOr`/`append`/`prepend`/`removeKey` and `strings::replace` — is **allowed** but the handler is dead code, flagged by the advisory warning `TYPE_INLINE_TRAP_DEAD_HANDLER` (the program still compiles and runs, returning the call's value; the handler never fires, exactly as a `TRAP` on an infallible user `FUNC` would behave).
12. Every path through an inline `TRAP` handler must end in `RECOVER` or a diverging statement (`RETURN`, `FAIL`, `PROPAGATE`, or an `EXIT` form). Falling through to `END TRAP` is a compile error.
13. `RECOVER` is valid only inside an inline `TRAP` handler; it is a compile error in a function-level `TRAP` or anywhere else (`TYPE_RECOVER_OUTSIDE_INLINE_TRAP`). `RECOVER`'s value must be assignable to the trapped expression's success type; it carries a value iff that type is not `Nothing`. Supplying the wrong type, omitting a value when one is required, or supplying a value for a value-less trapped expression is `TYPE_RECOVER_TYPE_MISMATCH`. The handler binding is scoped to the handler block only.
14. An inline `TRAP` on a fallible inline-lowered member traps the real runtime
    error, with the happy value auto-unwrapping and the handler running on failure.
    This covers the index/range members `collections::get`/`set`/`insert`/`removeAt`
    and `strings::mid`/`find` (index-out-of-range, range, not-found) **and** the
    callback members `collections::forEach`/`transform`/`filter`/`reduce` (a failing
    user callback routes its `Error` to the handler; the member frees its own
    loop-scoped intermediates first, so a partially-built result leaks nothing).
    Conversion built-ins like `toInt`, helper-backed built-ins like `fs::*`, and user
    `FUNC`/`SUB` calls all support inline `TRAP` the same way. (Infallible inline
    built-ins are also legal — see rule 11 — but their handler is dead code.)

## 8.7 Program entry point

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

The actual name is the manifest entry value, so `main` above is illustrative. The accepted entry signatures are closed: a `SUB` entry has success type `Nothing`, a `FUNC` entry must have success type `Integer`, and the only allowed parameter is one `List OF String` argument. Multiple matching entry declarations, a missing entry declaration in an executable, any other parameter list, or any non-`Integer` `FUNC` entry return type are compile-time errors. [[src/manifest/entry.rs:validate_entry_point]]

When an entry declares `args AS List OF String`, the runtime passes the command-line argument vector as an owned immutable list. `collections::get(args, 0)` is the program name as invoked by the host. Subsequent elements are user arguments in order.

Process result mapping:

| Entry outcome | Process behavior |
|---------------|------------------|
| `SUB` entry succeeds | Exit code `0`. |
| `FUNC ... AS Integer` succeeds with `n` | Exit code `n`. Implementations must reject or fail values outside the host process exit-code range. |
| `EXIT PROGRAM n` executes | Run stack-wide lexical cleanup, then exit with code `n`. |
| Entry fails with an uncaught error carrying `err` | Write `Code: <err.code> Message: <err.message>` to stderr and exit with code `255`. |

Environment access outside command-line arguments is outside the core language specification and may be provided by a future standard package.

## 8.8 Desugaring

This sketch is **compiler-internal**: it describes how source desugars into
**structured IR** (the same IR that is serialized as the package's Binary Representation).
`Result`, `Ok`, and `Err` below are the runtime's private representation of a
fallible outcome (§4.4), not types a user writes; the concrete native
register-level result form (success/error/exit tags) is specified by
`./mfb spec memory fallible-call-abi`. The control flow is
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

## See Also

* ./mfb spec memory fallible-call-abi — native result register ABI
* ./mfb man errors — error-code reference
