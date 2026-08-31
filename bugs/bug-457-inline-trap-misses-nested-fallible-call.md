# bug-457: an inline `TRAP` silently fails to cover a fallible call NESTED inside the trapped expression

Last updated: 2026-08-30
Effort: medium (1h–3h)
Severity: HIGH
Class: Miscompile (error handling silently defeated; no diagnostic)

Status: FIXED (daa6c8d35, 7a6e5c77f)
Regression Test: `tests/rt_inline_trap_nested_call.rs` (19 cases), plus the two
behavioural fixtures below. Of the 19: **14 measured RED before the fix** (13 in
the first run against this tree, plus the overload case rebuilt with the
merge-base compiler); **1 measured RED against the intermediate state** the
second commit repaired (`a_nested_conversion_whose_handler_reads_the_error` —
green on the merge-base, since there the root conversion's own failure was
caught); and **4 controls** green throughout (outermost call, call-free scrutinee
still rejected, success path, and the conversion whose handler ignores the
error).

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

## STATUS: FIXED (daa6c8d35, 7a6e5c77f)

Reproduced first, exactly as documented: `outer(inner(-1)) TRAP(e)` let
`9-000-0001` past the handler and exited 255 on macOS aarch64, with the
mechanism confirmed at `src/ir/lower.rs:lower_inline_trap` (`other => other` left
a nested `Call` un-checked).

**The fix.** `lower_inline_trap` now lifts every *unconditionally evaluated*
fallible call in the scrutinee into its own `CallResult` + `If ResultIsOk` check
ahead of the residual expression, nested in evaluation order so a failure skips
the rest of the expression. Two deviations from the shape this doc sketched, both
deliberate:

* **The handler is emitted once**, behind a shared `$trap_failed` flag, instead of
  cloned into every check's `else` as the sketch showed. Cloning duplicates the
  handler's own `ir::verify` diagnostics (a type error in the handler would be
  reported N+1 times) and its lowered temps. The flag reproduces `RECOVER`'s
  fall-through — recovery assigns the slot and continues to the delivery below —
  with a single copy.
* **Calls are lifted up to and including the LAST fallible one, not only the
  fallible ones.** The lifted binds all run before the residual expression, so a
  side-effecting infallible call left behind would move *after* a fallible call it
  used to precede. `hoisting_preserves_left_to_right_argument_order` pins this,
  and `user-function-default-args-result-valid` — three effectful `mark(…)`
  arguments including two defaults — proves it end to end: its runtime output is
  byte-identical while its IR is not.

A scrutinee with nothing to lift lowers byte-for-byte as before, so the common
single-call shape (and the `ir::verify` shape that reports
TYPE_INLINE_TRAP_REQUIRES_FALLIBLE / _DEAD_HANDLER on it) is untouched. Two
related shapes now work that previously did not, and are pinned: an expression
whose *outermost* node is not a call but which contains one (`inner(x) + 1`), and
a nested fallible call under a provably-infallible root (`toString(inner(x))`),
where the handler used to be wrongly flagged dead.

**The fallibility oracle** the doc asked for is `src/ir/fallible.rs`, shared by
lowering and the shape pass. A call is fallible unless PROVEN otherwise: an
inline built-in on `builtins::inline_builtin_is_infallible`'s census, or a
project function a fixpoint proves cannot let an error escape. Everything else is
fallible — over-approximating only adds a check whose error branch is dead, while
under-approximating drops the error on the floor. It is deliberately NOT
`audit/collect/source.rs:fallible_functions`, whose hand-curated per-package
census is tuned against over-*reporting* to a human; a noisy report is not a
miscompile.

**The one shape that cannot be desugared** is a fallible call in a
short-circuited `AND`/`OR` operand: lifting it ahead of the expression would call
it unconditionally (`AND`/`OR` short-circuiting was verified at runtime, not
assumed), and keeping it in place while a `RECOVER` must skip the remainder needs
the whole continuation duplicated per operand. That is now the error
`2-203-0137 TYPE_INLINE_TRAP_SHORT_CIRCUIT_CALL`, emitted from `ir::shape`
(lowering erases the operand structure). This is the doc's sanctioned interim
behaviour applied to the residual corner only — the general `f(g())` case is
fixed, not diagnosed.

**Blast radius, measured.** A census of inline `TRAP` scrutinees with a nested
call across `tests/`, `examples/` and `src/` found 10 sites. The compiler fix
covers all of them with no source change, including the two the doc named:

* `examples/browser/fetch/src/lib.mfb:141` — now lowers to a three-link check
  chain (`resolveLocation` → `net::toUrl` → `http::startRead`), so a failed
  `resolveLocation` runs `CONTINUE FOR` as written. The doc left this "not fixed"
  pending a behavioural decision about the example; the compiler fix answers it
  without touching the example.
