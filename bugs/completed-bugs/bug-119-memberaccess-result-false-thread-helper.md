# bug-119 — MemberAccess member named `result` unconditionally declares the Thread helper → valid program rejected

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, runtime-specs slice).
**Severity:** MED — a valid program with a record/enum field named `result`
fails to compile.
**Class:** correctness (collector asymmetry / false positive).

## Finding

`src/target/shared/runtime/usage.rs:233-237` (`push_value_helpers`,
`IrValue::MemberAccess` arm):

```rust
if member == "result" { push_unique(helpers, RuntimeHelper::Thread) }
```

This fires for ANY member access named `result`, not just Thread handles.
`.result` on Thread types was removed from the language (ir/verify/mod.rs:1725,
`TYPE_THREAD_RESULT_REMOVED`; syntaxcheck/inference.rs:764), so the only
`.result` accesses surviving to IR are user record/enum members — pure false
positives. validate.rs has no matching `"result"` heuristic (its MemberAccess
arms at 433/1481 push nothing), so the declared-but-unused check
(validate.rs:103-108) rejects the program.

## Trigger

`TYPE Score` record with field `result AS Integer`, access `s.result` anywhere,
no thread usage → compile fails with `NIR declares unused runtime helper
'thread'`. (`result` is not a reserved identifier — no lexer/resolver
restriction.)

## Fix

Delete the `member == "result"` heuristic (it's dead now that `.result` is
removed from the language), or type-gate it to Thread-typed receivers only,
matching the type-gated sibling heuristics (plan/symbols.rs:593,
builder_values.rs:1283).

## Third site (plan/symbols.rs) — same root cause, independently reproduced

A parallel review of the plan/nir layer reproduced this **end-to-end** with
`target/debug/mfb`: `TYPE Calc { result AS Integer }` + `LET c = Calc[5]` +
`io::print(toString(c.result))` → build fails with `error: NIR declares unused
runtime helper 'thread'`. The stale `.result`-means-Thread heuristic exists in
**three** type-blind copies (all predating the removal of `Thread.result` from
the language, per plan-result-cleanup):

1. `src/target/shared/runtime/usage.rs:233-236` (`push_value_helpers`) — the one
   this doc's title names, declares `RuntimeHelper::Thread`.
2. `src/target/shared/plan/symbols.rs:592-599`
   (`collect_runtime_symbols_from_value`) — additionally injects
   `_mfb_rt_thread_thread_waitFor` into `runtimeSymbols` for any `.result`
   access (the next failure point if the validate mismatch were fixed on only
   one side).
3. `src/target/shared/validate.rs:433-435` treats MemberAccess as plain
   recursion and never counts it, so the used-helper scan and the declared set
   disagree → hard reject at validate.rs:104-110.

The fix must remove/type-gate **all three** copies together.

## Prior art

Same asymmetry class as bug-45, different site.
