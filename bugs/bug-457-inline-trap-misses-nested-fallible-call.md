# bug-457: an inline `TRAP` silently fails to cover a fallible call NESTED inside the trapped expression

Last updated: 2026-08-30
Effort: medium (1h–3h)
Severity: HIGH
Class: Miscompile (error handling silently defeated; no diagnostic)

Status: Open
Regression Test: — (a Phase 1 fixture must pin `LET b = outer(inner()) TRAP(e) … END TRAP`
catching `inner`'s error; today it escapes the handler entirely)

An inline `TRAP` catches a fallible call only when that call is the **outermost**
node of the trapped expression. A fallible call nested inside another call
propagates *past* the handler to the function-level trap — the handler never
runs, and to the author the error handler simply does not work. There is no
diagnostic; the code compiles and the error surfaces at runtime somewhere else.

Minimal repro (`/tmp/trapnest`), macOS aarch64:

```basic
FUNC inner(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(90000001, "inner failed")
  RETURN n * 2
END FUNC

FUNC outer(n AS Integer) AS Integer
  RETURN n + 1
END FUNC

FUNC main AS Integer
  LET a = inner(-1) TRAP(e)              ' outermost: caught
    io::print("caught outermost code=" & toString(e.code))
    RECOVER 0
  END TRAP
  io::print("a=" & toString(a))

  LET b = outer(inner(-1)) TRAP(e)       ' nested: NOT caught
    io::print("caught nested code=" & toString(e.code))
    RECOVER 0
  END TRAP
  io::print("b=" & toString(b))
  RETURN 0
END FUNC
```

Observed:

```
caught outermost code=90000001
a=0
Error: 9-000-0001
inner failed
[exit 255]
```

Expected: `caught nested code=90000001` then `b=0`, exit 0.

**Pre-existing, not a plan-110 regression.** Reproduced with the main-tip
compiler at `f79f6212a` in a detached worktree (`/tmp/p110-maintip`), byte-identical
output. plan-110-D only *exposed* it: commit 26e5d057c removed `tls::readText`
and rewrote `tests/rt-behavior/tls/tls-poll-rt`'s

```basic
LET chunk AS String = tls::readText(conn, 4096) TRAP(e) …
```

into

```basic
LET chunk AS String = encoding::utf8Decode(tls::read(conn, 4096)) TRAP(e) …
```

which moved the fallible call one level in. The fixture's EOF-terminated read
loop then stopped terminating: `tls::read`'s `ErrConnectionClosed` escaped the
`TRAP`, propagated out of `fetch()` and out of `main`, and the program died with
`7-707-0004` instead of printing `loop=TRUE`.

## Why this is a bug and not the documented behaviour

`mfb spec language error-model` §8.8 desugars a call as

```text
call g(x)  =>  MATCH g(x)
                 CASE Ok(v)    : v
                 CASE Error(e) : PROPAGATE to enclosing TRAP region
```

An inline `TRAP` **is** a TRAP region, so a nested call's error must propagate to
it, not past it. §8.4's "it scopes to exactly one expression" says the same thing
in the other direction: the scope is the *expression*, and `outer(inner(-1))` is
one expression. (§8.4 does say "to wrap several fallible calls, use the
function-level `TRAP`" — but the repro has exactly ONE fallible call, and it is
still missed.)

## Cause

`src/ir/lower.rs:lower_inline_trap` converts only the top-level node:

```rust
let raw = lower_expression(inner, locals, context);
let call_result = match raw {
    IrValue::Call { target, args, loc, .. } => IrValue::CallResult { … },
    other => other,          // <- a nested Call stays a plain Call and auto-propagates
};
```

so exactly one `Result` is produced and checked, for the outermost call only.

## What a fix must produce

Every fallible call in the trapped expression routes to the handler, and a
`RECOVER` skips the remainder of the expression. Because the IR is structured
(no jumps), that means nesting one check per fallible call, evaluation order
outermost-last:

```text
Bind $r1 = CallResult(inner(-1))
If ResultIsOk($r1) {
    Bind $v1 = ResultValue($r1)
    Bind $r0 = CallResult(outer($v1))
    If ResultIsOk($r0) { $slot = ResultValue($r0) } else { <handler> }
} else { <handler> }
Bind b = $slot
```

Two constraints worth stating before coding:

* **Keep the single-fallible-call-at-outermost shape byte-identical.** That is
  the overwhelmingly common case; if the fix only nests when there IS a nested
  fallible call, existing IR goldens do not churn and the diff is confined to
  code that was mis-lowering.
* **`LowerContext` has no fallibility map today** (`function_returns` /
  `function_types` / `function_params` only), and every user `FUNC` can `FAIL`,
  so "which nested calls are fallible" needs a source. `builtins::inline_builtin_is_infallible`
  covers the built-in side (it already backs the dead-handler warning in
  `ir/verify/resources.rs:698`); user functions need the equivalent of
  `audit/collect/source.rs:fallible_functions`. Over-approximating (treat every
  nested call as fallible) is correct but would add a check around nested calls
  everywhere and churn goldens broadly — hence the first constraint.

An acceptable interim behaviour, if the full fix is deferred, is a **diagnostic**:
reject an inline `TRAP` whose expression contains a fallible call that is not the
outermost node, telling the author to bind the inner call first. That converts a
silent miscompile into a compile error. It must not ship as the final answer —
`f(g())` is reasonable code and §8.8 says it should work.

## Blast radius

Any `LET x = f(g()) TRAP … END TRAP` where `g` can fail. Two sites are known:

* `tests/rt-behavior/tls/tls-poll-rt` — introduced by 26e5d057c, **fixed** by
  binding the read before decoding (the trapped call is outermost again). It is
  the reproduction that found this bug, not an instance to fix again.
* `examples/browser/fetch/src/lib.mfb:141` —
  `http::startRead(net::toUrl(resolveLocation(base, href))) TRAP(e) CONTINUE FOR END TRAP`.
  Pre-existing and **not fixed**: if `net::toUrl` or `resolveLocation` fails, the
  `CONTINUE FOR` never runs and the error escapes `fetchStyles`. Left alone
  deliberately — the correct rewrite depends on whether a failed
  `resolveLocation` should skip the sheet or abort the batch, which is a
  behavioural question about the example, and changing it before the compiler
  fix would hide a second instance of the bug the fix needs to cover.

Grep for inline `TRAP` statements whose expression has a nested call before
fixing, to size the full set of programs currently mis-handling errors.

References: `src/ir/lower.rs:lower_inline_trap`;
`src/docs/spec/language/08_error-model.md` §8.4, §8.8;
`tests/rt-behavior/tls/tls-poll-rt` (the fixture that exposed it);
plan-110-D §C11.
