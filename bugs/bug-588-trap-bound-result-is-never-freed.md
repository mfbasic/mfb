# bug-588: a `Result OF T` bound through `TRAP` is never freed

Last updated: 2026-09-12
Effort: unknown (the desugared binding needs a scope-drop free; audit the shapes)
Severity: HIGH — unbounded, type-independent leak on every fallible call in an
expression
Class: Memory / correctness

Status: Open
Regression Test: an RSS pin in `tests/runtime/rt_scope_drop_leaks.rs`.

Split out of **bug-536**, found while fixing shape B-2 and recorded rather than
filed so the numbering would not race a peer session. It is NOT shape B-2 and is
unaffected by it — it reproduces unchanged on that fix's base commit.

## Reproduction

```basic
LET n AS Integer = fallibleFn(i) TRAP … END TRAP
```

in a loop. Measured per call:

| bound type | leak per call | 200k | 400k |
|---|---:|---:|---:|
| `Integer` | 128 B | 25 MB | 50 MB |
| `String` | 64 B | | |
| `List OF Integer` | 256 B | | |

**The leak is type-independent** — it is the `Result` binding itself, not the
payload. That is the distinguishing evidence: a payload-lifetime bug would not
leak 128 B for an `Integer`.

Calibrate at **>=200k iterations**, measure RSS with `--test-threads=1`.

## Root Cause (suspected — VERIFY BEFORE FIXING)

The `TRAP` desugar binds

    $trap_resN AS Result OF T = callResult …

and that synthesized binding gets **no scope-drop free**. Every fallible call in
an expression position goes through this desugar — which is every
`__csv_fieldValue` call in `csv::parse`.

Inherited from bug-536's measurements and **not** independently confirmed.
Reproduce and localize before changing anything.

## Why it matters beyond the leak

A missed desugar shape MISCOMPILES rather than merely leaking, and the elision
analyses are shape-coupled to lowering. Whatever fix lands must enumerate the
`TRAP` desugar's shapes exhaustively rather than patching the one in the repro —
the inline `TRAP` form and the statement form are different shapes, and a
function-level `TRAP` test cannot see the inline one.

Together with bug-587 this is the whole of `csv::parse`'s residual ~112 MB per
repeat call.

## Goal

- A `TRAP`-bound result is freed at the end of the binding's scope, for every
  bound type and every `TRAP` shape.

### Non-goals (must NOT change)

- Do not free the payload when it has escaped into the bound value — the
  `RECOVER` path and the success path bind different things.
- Do not change the observable `TRAP`/`RECOVER` semantics.

## Memory gate (required)

1. the RED RSS pin flips flat, for `Integer`, `String` AND a collection (the
   type-independence is the signature — pin all three);
2. name the documented contract in `mfb spec language error-model` §8 /
   `mfb spec` §14 the fix realizes, and show it only ADDS a free;
3. artifact-gate delta confined to emitting fixtures, everything else
   byte-identical; zero `.run` goldens moving;
4. a POSITIVE pin that a correct `TRAP`/`RECOVER` program is unchanged — both
   the success path and the raising path.
