# bug-469: an inline `TRAP` does not cover a raising **operator** inside the trapped expression

Last updated: 2026-08-30
Effort: large (a codegen capability, not a lowering tweak)
Severity: MEDIUM (miscompile class; narrower shape than bug-457)
Class: Miscompile (error handling silently defeated; no diagnostic)

Status: Open
Regression Test: — (must pin `LET d = two(1 / z, 2) TRAP(e) … END TRAP` catching
the division's `7-705-0002`; today it escapes the handler entirely)

An inline `TRAP` covers every fallible **call** in the trapped expression
(bug-457, fixed in `daa6c8d35`). It does **not** cover an *operator* in that same
expression that raises — a division by zero, an arithmetic overflow. The error
propagates past the handler to the function-level trap exactly as a nested call
used to, with no diagnostic.

This is the same escape class as bug-457 but a different mechanism, and it was
deliberately left out of that fix rather than bolted on — see "Why bug-457's fix
does not reach it".

## Failing reproduction

`/tmp/b457op`, macOS aarch64, built with the bug-457 fix in place
(`daa6c8d35`), so this is *not* a shape that fix was expected to cover:

```basic
IMPORT io

FUNC two(a AS Integer, b AS Integer) AS Integer
  RETURN a * 100 + b
END FUNC

FUNC main AS Integer
  MUT z AS Integer = 0
  LET d = two(1 / z, 2) TRAP(e)
    io::print("caught operator code=" & toString(e.code))
    RECOVER -1
  END TRAP
  io::print("d=" & toString(d))
  RETURN 0
END FUNC
```

Observed:

```
Error: 7-705-0002
Argument value is not valid for the requested operation.
[exit 255]
```

Expected: `caught operator code=77050002` (the code as `toString`ed — the same
value the root-position case below prints) then `d=-1`, exit 0.

`z` is a `MUT` local rather than a literal `0` so the division is not
constant-folded at compile time.

## The error IS trappable — the escape is positional

The same division caught at the **root** of the trapped expression works today,
which is what makes this a miscompile rather than an unsupported feature:

```basic
FUNC divz(a AS Integer, b AS Integer) AS Integer
  RETURN a / b
END FUNC
...
LET d = divz(1, 0) TRAP(e)
  io::print("caught div code=" & toString(e.code))
  RECOVER -1
END TRAP
```

prints `caught div code=77050002` and `d=-1`, exit 0. The error escapes `divz`
through the ordinary call boundary and is captured as the root call's `Result`.
Only an operator raising *inside the trapped expression itself* is missed.

## Why this is a bug and not the documented behaviour

`mfb spec language error-model` §8.4 scopes an inline `TRAP` to the whole
expression, and bug-457's fix made that literal for calls: "an inline `TRAP` …
covers **every** fallible call inside that expression". An error raised while
evaluating that expression must reach the handler regardless of which node
raised it. Nothing in §8.4 or §8.8 distinguishes a call from an operator here.

## Why bug-457's fix does not reach it

Two independent reasons, both structural:

* **The capture is per-value, not per-region.** Codegen's redirect is
  `self.raw_result_capture`, set around the lowering of *one* value
  (`lower_inline_conversion_raw` / the plan-21-B member generalization in
  `builder_values.rs`), and consumed by `emit_error_register_return`
  (`error/emission/builder_error_emission.rs`). It converts that value's
  domain-error exit into a branch to a capture label. There is no notion of "every
  op in this trap region redirects here".
* **bug-457's desugar spreads the expression across statements.** The fix lifts
  each nested fallible call into its own `Bind $trap_argN` / `Bind $trap_resN` op
  ahead of the residual expression (`ir::lower::lower_inline_trap`). An operator
  that raises can therefore sit in a plain `Bind` op that is not under any
  capture at all. Extending the existing per-value capture would miss exactly
  those.

So the fix is not "widen `raw_result_capture` a bit". It needs a trap-**region**
concept in codegen — every error exit emitted while lowering the ops of one
inline-`TRAP` region routes to that region's capture — which the structured,
jump-free IR (`mfb spec architecture ir`) deliberately does not have today.

## What a fix must produce

Every error raised while evaluating the trapped expression — from a call, an
operator, or a conversion — runs the handler, and a `RECOVER` skips the
remainder of the expression, matching bug-457's guarantee for calls.

Sketch of the two plausible shapes, neither costed:

1. **A region-scoped capture.** Give the lowered inline-`TRAP` region a codegen
   label and have `emit_error_register_return` prefer it over
   `error_exit_destination()` for every op inside the region, joining bug-457's
   shared `$trap_failed`/`$trap_err` flag on the way. Needs the region boundary
   to survive from `ir::lower` into codegen, which is new IR surface.
2. **Lift raising operators the way calls are lifted.** Give arithmetic a
   `Result`-producing IR form so `ir::lower` can hoist `a / b` into a checked
   `Bind` exactly as it hoists a fallible call. Conceptually the cleanest and
   reuses bug-457's whole desugar, but there is no `BinaryResult` node today and
   every raising operator would need a raw lowering.

An acceptable interim, as with bug-457, is a **diagnostic**: reject an inline
`TRAP` whose expression contains a raising operator that is not itself the
trapped root, telling the author to bind the arithmetic first. That converts a
silent miscompile into a compile error. It must not ship as the final answer.

## Blast radius

Any `LET x = f(… a / b …) TRAP … END TRAP` where the arithmetic can raise.
Division by zero is the one confirmed instance (`7-705-0002`, measured above);
any other operator that raises through the same `emit_error_register_return`
seam is in scope by construction, but none was separately measured. Narrower
than bug-457's
(nested calls are far more common than nested raising arithmetic), and no
instance was found in `tests/`, `examples/` or `src/` by the bug-457 census, so
this is a latent shape rather than one currently breaking a fixture.

## Ordering against bug-467

bug-467 (a socket write to a closed peer kills the process with `SIGPIPE`, exit
141) is the same family — a handler the author wrote correctly never runs — but
its signal fires *inside the syscall*, before any IR-level machinery can see an
error at all. So a trap-region design landed here first would appear to fail on
socket writes when it had not: no amount of region tracking helps until the
signal is suppressed. Fix 467 before attempting either shape above, and use its
repro as an adversarial input for whichever is chosen.

References: `src/ir/lower.rs:lower_inline_trap` (bug-457's desugar);
`src/codegen/engine/value/builder_values.rs` (`raw_result_capture` setup);
`src/codegen/error/emission/builder_error_emission.rs`
(`emit_error_register_return`, where the capture is consumed);
`src/docs/spec/language/08_error-model.md` §8.4, §8.8;
`bugs/completed/bug-457-inline-trap-misses-nested-fallible-call.md`.
