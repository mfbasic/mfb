# bug-174 — middle-end nits (ir/verify permissive/unbounded, resolver alias dup, monomorph arg-drop, ir/lower For column)

Last updated: 2026-07-12
Severity: LOW (batch).
Class: Correctness / Footgun.
Status: Open

## Findings

**A. `Money + Unknown` wrongly rejected as an operator mismatch.**
`src/ir/verify/mod.rs:1835` (guard) and `:1839` (`numeric` closure). The Money
branch is entered only when `(lt=="Money"||rt=="Money") && lt!="Unknown" &&
rt!="Unknown"`. When one side is `Unknown`, control falls to the generic path
whose `numeric` closure excludes `Money`, so `Money + Unknown` →
`TYPE_BINARY_OPERATOR_MISMATCH` (a `RELOCATED_TO_IR_VERIFY` code, surfaced on the
source path). Contradicts the module's "Unknown stays permissive" contract
(comment at :1834). Trigger: `LET u = someUntypableCall()` then `m + u` with
`m AS Money`. Fix: treat Money-with-Unknown as permissive (add `Money` to the
generic `numeric` closure, or return without emitting when a companion is
Unknown).

**B. Value-expression recursion has no depth cap.**
`src/ir/verify/mod.rs:1372` (`check_value`; `MAX_DEPTH`/`check_ops` at :385
bounds *statement* nesting only). `check_value`/`infer_type`/`walk_captures`/
`collect_closures`/`collect_local_reads_value` recurse on expression depth with no
cap; a `.mfp`/synthesized IR with a value expression nested tens-of-thousands deep
overflows the stack. Latent (package path bounded by the decoder's
`MAX_DECODE_DEPTH`, source path by the parser). Fix: thread a depth counter
through the value walkers mirroring `check_ops`'s `MAX_DEPTH`.

**C. Re-export alias functions bypass duplicate detection.**
`src/resolver/mod.rs:327-360`. `insert_alias_function` checks only
`self.top_levels` before pushing into `self.functions`; unlike `insert_function`
(:402-422) it never scans `self.functions` for a name/param collision, so two
`EXPORT FUNC a AS link::x` aliases with the same name silently coexist as an
unintended overload set instead of `SYMBOL_DUPLICATE_TOP_LEVEL` (detection is
asymmetric — a later `FUNC` is still caught). Fix: in `insert_alias_function`,
also reject when `self.functions.get(name)` already has an entry with equal
params.

**D. Uninferable argument types are dropped, misaligning overload resolution.**
`src/monomorph/lower.rs:1016-1019`. `arg_types` is built with
`filter_map(... expression_type ...)`, so an argument whose type is `None` is
*removed* rather than kept as `Unknown`, shortening the vector and shifting
remaining types into wrong parameter positions in `params_match`/`resolve_overload`
(a 2-arg call can match a 1-arg overload). Later stages re-check arity, so it
degrades to a confusing diagnostic rather than a miscompile. Fix: map an
uninferable argument to `"Unknown"` (a wildcard the matchers handle) instead of
dropping it.

**E. `IrOp::For` stamped with column 0 while all other ops use column 1.**
`src/ir/lower.rs:1226`. Harmless today (diagnostics report line only) but
inconsistent with `LowerContext.current_loc`/`statement_loc`. Fix: use
`column: 1`.
