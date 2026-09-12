# bug-588: a `Result OF T` bound through `TRAP` is never freed

> **WITHDRAWN.** This is the same defect as **bug-561** (`bugs/completed/bug-561-trap-result-binding-never-freed.md`),
> which was filed on 2026-09-06 and FIXED in `26e47b003` (2026-09-07), with the param-borrow half in `1bf2a4a94` (bug-568).
> It should never have been filed. See "Why this was filed twice" at the end.


Last updated: 2026-09-12
Effort: unknown (the desugared binding needs a scope-drop free; audit the shapes)
Severity: HIGH — unbounded, type-independent leak on every fallible call in an  
*(severity as filed; moot — the defect is fixed)*
expression
Class: Memory / correctness

Status: **CLOSED — DUPLICATE of bug-561, which was fixed before this was filed.**
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

## Why this was filed twice — and how to not do it again

bug-536 recorded three defects found while fixing its shape B-2, with the note
"they are recorded here rather than filed so the numbering does not race with a
peer session". **They were filed the next day**, as bugs 560, 561 and 562
(`8c57683f3`, 2026-09-06) — but nobody went back and updated bug-536's item
list. Six days later a session read that unchanged list, took the note at face
value, and filed all three a second time as 587/588/589.

The number checks that were run (`ls bugs/ bugs/completed/` and
`git log --all --grep=bug-NNN`) all passed, because they answer "is this NUMBER
free?" — which it was. Nobody asked "is this DEFECT already tracked?", and that
is the question that mattered.

**The rule: before filing, search for the DEFECT, not the number.** Grep
`bugs/completed/` for the symptom, the function name, and the idiom — here,
`git grep -il "self.append" bugs/` would have surfaced bug-561 immediately.
A "recorded but not filed" note in an older document is a claim about the past
with a timestamp on it; check whether it is still true before acting on it.

Verified before closing (not assumed): bug-561 is marked FIXED with a named
commit, and its regression pins are live at HEAD —
the inline-`TRAP` cases in `tests/runtime/rt_scope_drop_leaks.rs`
(see the `bug-561:` block, and the `failable(i) TRAP` case at its line 840).
