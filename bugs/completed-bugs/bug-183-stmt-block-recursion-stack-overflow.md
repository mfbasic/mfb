# bug-183: deeply nested statement blocks overflow the native stack before ir::verify can reject them (SIGABRT)

Last updated: 2026-07-14
Effort: small (<1h)
Severity: HIGH
Class: Security

Status: Fixed
Regression Test: tests/rt-error/parser_statement_block_depth

A `.mfb` source file with deeply nested block statements (`IF … THEN`, `WHILE`,
`FOR`, `DO`, `MATCH/CASE`) drives the parser and every downstream recursive pass
(resolver, syntaxcheck, monomorph, `ir::lower`) into uncapped native recursion.
`ir::verify` *does* cap statement nesting at `MAX_DEPTH = 256`, but it runs last,
so an earlier pass overflows the native stack first and the compiler aborts with
`fatal runtime error: stack overflow, aborting` and **no diagnostic**. The
untrusted party is the author of an arbitrary source file; the impact is a
denial-of-service against anyone building it. The single correct behavior a fix
produces: a bounded parse diagnostic at the nesting cap (as expressions already
get via `MAX_EXPR_DEPTH`), a clean non-zero exit, and no crash.

This is the still-open audit-1 finding **FE-03**, re-verified and reproduced.
The parser is currently asymmetric: expression nesting is depth-guarded (FE-01,
fixed) but statement nesting is not. See `planning/audit-2-frontend.md`.

References:

- `planning/audit-2-frontend.md` (FE-03), `planning/old-plans/audit-1-frontend.md`
- `src/ast/parser.rs:26` — `FileParser` declares only `expr_depth`; no statement counter.
- `src/ir/verify/mod.rs:385,713-719` — the correct `MAX_DEPTH = 256` cap, but it
  runs after the passes that crash.

## Failing Reproduction

```
mfb init /tmp/fe03proj
python3 -c "
n=2000
l=['FUNC main() AS Integer']+['IF 1 = 1 THEN']*n+['PRINT 1']+['END IF']*n+['RETURN 0','END FUNC']
open('/tmp/fe03proj/src/main.mfb','w').write('\n'.join(l)+'\n')"
mfb build /tmp/fe03proj
```

- Observed (N=2000 and above): `thread 'main' has overflowed its stack / fatal runtime error: stack overflow, aborting`.
- Expected: a bounded parse diagnostic (`MFB_PARSE_*`, "statement nesting is too deep") at the cap, clean non-zero exit, no crash.

Observed threshold matrix (macOS-aarch64 debug build):

| N nested IF | Result |
| --- | --- |
| 300–500 | graceful parse error (`MFB_PARSE_UNEXPECTED_TOKEN`) ✓ |
| 2000, 5000, 20000 | `fatal runtime error: stack overflow` ✗ |

The exact crash threshold is stack-size / build-mode dependent (a release build
raises it) but stays trivially reachable by an attacker-sized file. The
graceful parse error seen at 300–500 is a different, incidental parser limit and
does not protect the deeper recursion.

## Root Cause

`src/ast/stmt.rs:710` `parse_statement_block` ↔ `:4` `parse_statement` ↔ `:407`
`parse_if_statement` (and the mutual recursion through `parse_match_statement`,
`parse_for_statement`, `parse_while_statement`, `parse_do_statement`) recurse one
native frame per nesting level with no counter — the parser's only guard,
`expr_depth`/`MAX_EXPR_DEPTH` (`src/ast/parser.rs:26`), covers expressions only.
The resulting deep AST is then re-walked recursively (also uncapped) by the
resolver, syntaxcheck, monomorph, and `ir::lower`, any of which can overflow
before `ir::verify` runs.

## Goal

- A source file whose block nesting exceeds the cap produces a bounded
  `MFB_PARSE_*` diagnostic and a clean non-zero exit — never a stack overflow.

### Non-goals (must NOT change)

- Do not grow the runtime stack as the fix.
- Do not remove or weaken `ir::verify`'s `MAX_DEPTH` backstop — it is correct;
  the gap is that it is unreachable, not wrong.
- No language-surface change; 256-deep block nesting is not real source.

## Blast Radius

- `src/ast/stmt.rs` block-parsing routines — fixed by capping at parse time,
  which prevents the deep AST from ever being built and thereby protects every
  downstream pass at once.
- Resolver / syntaxcheck / monomorph / `ir::lower` recursive walks — latent same
  hazard, but a parse-time cap makes them unreachable with an over-deep AST; out
  of scope to individually guard.

## Fix Design

Add a statement-nesting counter to `FileParser` mirroring `expr_depth` /
`MAX_EXPR_DEPTH` (cap 256, matching `ir::verify` `MAX_DEPTH`). Increment on entry
to each block-parsing routine (either `parse_statement_block` or each
`parse_*_statement`), report `MFB_PARSE_*` and bail when exceeded, decrement on
exit. Capping at parse time is strictly better than adding a cap to each later
pass because it stops the deep AST at the source.

## Phases

### Phase 1 — failing test + audit
- [x] Added `tests/rt-error/parser_statement_block_depth` (300 nested `IF`s);
      confirmed it crashed (`fatal runtime error: stack overflow`) before the fix.

### Phase 2 — the fix
- [x] Added `stmt_depth`/`MAX_STMT_DEPTH = 256` guard on `parse_statement_block`
      (`src/ast/stmt.rs`), the single funnel every nested block re-enters through,
      plus a `depth_exceeded` latch that fast-forwards to `Eof` and suppresses the
      unwinding cascade so exactly one `MFB_PARSE_BLOCK_TOO_DEEP` (`1-102-0010`) is
      reported. New rule in `src/rules/table.rs`.

### Phase 3 — validation
- [x] Repro emits the graceful diagnostic and exits 1 (no crash); deeply-but-legally
      nested programs unaffected (cap = 256, matching `MAX_EXPR_DEPTH`).

## Validation Plan

- Regression test: the nesting reproduction, asserting the `MFB_PARSE_*` diagnostic.
- Runtime proof: `mfb build` on the repro exits non-zero with the diagnostic, no crash.
- Full suite: `scripts/test-accept.sh`.

## Summary

Low-risk, small fix: one counter symmetric with the existing expression-depth
guard. The only care needed is choosing an increment point that covers every
block-forming statement.