* `tests/rt-behavior/tls/tls-poll-rt` — already worked around; unaffected.

**A third instance the doc did not know about**, found by that census:
`tests/rt-behavior/tcp/tcp-bounded-accept-blocking-rt`, whose committed golden
RECORDED THE BUG. Its source ends

```basic
LET quiet = encoding::utf8Decode(tcp::read(conn, 16)) TRAP(e)
  io::print("explicit read timeout still fires")
  RETURN 0
END TRAP
```

yet `golden/build.log` ended `Error: 7-705-0008 / Operation did not complete
before its deadline. / [exit 255]` — the timeout escaping its own inline `TRAP`
and killing the process. Regenerated to `explicit read timeout still fires` /
`[exit 0]`. The four-question gate (AGENTS.md): written by `008d745c2`
("plan-110-B: the tcp package, and a pre-existing accept bug it uncovered") to
prove a bounded-accepted socket still honours an explicit read timeout; nothing
else reads it; and the proof it was wrong is the golden itself contradicting
`mfb spec language error-model` §8.8, reproduced independently.

**Attribution.** `scripts/test-accept.sh` driven by a release compiler built from
the merge-base (`52d60054d`) reproduces BOTH changed goldens with 0 mismatches,
so both diffs are caused by this change and neither is pre-existing drift. The
overload case was measured the same way: `outer(len(r))` with a failing user
`FUNC len(r AS Ring)` exits 255 with an uncaught `9-000-0003` under the
merge-base compiler.

**Spec.** `src/docs/spec/language/08_error-model.md` §8.4, rule 11 and the §8.8
desugar sketch now state that an inline `TRAP` covers every fallible call in the
trapped expression, name the short-circuit exception, and show the nested
desugar. `2-203-0137` is registered in `src/rules/table.rs` and the
`diagnostics rule-codes` table.

**A regression the first commit caused, fixed in `7a6e5c77f`.** The desugar's new
shape broke a codegen analysis that was silently coupled to the old one.
`function_lowering.rs:trap_discard_error_results` (plan-64-I) decides whether an
inline-*conversion* `CallResult`'s `Error` is ever observed, so
`emit_error_register_return` may emit only the result tag and skip building the
ErrorLoc + flat `Error` block. It located the paired error local by matching one
op shape — `Bind err = ResultError(result)`. The check chain reports through a
shared slot and emits `Assign $trap_errN = ResultError(result)` instead, which
was invisible to it: every chained `to*` conversion was classified
error-discardable, codegen emitted the tag with no `Error` block, and the
`Assign` then read one that did not exist. The process died on a signal — no exit
code, no stdout, no stderr.

Three things about how it was caught are worth keeping:

* **Only `scripts/test-accept.sh` saw it.** The full `cargo test` was green (the
  acceptance harness is not in it) and `artifact-gate all` reported only the two
  expected `.ir` diffs — it is execution-free and skips `tests/acceptance`
  outright for want of a `golden/` dir. Both documented blind spots
  (`.ai/testing-gates.md`), hit at once.
* **A signal death prints nothing**, so `tests/acceptance` simply stopped mid-run
  with no `[F]` marker. The last line printed names the group, not the failure.
* **Only the `to*` conversions are plan-64-I candidates**
  (`is_trap_discard_conversion`), which is why all 17 user-`FUNC` tests passed
  while the 732-case acceptance app died.

Fixed by matching both op shapes. RED-verified in both directions: with only the
`Assign` arm removed, `a_nested_conversion_whose_handler_reads_the_error` dies
`exit None` (killed by a signal, no output) while
`a_nested_conversion_whose_handler_ignores_the_error` stays green — so the fix
did not simply switch the optimisation off. `mfb test tests/acceptance` is back
to 732/732, matching the merge-base.

The transferable lesson is recorded in auto-memory: an elision analysis that
pattern-matches a desugar's emitted op shape is silently coupled to that
desugar, and a miss is a **miscompile**, not a lost optimisation.

**One thing deliberately NOT fixed here — filed as bug-469.** An *operator* that
raises inside the trapped expression (`two(1 / z, 2) TRAP(e)`) is the same class
of escape but is not a call, so it is outside this doc's scope ("every fallible
call in the trapped expression") and outside the fix. Reproduced with the fixed
compiler: it still exits 255 with an uncaught `7-705-0002`, while the same
division caught at the *root* (`divz(1, 0) TRAP(e)`) works — so the error is
trappable and the escape is positional. It is not a widening of this fix:
codegen's `raw_result_capture` is a per-VALUE redirect, and this desugar lifts
nested calls into separate `Bind` ops that sit outside any capture, so covering
operators needs a trap-*region* notion the jump-free IR does not have. See
`bugs/bug-469-inline-trap-misses-raising-operator.md`, which also records that
bug-467 (SIGPIPE) must be fixed first or the region work will look like it
failed on socket writes when it had not.

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
