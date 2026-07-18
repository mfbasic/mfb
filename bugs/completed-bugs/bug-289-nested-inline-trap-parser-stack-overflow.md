# bug-289: nested inline-`TRAP` handlers recurse unbounded → compiler stack-overflow abort (no diagnostic)

Last updated: 2026-07-18
Effort: small (<1h)
Severity: HIGH
Class: Correctness (DoS on pathological/malformed source)

Status: Fixed 2026-07-18
Regression Test: tests/ (new) — deeply nested inline-TRAP source yields a `MFB_PARSE_BLOCK_TOO_DEEP` diagnostic, not a crash

The postfix-TRAP handler loop calls `parse_statement` directly without the
`enter_stmt`/`leave_stmt` depth accounting. A handler statement that itself carries
a postfix TRAP re-enters `parse_statement → parse_simple_statement →
maybe_attach_postfix_trap → parse_statement …` — the one statement-nesting funnel
that bug-183's `MAX_STMT_DEPTH` guard does not count (it guards only
`parse_statement_block`). Deeply nested (even properly-closed) inline TRAPs overflow
the native stack and abort the compiler with no diagnostic, and downstream AST
walkers inherit the same unbounded `Expression::Trapped → handler → Trapped` depth.

The single correct behavior a fix produces: pathologically nested inline-TRAP source
is rejected with a single `MFB_PARSE_BLOCK_TOO_DEEP` (or equivalent) diagnostic, the
same as every other over-deep construct — never a stack-overflow abort.

References:

- `bugs/completed-bugs/bug-183-*` and audit-2 FE-01/FE-03 (recursion caps on
  expr/stmt-block/type/TGROUP funnels — this funnel was missed).
- Found during goal-06 review of `src/ast/stmt.rs`.

## Failing Reproduction

```
# 300 properly-closed nested inline TRAPs (or ~100k unclosed `x = f() TRAP` lines)
x = f() TRAP x = f() TRAP x = f() TRAP … END TRAP END TRAP END TRAP
```

- Observed: `fatal runtime error: stack overflow, aborting` in the parser; even 300
  properly-closed levels abort with zero diagnostics. ≤256 levels produce a normal
  diagnostic.
- Expected: a single `MFB_PARSE_BLOCK_TOO_DEEP` diagnostic at the cap.

## Root Cause

`src/ast/stmt.rs:395-403` (`maybe_attach_postfix_trap`, handler loop) calls
`parse_statement` without `enter_stmt()`/`leave_stmt()`, so the recursion through
the postfix-trap handler is not counted by the `MAX_STMT_DEPTH` latch (which only
wraps `parse_statement_block`). The function-level `parse_trap` body loop
(`src/ast/items.rs:190-197`) has the same gap.

## Goal

- Wrap the postfix-trap handler-statement loop (and the function-level trap body
  loop) in `enter_stmt()`/`leave_stmt()`, so the existing 256 cap + `depth_exceeded`
  latch yields one diagnostic and protects every downstream pass.

### Non-goals (must NOT change)

- The depth cap value or the diagnostic code.
- Well-formed programs within the cap (must still parse).

## Blast Radius

- `stmt.rs:maybe_attach_postfix_trap` handler loop — fixed here.
- `items.rs:190-197` function-level trap body loop — same fix (consistency).
- Downstream AST walkers over `Expression::Trapped` inherit the protection once the
  parser caps depth.

## Fix Design

Add `enter_stmt()` at the top of the handler-statement loop iteration and
`leave_stmt()` after, exactly as `parse_statement_block` does. Rejected alternative:
a separate trap-specific counter — unnecessary; the shared stmt-depth latch already
exists and gives a uniform diagnostic.

## Phases

### Phase 1 — failing test
- [ ] Test 300-deep nested inline TRAP aborts today; add a `#[should_panic]`-free
      assertion once fixed that it yields the diagnostic.
### Phase 2 — the fix
- [ ] Wrap both trap loops in enter/leave.
### Phase 3 — validation
- [ ] Full suite green; a within-cap nested-trap program still parses.

## Validation Plan

- Regression: deep nested-trap source → `MFB_PARSE_BLOCK_TOO_DEEP`, exit non-crash.
- Runtime proof: no stack-overflow abort.
- Doc sync: none.

## Summary

The last uncounted recursion funnel in the parser; wrapping the trap handler loops
in the existing depth latch converts a crash into a clean diagnostic. Small,
mirrors bug-183.

## Resolution

`maybe_attach_postfix_trap`'s handler loop calls `parse_statement`, which can
re-enter `maybe_attach_postfix_trap` — a recursion funnel `MAX_STMT_DEPTH` never
counted, because the latch only wraps `parse_statement_block`. The handler loop
now sits inside `enter_stmt()` / `leave_stmt()` (`src/ast/stmt.rs:395-416`), so
deep nesting reports `MFB_PARSE_BLOCK_TOO_DEEP` instead of aborting.

Tests: `ast::tests::deeply_nested_inline_traps_hit_the_statement_depth_cap` (run
on a 64 MiB thread, mirroring `ir::coverage_tests::decode_rejects_depth_limit`)
plus the golden fixture `tests/rt-error/parser_inline_trap_depth/`. Proven both
ways: with the fix reverted the test aborts with a stack overflow in the spawned
thread; with it, a single diagnostic.

### Two claims in this report were wrong

1. **The threshold was overstated by roughly 10x.** The report says "even 300
   properly-closed levels abort with zero diagnostics" and "≤256 levels produce a
   normal diagnostic". Measured on a release build, 300 *and* 1000 levels parse
   cleanly through to typecheck (`TYPE_INLINE_TRAP_FALLS_THROUGH`), and ≤256
   produces no parse diagnostic at all. The real overflow threshold is between
   1000 and 5000 levels. The bug is real; the numbers were not.
2. **The second named fix site is not reachable.** The report claims
   `src/ast/items.rs:190-197` (the function-level `parse_trap` body loop) "has the
   same gap". `parse_trap` is called only from `parse_function`, which is called
   only from the top-level item loop; `parse_statement` has no path back to it, so
   it is not a recursion funnel. Left unchanged deliberately — adding a latch
   there would shave a level off the nesting budget inside function-level TRAP
   bodies for no safety gain.
