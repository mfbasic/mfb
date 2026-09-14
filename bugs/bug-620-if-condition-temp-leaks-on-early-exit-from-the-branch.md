# bug-620: a call result in an IF condition leaks when the branch leaves by RETURN, EXIT or CONTINUE

Last updated: 2026-09-13
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness (memory)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (to add, Phase 1); tests/runtime/rt_debug_soak.rs (`a_dom_parse_loop_keeps_live_bytes_constant`, plan-133-A)

A heap value produced while evaluating an `IF` (or `ELSEIF`) condition — a `String` from
`strings::lower`, a record copy from `collections::get(...)`, any call result — is never freed
when the branch it selects leaves the statement by `RETURN`, `EXIT DO`/`EXIT FOR`/`EXIT WHILE`
or `CONTINUE`. One block leaks per such exit. When the condition is false, or the branch falls
through to `END IF`, the temp is freed. In the browser example this is most of the page load's
retained memory (plan-133-A): `selectorMatches`/`compoundMatches` take this exit once per
element × CSS rule.

**The single correct behavior a fix produces:** every temp allocated by a condition is freed
exactly once on every path out of the `IF` — fall-through, `RETURN`, `EXIT`, `CONTINUE` — so
the reproduction below reports the same `live_bytes` at N=1000 and N=2000.

References:

- `src/docs/spec/memory/04_arenas.md`, the scope-drop contract (a value no binding owns is
  freed at the end of its statement).
