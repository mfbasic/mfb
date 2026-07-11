# bug-94 — `datetime::fixedOffset(hours := N)` silently binds N as seconds

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G4).
**Severity:** MED — silent wrong value; compiles clean, zone is off by ~3600×.
**Class:** correctness (named-argument binding across structurally different overloads).

## Finding

`src/builtins/datetime.rs:144` — `call_param_names` for `FIXED_OFFSET` merges
both overloads' names into one positional alias table:

```rust
&[&["hours", "offsetSeconds"], &["mins"]]
```

But the two overloads give position 0 different meanings — 1-arg
`(offsetSeconds)` vs 2-arg `(hours, mins)` (see `datetime_package.mfb:385/392`
and the man page). The merged table lets the name `hours` bind position 0 of
the **1-argument** call, which the arity-keyed `implementation_name` routes to
`__datetime_fixedOffset1(offsetSeconds)`. The reverse also holds:
`fixedOffset(offsetSeconds := X, mins := Y)` binds X as hours.

The mod.rs guard test only rejects an alias repeated at *two positions*; this
alias sits at one position, so nothing fires.

## Trigger

```
LET z = datetime::fixedOffset(hours := 5)
```

Compiles cleanly; returns a zone of +5 **seconds** (label "+00:00"), not
+05:00. `datetime::inZone(now, z)` then yields civil fields ~5 hours off, with
no diagnostic.

## Fix sketch

Same treatment `net.connectTcp` got for this exact class (bug-28): declare a
per-overload table via `call_param_name_overloads` for datetime `fixedOffset`
so `hours`/`mins` only bind the 2-arg overload and `offsetSeconds` only the
1-arg one. Audit the other builtins' merged alias tables for the same
structural-disagreement shape while there.

## Prior art

Same root-cause class as bug-28 (fixed for net.connectTcp only). This instance
was never covered.
