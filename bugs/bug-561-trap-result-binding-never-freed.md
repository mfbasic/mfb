# bug-561: a `Result OF T` bound through `TRAP` is never freed, for every `T`

Last updated: 2026-09-06
Effort: medium
Severity: **HIGH** (unbounded leak on every fallible call in an expression)
Class: Memory / correctness

Status: Open
Regression Test: — (a `tests/rt_scope_drop_leaks.rs` RSS case, to be added)

Found while fixing bug-536 shape B-2. **Reproduces unchanged on the base commit
and is unaffected by that fix.**

## The finding

`LET n AS Integer = fallibleFn(i) TRAP … END TRAP` in a loop leaks, and the leak
is **type-independent** — it is the `Result` wrapper, not the payload:

| bound type | leak per call | measured |
| --- | --- | --- |
| `Integer` | **128 B** | 25 MB at 200k, 50 MB at 400k |
| `String` | 64 B | |
| `List OF Integer` | 256 B | |

## Root cause (stated, not yet confirmed by a fix)

The `TRAP` desugar binds `$trap_resN AS Result OF T = callResult …`, and **that
binding gets no scope-drop free**. Every fallible call appearing in an expression
goes through the desugar, so the reach is every `TRAP` in the language.

## Why it matters

Together with bug-560 this is the **entirety** of `csv::parse`'s residual
112 MB per repeat call after bug-536 shape B-2. Anyone tracking that number
should attribute it here, not to B-2.

Every `__csv_fieldValue` call in `csv::parse` goes through this path.

## What a fix must produce

A `TRAP`-bound value in a loop runs at constant RSS for every payload type,
including a payload type that is itself not freeable — in which case the wrapper
must still be freed.

**The obvious wrong fix is a double free.** The success path binds the payload
out of the `Result` and the caller then owns it; freeing the wrapper must not
free what was moved out of it. Follow bug-536 shape B's precedent: fail-closed
provenance, so an unproven case keeps leaking rather than wild-freeing.

Measure as peak RSS at N and 2N, not one-shot.
