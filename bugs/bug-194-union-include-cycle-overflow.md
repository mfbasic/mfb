# bug-194: UNION include cycle overflows the stack during IR lowering (no cycle guard)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: memory-safety (compile-time DoS)

Status: Open
Regression Test: tests/syntax/ (mutually-including unions → include-cycle diagnostic, not abort)

`expanded_union_variants` in IR lowering recurses through `UNION ... INCLUDES ...`
edges with no cycle guard, so a mutually- or self-including union recurses
unboundedly and overflows the native stack, aborting the compiler with no
diagnostic. The syntaxcheck copy of the same routine
(`src/syntaxcheck/mod.rs:1125`) has a `visiting: HashSet` cycle guard and
silently returns empty, so the cycle passes all front-end checks and only crashes
at lowering.

## Failing Reproduction

```
UNION A INCLUDES B ... END UNION
UNION B INCLUDES A ... END UNION
```
Confirmed: `mfb build` prints `Building ...` then `thread 'main' has overflowed
its stack / fatal runtime error: stack overflow, aborting`. Expected: a clean
include-cycle diagnostic.

## Root Cause

`src/ir/lower.rs:3838` `expanded_union_variants` (reached from `TypeIndex::new`
at 3765) lacks the `visiting`/seen set that the syntaxcheck twin has.

## Non-goals

- Do not change expansion of acyclic (valid) unions.

## Blast Radius

- `expanded_union_variants` in lower.rs. Ideally also add a real include-cycle
  diagnostic in syntaxcheck so the error is reported before lowering.

## Fix Design

Thread a `visiting: HashSet<&str>` through `expanded_union_variants` in lower.rs
(mirroring the syntaxcheck version) so a revisited union short-circuits; emit an
include-cycle diagnostic in syntaxcheck.
