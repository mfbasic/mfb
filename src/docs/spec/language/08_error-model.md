# 8. Error Model — Implicit Failure, One `TRAP` per Function

## 8.1 The core rule

Every function call either **produces its value** or **fails with an `Error`**. On success the value is delivered directly (auto-unwrapped). On failure, control immediately transfers to the enclosing `TRAP`; if there is no `TRAP`, the function fails with that same `Error` to *its* caller.

```basic
LET x = toFloat(input)    ' on success: x = v
                            ' on failure: jump to TRAP, or fail to the caller carrying e
```

There is **no `TRY` keyword and no `GOTO`**. Propagation is the default behavior of calling a function; a call auto-propagates **unless** a postfix inline `TRAP` is attached to its expression (§8.4), which overrides the default for that one expression.

After named arguments and defaults are bound, function arguments are evaluated in parameter declaration order (§6). If any argument expression fails, later arguments are not evaluated and the error routes to the enclosing `TRAP` or propagates to the caller.

When an error path leaves a scope, any live resource bindings in that scope are closed by lexical drop (§14.7, §15) before the final error reaches the enclosing `TRAP` or caller.

## 8.2 Entering the error path

Use `FAIL` to fail explicitly with an `Error`:

```basic
IF n < 0 THEN FAIL error(77050002, "negative")
```

`FAIL e` routes to the enclosing `TRAP`; with no trap, the function fails to its caller carrying `e`.

## 8.3 The `TRAP` block — one keyword, two scopes

`TRAP(e)` traps errors in two scopes. A **function-level** `TRAP(e)` at the bottom of a `FUNC` traps every error from the body; an **inline** `TRAP(e)` attached postfix to a single expression traps just that expression (§8.4). Both bind an `Error` named by the parenthesized identifier, so no type annotation is needed.

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

Each `FUNC` may declare **at most one** function-level `TRAP`, at the bottom, after normal flow.

The `(err)` binding is **optional**. When the handler does not reference the caught error, write a bare `TRAP` (no parentheses); it behaves identically to a `TRAP(err)` whose `err` is unused. The error is still caught and remains internally bound, so `PROPAGATE` in a bare handler re-propagates the same `Error` — with its `code`, `message`, and `source` unchanged — exactly as a named handler does; the code below is equivalent to the `TRAP(err) / PROPAGATE` form:

```basic
FUNC relay() AS Integer
  RETURN mightFail()

  TRAP
    PROPAGATE                            ' re-raise the caught error, unnamed
  END TRAP
END FUNC
```

Trap outcomes:

| Statement | Meaning | Produces | Scope |
|-----------|---------|----------|-------|
| `RECOVER v` | bind `v` and continue after the trap | binding gets `v` | inline only |
| `RETURN v` | function succeeds | success value `v` (a bare `RETURN` in an `AS Nothing` function: `NOTHING`) | both |
| `PROPAGATE` | re-propagate the current `err` | failure carrying `err` | both |
| `FAIL e2` | replace/wrap the error | failure carrying `e2` | both |

The function-level `TRAP` is **diverging-only**: it has no `RECOVER`, because at function scope there is no failing statement to resume into. It may convert the error into the function's final success value with `RETURN` (a bare `RETURN` in a function declared `AS Nothing`), rethrow the same error with `PROPAGATE`, or replace it with `FAIL`. Once control enters a function-level `TRAP`, the failed expression is abandoned.

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

An inline `TRAP` is legal only as the value of a `LET`/`MUT`/`RES` binding, an assignment, or a bare expression statement. It scopes to exactly **one** expression, and covers **every** fallible call inside that expression, not only its outermost one: in `LET b = outer(inner(x)) TRAP(e)` a failure of `inner` runs the handler, and a `RECOVER` there skips the rest of the expression, so `outer` is never called. It equally covers **every raising operator** in that expression — a division by zero, an arithmetic overflow, a `Float` that overflows to infinity, a comparison whose `Integer` or `Float` operand is out of range for the `Fixed` it is converted to: in `LET d = two(1 / z, 2) TRAP(e)` the division's failure runs the handler and `two` is never called. Which node raised makes no difference. The one exception is a fallible call *or* a raising operator in a **short-circuited** operand — the right side of `AND`/`OR`, evaluated only when the left side does not already decide the result. It cannot be lifted ahead of the expression the way the desugar (§8.8) requires, so it is rejected (`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`); bind it to its own `TRAP` first. To wrap several *statements*, use the function-level `TRAP`. Every path through the handler must `RECOVER` or diverge; falling through to `END TRAP` is a compile error (there must be no path that leaves the binding unset). For a value-less trapped call (a function declared `AS Nothing`, user or built-in), `RECOVER` takes no operand.