- Found by plan-133-A Phase 2 (`planning/plan-133-A-browser-memory-diagnosis-and-soak-test.md`).
- Sibling: bug-621 (the same statement-scope drop, missed on each pass of a loop condition).
- Earlier fixes of the same family: bug-567 (a RETURN expression's own temps), bug-571
  (the FOR EACH item on an early exit).

## Failing Reproduction

`/tmp/plan-133-a/stages/r1_nomut.mfb` (the plan-133-A harness), with `{N}` = 1000 and 2000,
built with `target/release/mfb build --debug` on macOS (main `14c9fc1ca`):

```
IMPORT io
IMPORT strings

FUNC f(s AS String) AS Boolean
  IF strings::lower(s) <> "zz" THEN RETURN FALSE
  RETURN TRUE
END FUNC

SUB main()
  MUT hits AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    IF f("p") THEN hits = hits + 1
    i = i + 1
  END WHILE
  io::print("hits=" & toString(hits))
END SUB
```

- Observed (`arena.0.*` of the `--debug` report): N=1000 `live_bytes 29520`,
  `alloc_calls 1004`, `free_calls 2`; N=2000 `live_bytes 45520`, `alloc_calls 2004`,
  `free_calls 2`. One 16 B block per call is never freed.
- Expected: `live_bytes` equal at both N (13,520, the program's baseline).

Contrast cases (same harness, N=1000 vs 2000):

| Shape | `live_bytes` 1000 → 2000 | Verdict |
|---|---|---|
| the repro above, one-line `IF … THEN RETURN` | 29,520 → 45,520 | leaks ✗ |
| same, block `IF … THEN` / `RETURN FALSE` / `END IF` (`r7_block`) | 29,520 → 45,520 | leaks ✗ |
| condition false (`f("ZZ")`, `r5_condfalse`) | 13,520 → 13,520 | flat ✓ |
| temp bound first: `LET l AS String = strings::lower(t)` then `IF l <> …` (`r1_bound`) | 13,520 → 13,520 | flat ✓ |
| Boolean-returning call in the condition, `IF NOT hasStr(classes, tok) THEN RETURN FALSE` (`r3_boolcall`) | 13,520 → 13,520 | flat ✓ (no heap temp) |
| `IF collections::get(stack, s).tag = name THEN k = s : EXIT DO` over a recursive record (`ct_close`) | 141,520 → 269,520 | leaks ✗ (128 B, 2 blocks per exit) |
| the same `closeTag` without the search loop (`ct_nosearch`) | 13,520 → 13,520 | flat ✓ |

In the browser (plan-133-A § 2, macOS): `dom::resolveStyles` of the saved `BASIC` page leaves
718,525,504 B per call (41,384,934 unfreed blocks), `dom::parse` 8,318,848 B per call. The
share this bug owns is in plan-133-A's per-stage table (measured with the sites rewritten).

## Root Cause

Condition temps are recorded as pending temps and freed only by the statement-scope drop that
runs after the whole `IF`:

- `register_pending_temp` (`src/codegen/engine/value/builder_values.rs`) pushes each fresh
  block onto `pending_temp_frees`; `lower_ops_inner`
  (`src/codegen/engine/control/builder_control.rs`) takes a per-statement
  `temp_watermark = self.pending_temp_frees.len()`.
- The `NirOp::If` arm lowers the condition (`self.lower_value(condition)`), then the branches
  (`lower_ops(then_body)`, `lower_ops(else_body)`), then `if_end`. It frees nothing itself.
  `NirOp::If` is `TransferTemps::StatementScope`, so the only free is
  `drop_pending_temps_to(temp_watermark)` emitted after `if_end`.
- `RETURN` inside the branch calls `emit_return_exit(value, Some(temp_watermark))`
  (`builder_exits.rs::emit_return_exit_inner`) with **its own** statement watermark, which sits
  above the enclosing `IF`'s condition temps; `drop_interior_temps_before_branch` pops only
  above that watermark. It then frees `active_cleanups` (owned locals) and returns.
- `EXIT`/`CONTINUE` call `emit_cleanup_branch_to_depth` (`builder_exits.rs`), which frees only
  `active_cleanups[cleanup_depth..]` and never reads `pending_temp_frees`.

So every exit that jumps past `if_end` skips the drop. A `LET`-bound value is immune because it
is an owned local in `active_cleanups`, which RETURN and EXIT do free. An `ELSEIF` is a nested
`If` in `else_body` and has the same hole.

## Goal

- The repro reports `live_bytes` equal at N=1000 and N=2000, and so do the `r7_block` and
  `ct_close` shapes above, with `EXIT FOR`, `EXIT WHILE` and `CONTINUE` variants added.
- No double free: the fall-through path still frees each condition temp exactly once
  (`double_free_skips 0` in the report).

### Non-goals (must NOT change)

- The condition's value and short-circuit semantics.
- The fall-through drop after `if_end`.
- **Tempting wrong fix:** rewriting the browser example to bind temps first (the diagnosis
  copy in plan-133-A did exactly that to measure). That hides the leak in one program; every
  other program keeps it. The compiler is the fix.

## Blast Radius

Compiler paths (search: the `NirOp::If` arm and every exit emitter in
`src/codegen/engine/control/builder_exits.rs`):

- `RETURN` inside an `IF`/`ELSEIF` branch whose condition holds a temp — fixed by this bug.
- `EXIT DO`/`EXIT FOR`/`EXIT WHILE`/`CONTINUE` inside such a branch — fixed by this bug.
- `MATCH`/`SELECT` scrutinee temps with an exit inside a `CASE` — same statement-scope
  structure; audit in Phase 1, fixed here if it leaks.
- A raised error inside the branch — unaffected: `emit_call_error_exit` already frees every
  pending temp in place (`emit_pending_temp_frees_in_place`).
- Loop conditions re-evaluated each pass — bug-621.

Consumers observed leaking (plan-133-A scan of `examples/browser`, heap temp in the condition
and an exit in the branch): `dom/src/parse.mfb` `findCloseTag`, `findCloseTagStart`
(`IF tagName(html, p + 2) = name THEN … RETURN`), `closeTag`
(`IF collections::get(stack, s).tag = name THEN … EXIT DO`); `dom/src/resolve.mfb`
`compoundMatches` (`IF strings::lower(tagPart) <> tag THEN RETURN FALSE`), `selectorMatches`
(`IF NOT compoundMatches(collections::get(parts, np - 1), …) THEN RETURN FALSE`);
`dom/src/layout.mfb` `wrapChildren`, `wrapGlyphs` (`IF strings::trim(x) = "" THEN RETURN …`).
All are fixed by the compiler fix; none needs a source change.

## Fix Design

Free the enclosing statements' pending temps on every exit that leaves them. Recommended:
record, per enclosing construct, the pending-temp watermark at its entry (the `LoopLabels`
already carry `cleanup_depth`; add `temp_depth`), and have `RETURN` drop to the function's
base watermark and `EXIT`/`CONTINUE` drop to the target loop's `temp_depth`. Use the in-place
form (`emit_pending_temp_frees_in_place`) so the fall-through path still owns and frees its
temps once. Every free zeroes its slot and the prologue zeroes every slot, so a path that skips
a temp's allocation frees null.

Rejected: freeing condition temps at the start of each branch — the condition value can
borrow from the temp (a comparison against a field of `collections::get(...)`), and a branch
body may read the temp's contents in future shapes.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] `rt_scope_drop_leaks.rs`: the repro, `r7_block`, `ct_close`, and `EXIT FOR` / `CONTINUE`
      / `ELSEIF` / `MATCH` variants, each N vs 2N on `live_bytes`; confirm each fails.
- [ ] Audit `MATCH`/`SELECT` scrutinee temps with an exit inside a `CASE`; record the verdict.

Acceptance: every new case fails for the documented reason; the audit list has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Temp watermark per enclosing construct; drop on `RETURN`/`EXIT`/`CONTINUE`
      (`builder_control.rs`, `builder_exits.rs`).

Acceptance: Phase 1 cases pass; the contrast cases stay flat; `double_free_skips 0`.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate the goldens the new frees shift (`scripts/sync-goldens.sh`); confirm each
      delta is a free on an exit edge.
- [ ] Full suite and `scripts/test-accept.sh`.
- [ ] Re-run plan-133-A's `resolve` and `parse` stages; record the new per-call leak.

Acceptance: full suite green; golden deltas are only exit-edge frees; the browser stages drop
by this bug's share.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 cases in `rt_scope_drop_leaks.rs`; plan-133-A's
  `rt_debug_soak.rs` dom case.
- Runtime proof: plan-133-A's stage programs before and after.
- Doc sync: none expected (the spec already requires the free).
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- Fix bug-620 and bug-621 together — recommended (one watermark model for condition temps) vs.
  separately.

## Summary

The risk is in the exit emitters: they must free exactly the temps of the constructs they
leave, no more (a double free on fall-through) and no fewer. The browser source is left as is.
