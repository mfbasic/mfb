# bug-625: every AttributedString value leaks one 48-byte block when it is dropped

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (to add, Phase 1); tests/runtime/rt_debug_soak.rs (`a_paint_loop_keeps_live_bytes_constant`, plan-133-A)

An `AttributedString` built by `astrings::fromString` and then dropped leaves one 48-byte
block live. It doesn't matter whether the value was bound to a `LET`, was an element of a
`List OF AttributedString`, or was a record field. A terminal UI that repaints rows as
attributed strings leaks one block per row per repaint. The browser example's
`display::paint` leaks 1,632 B per paint of the `BASIC` page this way (plan-133-A § 2).

**The single correct behavior a fix produces:** dropping an `AttributedString` frees every
block it owns, so the reproduction below reports equal `live_bytes` at N=1000 and N=2000.

References:

- `src/docs/spec/memory/04_arenas.md` (scope drop); the `astrings` man page.
- Found by plan-133-A Phase 2: the canvas remainder of the paint stage, isolated by bisecting
  scratch copies of `examples/browser/display` (`/tmp/plan-133-a/bisect_display*.py`).

## Failing Reproduction

`/tmp/plan-133-a/stages/as2_single.mfb`, `{N}` = 1000 and 2000, `target/release/mfb build
--debug`, macOS, main `14c9fc1ca`:

```
IMPORT io
IMPORT astrings

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET a AS AttributedString = astrings::fromString("ab" & toString(i))
    total = total + 1
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

- Observed (`arena.0.*`): N=1000 `live_bytes 75040`, `alloc_calls 4006`, `free_calls 3002`;
  N=2000 `live_bytes 123040`, `alloc_calls 8006`, `free_calls 6002`. Each iteration makes 4
  allocations and 3 frees, leaving one 48 B block.
- Expected: `live_bytes` equal at both N.

Shapes (N=1000 → 2000):

| Shape | `live_bytes` | Per iteration |
|---|---|---|
| `LET a AS AttributedString = astrings::fromString(…)` (`as2_single`) | 75,040 → 123,040 | 1 block, 48 B |
| `LET l AS List OF AttributedString = [fromString(…), fromString(…)]` (`as1_list`) | 123,040 → 219,040 | 2 blocks, 96 B |
| a record `{rows AS List OF AttributedString, count}` returned from a helper that appends 4 (`as3_record`) | 219,040 → 411,040 | 4 blocks, 192 B |
| `display::paint` of a 5-row page, dom's bug-620/621 sites rewritten (`pt_paint`) | 37,520 → 61,520 at N=100 → 200 | 240 B = 5 × 48 |
| the same with paint's final `FOR EACH rowText IN cv.rows` loop (which builds the attributed rows) removed (`display-v11`) | 13,520 → 13,520 | 0 |
| plain `String` values in the same loops (`fe_field`, `fe_field_let`) | 13,520 → 13,520 | 0 |

## Root Cause

Not localized yet. The hypothesis: `astrings::fromString` allocates a small internal block
(an attribute-run list, or a header) that the `AttributedString` drop does not walk. The
drop frees the value's other blocks and the concat temp (3 of the 4 allocations), but not
this one. To confirm: read the `AttributedString` layout that `astrings` emits, and the drop
path its binding and its list elements get. Find which of the 4 allocations is the 48 B block
and why its free is missing.

## Goal

- The three reproduction shapes report equal `live_bytes` at N and 2N.
- `astrings` values that carry attributes (`astrings::addAttribute`) are also freed completely.

### Non-goals (must NOT change)

- `AttributedString` contents, rendering, and the `astrings` API.
- **Tempting wrong fix:** building display rows as plain `String` in the browser example.
  That hides the leak in one program and leaves it in the type.

## Blast Radius

- Every `astrings` constructor and transformer (`fromString`, `addAttribute`, …): audit in
  Phase 1 for the same missing free.
- `List OF AttributedString` element drops and record-field drops: fixed by the same drop.
- `term` / `app` APIs taking `AttributedString` rows (the browser's screen): consumers, and
  unaffected by the fix.

## Fix Design

Once Phase 1 names the block, have the `AttributedString` drop free it, on every path that
drops the type: binding, temp, collection element, and record field.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] `rt_scope_drop_leaks.rs`: the three shapes above, plus a value with an attribute, N vs 2N;
      confirm each fails.
- [ ] Localize the leaked block and cite the drop path here; audit the other constructors.

Acceptance: the cases fail for the documented reason; Root Cause names the block and the path.
Commit: —

### Phase 2 — the fix

- [ ] Free the block in the `AttributedString` drop.

Acceptance: Phase 1 cases flat; `double_free_skips 0`; `astrings` suites green.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate shifted goldens; full suite; `scripts/test-accept.sh`; plan-133-A paint stage
      re-run.

Acceptance: full suite green; the paint stage's remainder drops to 0.
Commit: —

## Validation Plan

- Regression tests: Phase 1 cases; plan-133-A's soak paint case, together with bug-620/621.
- Runtime proof: `as2_single` flat.
- Doc sync: none expected.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- None.

## Summary

A single missing free in one builtin type's drop. The only care needed is covering every path
that drops the type.