The inline `(e)` binding is likewise **optional**: when the handler ignores the error, write a bare `TRAP` (e.g. `LET n = toInt(s) TRAP / RECOVER -1 / END TRAP`). As with the function-level form, the error stays internally bound, so a bare inline handler may still `PROPAGATE` it unchanged.

Use the same construct for ordinary absence — `RECOVER` the recoverable case, bail on the rest:

```basic
IMPORT errorCode

LET user = getUser(id) TRAP(e)
  IF e.code = errorCode::ErrNotFound THEN RECOVER defaultUser   ' use default, continue
  FAIL e                                                        ' any other error: bail
END TRAP
```

`MATCH` does not intercept call errors. A call used as a `MATCH` scrutinee auto-unwraps like every other call site; `MATCH` matches successful values, including literals, enums, and unions (§9).

## 8.5 `RETURN` semantics

`RETURN v` **always** means function success with the value `v`, whether it appears in the body or in the `TRAP`. It does not resume at the failed expression. In a function declared `AS Nothing`, a bare `RETURN` is the value-less early success exit, and falling through to `END FUNC` also succeeds (rules 8–9). `RETURN` never produces an error. `FAIL` and `PROPAGATE` produce errors.

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

A native runtime helper has no MFB-level origin, so the call site stamps its error: `emit_stamp_runtime_helper_error` [[src/codegen/error/emission/builder_error_emission.rs:emit_stamp_runtime_helper_error]] builds the call's `ErrorLoc` and parks the error as one owned `Error` block (result tag `RESULT_ERR_BLOCK_TAG`), which the catcher adopts. A helper normally returns a loose error with a fixed registered message. A helper that reports an OS status (the `audio` package, `mfb spec stdlib audio`) instead returns an already-parked block with a runtime-built message and no source, through `_mfb_rt_fail_status` [[src/codegen/error/emission/fail_status.rs:lower_fail_status_helper]]. Its call sites re-stamp that block with their own `ErrorLoc` through `_mfb_rt_restamp_parked_error` [[src/codegen/error/emission/fail_status.rs:lower_restamp_parked_error_helper]], on the propagating path and before an inline `TRAP` adopts it, so `e.source` is the call site either way. Only the packages in `PARKING_HELPER_PACKAGES` [[src/codegen/error/constants/error_constants.rs:PARKING_HELPER_PACKAGES]] carry that branch at their call sites, and the build rejects a helper outside the list that calls `_mfb_rt_fail_status`.

`code` is an ordinary `Integer` and is not validated: any value is accepted. Use a runtime registry code where one genuinely carries the right meaning (`errorCode::ErrInvalidArgument` for a bad argument, `errorCode::ErrPathNotFound` for a missing file). For a failure the registry does not model — a binding surfacing a C library's own error enum, say — use the generator `9` range (`90000000`–`99999999`) reserved for user-defined codes, rather than offsetting onto the `7-705-*` range where the value would collide with an unrelated standard name. See `./mfb spec diagnostics error-codes` for the convention and its caveats.

## 8.5b Reserved internal type names

`Result` and its success member `Ok` are the runtime's private representation of a fallible outcome (§8.8) and are not types a user may write. The resolver rejects them in any type position: naming `Result`, `Ok`, or a parameterized `Result OF ...` is reported as `TYPE_RESULT_NOT_USER_VISIBLE` ("`Result` is an internal type; declare the success type directly"), rather than falling through to the generic unknown-type error. The same resolver also treats the internal placeholder `Unknown` — and any currently active template parameter — as always resolved, so neither produces an unknown-type diagnostic. [[src/resolver/resolution.rs:resolve_type_name]]

Because these names still appear in compiler-internal positions, two resolution paths skip them deliberately so they are never re-checked as user types:

* Constructor resolution special-cases `Error`, `Ok`, and `Err`: when a constructor's type name is one of these, the type-name resolution step is skipped (only the arguments are resolved). [[src/resolver/resolution.rs:resolve_expression]]
* `MATCH`-pattern resolution skips the `Ok` union type: a union pattern whose type name is `Ok` is not resolved as a user type (the desugaring in §8.8 introduces `Ok` arms internally). [[src/resolver/resolution.rs:resolve_match_pattern]]

## 8.6 Rules

1. At most one function-level `TRAP` per function, at the bottom, after normal flow.
2. The trap payload is always `Error`; written `TRAP(err)` with no type. The same spelling is used for the inline and function-level forms. The `(err)` binding is optional in both forms: a bare `TRAP` handler runs without naming the error but still catches it (so `PROPAGATE` re-raises it unchanged).
3. The function-level trap block is entered by a failure from the normal body, including a checked operator, a call, `FAIL`, or propagation out of an inline handler. `FAIL` or `PROPAGATE` inside the function-level handler fails to the caller; it does not re-enter that handler. It is never reached by fall-through. Every route into it first closes the function's own resources (a `RES` parameter or a `FOR EACH` element is not the function's to close), so the handler may not use one: a read of such a binding in the handler is `TYPE_USE_AFTER_MOVE`, exactly as a read after `fs::close(h)` is. The function's other locals that have completed initialization stay live for the handler (see §8.6a for the current checking limitation); a `Thread` is one of the function's resources, so it is dropped on the way like any other. [[src/ir/verify/resources.rs:check_resource_moves]] [[src/codegen/engine/control/builder_exits.rs:trap_route_cleanups]]
4. `PROPAGATE` is valid inside a function-level `TRAP` or an inline `TRAP` handler (it refers to the current `err`). Elsewhere it is a compile error (`TYPE_PROPAGATE_REQUIRES_TRAP`); use `FAIL e` instead. `FAIL`'s operand must be `Error`-typed (`TYPE_FAIL_REQUIRES_ERROR`). [[src/rules/table.rs:312]] [[src/rules/table.rs:318]]
5. With no enclosing `TRAP`, any failure (from `FAIL` or an auto-propagated call) becomes the function's failure to its caller.
6. The completion checker must establish that a function-level `TRAP` has no reachable normal fall-through path: it must return, propagate/fail, or terminate the program. The conservative completion check below also accepts `EXIT PROGRAM`.
7. For a value-producing `FUNC`, the completion checker must establish that no reachable normal fall-through path remains. `RETURN value`, failure, and `EXIT PROGRAM` end the path; function fall-through is a compile error. See the conservative completion check below.
8. A function declared `AS Nothing` with no `TRAP` may fall through to `END FUNC`, which succeeds with `NOTHING`; a bare `RETURN` (or `RETURN NOTHING`) ends it early the same way. [[src/ir/verify/mod.rs:collect_diagnostics_with]]
9. A function declared `AS Nothing` with a function-level `TRAP` must end every normal path before the `TRAP` with `RETURN`, `FAIL error`, or `EXIT PROGRAM`. Falling through from the normal body into the `TRAP` is a compile error (`TYPE_TRAP_FALLTHROUGH`).
10. An executable entry point's uncaught failure terminates the process as an unhandled runtime error: the process exits with code `255`, and stderr receives `Error: <formatted code>` on one line and `<err.message>` on the next. [[src/codegen/error/constants/error_constants.rs:ENTRY_ERROR_PREFIX]] Give the entry point a `TRAP` for graceful handling.
11. An inline `TRAP` is legal only as the value of a `LET`/`MUT`/`RES` binding, an assignment, or a bare expression statement, and traps exactly one expression — every fallible call **and every raising operator** in it, however deeply nested. A `TRAP` is legal on **any call** — a built-in call is just a call. Two rejections: a scrutinee with no runtime call to trap anywhere in it — a call-free expression, or a **package constant** (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`); and a fallible call or raising operator in a short-circuited `AND`/`OR` operand — a raising comparison included, so `LET b = f(t AND fx < big) TRAP(e)` with `fx` a `Fixed` and `big` an `Integer` or `Float` is rejected — which cannot be evaluated ahead of the expression (`TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`). [[src/ir/shape.rs:note_short_circuited_binary_operator]] A raising operator widens what an inline `TRAP` *covers*, not what may *be* one: the scrutinee must still contain a runtime call, so `LET d = 1 / z TRAP(e)` is rejected while `LET d = f(1 / z) TRAP(e)` catches the division. An expression whose outermost node is not a call but which does contain a fallible call is accepted and traps it, operators included (`LET c = inner(x) + 1 TRAP(e)` catches both `inner`'s failure and the `+`'s overflow). Trapping changes where a failure goes, never whether it happens: a `Float` expression keeps exactly the observation boundaries it has untrapped (`./mfb spec language types`). A transient non-finite intermediate may still recover (`f(1.0 / (1e200 * 1e200)) TRAP(e)` passes `0.0` and runs no handler), a comparison operand is still not observed, and `a * b ± c` still contracts. A non-finite value that reaches a boundary inside the trapped expression — an argument, or the bound value — and a `Float` `MOD`/`^` domain error at its operator both run the handler, in source order. [[src/ir/lower.rs:trap_hoist_kind]] Trapping a provably-**infallible** inline-lowered built-in — `len`, `typeName`, every total `bits::*` op (all except the variable shifts `sl`/`sr`/`sra`, which trap `ErrInvalidArgument` on an out-of-range count), and the non-allocating pure-query / default-returning members `collections::contains`/`hasKey`/`getOr` — is **allowed** but the handler is dead code, flagged by the advisory warning `TYPE_INLINE_TRAP_DEAD_HANDLER` (the program still compiles and runs, returning the call's value; the handler never fires, exactly as a `TRAP` on an infallible user `FUNC` would behave). `toString` is fallible on every overload: each builds a new `String` and can raise `ErrOutOfMemory`, and `toString(<List OF Byte>)` also raises `ErrEncoding` on an ill-formed UTF-8 sequence, so a handler on it is live and it is never flagged `TYPE_INLINE_TRAP_DEAD_HANDLER`. The allocating `collections` members (`keys`, `values`, `append`, `prepend`, `removeKey`, `replace`, and the Set `add`/`remove`/`toList`) build a new collection and declare `ErrOutOfMemory`, so a handler on them is live, not dead. **Exception: the string-concatenation operator `&`.** `&` allocates a new `String` exactly like `toString` does, and a failed allocation inside it raises `ErrOutOfMemory` at runtime same as any other allocation — but, unlike `toString`, `&` is **not** treated as fallible by this census, by the inline-`TRAP`-liveness/dead-handler check above, by function fallibility (the "can fail" census below in this section)[[src/ir/verify/trap_init.rs:total_functions]], or by the declared-errors guards: `LET s = a & b TRAP(e)` is rejected with `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` — `&` is not a call and does not count as a raise site, so the scrutinee has nothing to trap (likewise `LET s = name() & b TRAP(e)` when `name` is infallible) — and `&` alone does not make an enclosing function fallible. This is a deliberate exception (owner decision 2026-10-04, bug-814's open question), not an oversight parallel to the one `toString` used to have (bug-807): treating every concatenation as fallible would force an error path through nearly all text-building code, where `toString`'s far rarer call sites could absorb that cost. No other MFBASIC operator allocates — there is no infix operator on `List`/`Map`/`Set`, only `collections::*` calls — so `&` is this exception's entire scope, not an example of a larger class. `mfb audit`'s own fallibility census has the identical blind spot for the same reason (`src/audit/collect/source.rs`'s `Expression::Binary` walk only follows nested calls, never the operator itself). **A member is infallible here only if it declares no error AND raises none** — the verdict is read off the member's declared errors, so a member that raises an error it does not declare is wrongly proved infallible and its handler is deleted; `strings::left`/`right`/`padLeft`/`padRight` each raise `ErrInvalidArgument` and now declare it. `collections::sum` is fallible and retains its handler. [[src/codegen/builtins/mod.rs:inline_builtin_is_infallible]] A built-in whose `Float` result can leave binary64 raises at the call, with or without a `TRAP` and wherever the call sits in an expression: `collections::sum` checks its `Float` total and raises `ErrFloatOverflow`, so `LET t = collections::sum(xs)`, `LET c = 1.0 / collections::sum(xs)` and either one under an inline `TRAP` all fail the same way. [[src/codegen/builtins/collections/func_sum.rs:lower_sum]] The same holds for an inline built-in's `Float` argument, which is an observation boundary like any call argument: `math::abs(x * x)` raises when `x * x` overflows. [[src/codegen/engine/value/builder_values.rs:lower_abi_inline_args]]
12. Every path through an inline `TRAP` handler must end in `RECOVER` or a diverging statement (`RETURN`, `FAIL`, `PROPAGATE`, or an `EXIT` form). Falling through to `END TRAP` is a compile error.
13. `RECOVER` is valid only inside an inline `TRAP` handler; it is a compile error in a function-level `TRAP` or anywhere else (`TYPE_RECOVER_OUTSIDE_INLINE_TRAP`). `RECOVER`'s value must be assignable to the trapped expression's success type; it carries a value iff that type is not `Nothing`. Supplying the wrong type, omitting a value when one is required, or supplying a value for a value-less trapped expression is `TYPE_RECOVER_TYPE_MISMATCH`. The handler binding is scoped to the handler block only. A call to a function declared `AS Nothing` — user or built-in, such as `fs::writeAll` — participates in auto-propagation and inline `TRAP` like any other call, and its handler ends in a bare `RECOVER`:

    ```basic
    fs::writeAll(f, "done") TRAP(e)
      io::print(e.message)
      RECOVER            ' the trapped call's success type is Nothing
    END TRAP
    ```
14. An inline `TRAP` on a fallible inline-lowered member traps the real runtime
    error, with the happy value auto-unwrapping and the handler running on failure.
    This covers the index/range members `collections::get`/`set`/`insert`/`removeAt`
    and `strings::mid`/`find` (index-out-of-range, range, not-found) **and** the
    callback members `collections::forEach`/`transform`/`filter`/`reduce` (a failing
    user callback routes its `Error` to the handler; the member frees its own
    loop-scoped intermediates first, so a partially-built result leaks nothing).
    Conversion built-ins like `toInt` and `toString(<List OF Byte>)` (whose UTF-8
    decode raises `ErrEncoding`), helper-backed built-ins like `fs::*`, and user
    `FUNC` calls all support inline `TRAP` the same way. (Infallible inline
    built-ins are also legal — see rule 11 — but their handler is dead code.)

### Completion and divergence

**Normal fall-through** reaches the end of a block without returning, raising an
error, or terminating the program. **Divergence**, relative to that continuation,
means execution does not reach it; this includes nontermination, but a call that
merely *may* fail or never return is not proof of divergence on every path.

The current completion checker is conservative: it recognizes `RETURN`, `FAIL`
and `EXIT PROGRAM`, an `IF` whose two arms both terminate, and an exhaustive
`MATCH` whose arms all terminate. `PROPAGATE` lowers to failure. Loops do not
establish completion, even `WHILE TRUE`; calls do not establish completion even
when their implementation never returns. A trailing function-level `TRAP` does
not terminate the normal body: the body and handler are checked separately.
These are the accepted proof forms, not a general termination analysis.
[[src/ir/verify/resources.rs:block_always_returns]]
[[src/ir/verify/ops.rs:check_ops]]

### 8.6a Handler-local visibility and initialization

Name visibility is not definite initialization. The body's top-level locals are
visible in a function-level handler, but the handler may **read** one only if it
is initialized on every failure edge that reaches the handler. A failure inside a
declaration's own initializer, or in any statement before it, enters the handler
before that local exists. So a local declared at or after the first statement
that can fail may not be read there (`TYPE_TRAP_LOCAL_UNINITIALIZED`). Parameters
are always readable. A local declared before anything can fail is readable even
if it is later reassigned, because a failed replacement leaves the earlier value
in place (§14.2).
[[src/ir/verify/trap_init.rs:locals_unbound_on_some_failure]]

"Can fail" is the same census the compiler uses for function fallibility: `FAIL`,
an arithmetic operator that can raise (a negative literal cannot), a comparison
that converts an `Integer` or `Float` operand to `Fixed` (an operand of unproven
type counts; see `./mfb spec language types`), a counted `FOR` (its increment
is checked), and a call, unless the callee is a built-in proven total or a
project function proven unable to let an error out.
[[src/ir/fallible.rs:comparison_can_raise]] An imported callee is assumed to fail. A call or operator
under an inline `TRAP` is captured by that handler and is not an edge into this
one. The census is applied to the IR, so a decoded package is held to the same
rule as source. [[src/ir/verify/trap_init.rs:total_functions]]

For example, when `mightFail()` precedes `LET value = 42`, a handler's
`RETURN value` is rejected; declaring `value` first makes it valid. The function's
own resources are governed by the trap-cleanup rule instead (rule 3): every
handler read of one is already a use after close.

## 8.7 Program entry point

An executable program starts at the root-package function named by `project.json` `entry`, defaulting to `main`. The entry point may be any one of these source shapes; empty parentheses are optional for zero-argument entries:

```basic
FUNC main AS Nothing
END FUNC

FUNC main(args AS List OF String) AS Nothing
END FUNC

FUNC main AS Integer
END FUNC

FUNC main(args AS List OF String) AS Integer
END FUNC
```

The actual name is the manifest entry value, so `main` above is illustrative. The accepted entry signatures are closed: the entry must have success type `Integer` or `Nothing`, and the only allowed parameter is one `List OF String` argument. Multiple matching entry declarations, a missing entry declaration in an executable, any other parameter list, or any `FUNC` entry return type other than `Integer` or `Nothing` are compile-time errors. [[src/manifest/entry.rs:validate_entry_point]]

When an entry declares `args AS List OF String`, the runtime passes the command-line argument vector as an owned immutable list. `collections::get(args, 0)` is the program name as invoked by the host. Subsequent elements are user arguments in order.

Process result mapping:

| Entry outcome | Process behavior |
|---------------|------------------|
| `FUNC ... AS Nothing` entry succeeds | Exit code `0`. |
| `FUNC ... AS Integer` succeeds with `n` | Exit code `n` for `0..=255`; an out-of-range result fails with `ErrOverflow`. [[src/codegen/engine/function/entry.rs:lower_program_entry]] |
| `EXIT PROGRAM n` executes | Run stack-wide lexical cleanup, then exit with code `n`. |
| Entry fails with an uncaught error carrying `err` | Write `Error: <formatted code>`, a newline, then `<err.message>` to stderr and exit with code `255`. |

Process environment access is provided by `os::getEnv`, `os::getEnvOr`, `os::hasEnv`, `os::setEnv`, `os::unsetEnv`, and `os::environ`; see `./mfb spec stdlib os`. [[src/codegen/builtins/os/mod.rs:register]]

The entry diagnostic formats the numeric code as `g-sss-nnnn` (for example,
`77050010` is printed as `7-705-0010`).

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

  LET x = f(g(y)) TRAP(e)  =>  MATCH g(y)     ' a NESTED call is covered too:
    <handler>                    CASE Ok(v1)   : MATCH f(v1)
  END TRAP                                        CASE Ok(v)    : bind x = v
                                                  CASE Error(e) : <handler>
                                                END MATCH
                                 CASE Error(e) : <handler>
                               END MATCH
                           ' the checks nest in evaluation order, so a handler
                           ' that RECOVERs skips the rest of the expression
                           ' (`f` is never called when `g` fails)

  LET x = f(a / b) TRAP(e) =>  MATCH CHECKED(a / b)   ' a raising OPERATOR too:
    <handler>                    CASE Ok(v1)   : MATCH f(v1)
  END TRAP                                              CASE Ok(v)    : bind x = v
                                                        CASE Error(e) : <handler>
                                                      END MATCH
                                   CASE Error(e) : <handler>
                                 END MATCH
                             ' CHECKED(expr) evaluates expr with its domain-error
                             ' exits captured instead of propagating, so an
                             ' overflow or a divide-by-zero yields Error(e) here
                             ' exactly as a failing call does. It is the same
                             ' lift, in the same evaluation order, and carries
                             ' the same restriction: an operand AND/OR evaluates
                             ' only conditionally cannot be lifted (§8.4).

  TRAP region (function-level bottom trap):
    PROPAGATE      =>  RETURN error result carrying the bound error
    FAIL e2        =>  RETURN error result carrying e2
    RETURN v       =>  RETURN Ok(v)
```

A call used as a `MATCH` scrutinee **is** rewritten like any other call (it auto-unwraps). No real exceptions, no stack unwinding, no jumps — pure structured value flow that serializes directly into the package's Binary Representation.

## See Also

* ./mfb spec memory fallible-call-abi — native result register ABI
* ./mfb man errors — error-code reference
