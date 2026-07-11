# bug-15: Two ordered-only x86 float compares to the same target emit a duplicate `{target}__x86ford` skip label → NaN-only silent wrong control flow

Last updated: 2026-07-08
Effort: medium (1h–2h)

On x86-64, an ordered-only float branch (`<`, `<=`, `=` — AArch64 `b.mi`/`b.ls`/
`b.eq`, plus `b.lo`) is lowered by `x86_float_branch::ordered_only`
(`src/arch/x86_64/select.rs:568-575`) to `jp skip; <cc> target; skip:`, where the
skip label name is derived **solely from the target**:
`let skip = format!("{target}__x86ford")`. When **two** ordered-only float
branches target the **same** label — e.g. a short-circuit `IF a < b OR c < d THEN
GOTO L` where both `a<b` and `c<d` lower to ordered-only branches to `L` — both
emit a label literally named `L__x86ford`, at two different byte offsets.

The encoder stores labels in a name-keyed map and inserts with `.insert(name,
offset)` (`src/arch/x86_64/encode/mod.rs:100-102`) — **last writer wins** — so
both `jp L__x86ford` instructions resolve to the **second** label's offset.
`patch_labels` then patches the first branch's `jp` to that far offset. When the
first operand is NaN (unordered, PF=1), the first `jp` is taken and control jumps
**past the second comparison and its conditional branch**, so `a<b OR c<d`
evaluates as if false even when `c<d` is true → the `GOTO L` is wrongly skipped.

The single correct behavior a fix produces: each ordered-only float branch skips
**only its own** conditional branch on unordered inputs; two such branches to the
same target never share a skip label.

Severity MEDIUM: a silent wrong-control-flow miscompile, but only when the first
operand of an ordered-only float compare is NaN/unordered. Finite inputs never
take the `jp`, so it passes all finite-valued tests and hardware validation.
plan-17 finiteness-observation traps *some* NaNs upstream, reducing (not
eliminating) reachability; NaNs from unobserved sources still reach comparisons.

References:

- `src/arch/x86_64/select.rs:560-591` (`x86_float_branch`) — `:569` (skip label
  named from `target` only), `:579-582` (the four ordered-only conditions using
  it; `b.lo` at `:580` too).
- `src/arch/x86_64/encode/mod.rs:98-108` (label sub-pass; `.insert` is
  last-writer-wins), `:111-114` (emit + `patch_labels`).
- `src/arch/x86_64/encode/emitter.rs:172-189` (`patch_labels`).
- Contrast: AArch64 emits a native `b.cc` for these relations with no synthesized
  skip label, so it is immune.
- Found during goal-01 review of `src/arch/x86_64/**`.

## Failing Reproduction

Structurally demonstrable; an end-to-end NaN repro needs a float `<`/`<=`/`=` in a
short-circuit boolean sharing a target label plus a NaN first operand:

```
LET a AS Float = zero() / zero()   ' NaN, from an unobserved source
IF a < 1.0 OR 2.0 < 3.0 THEN GOTO hit
PRINT "missed"
GOTO done
hit:
PRINT "hit"
done:
```

- Observed: with `a = NaN`, control skips the `2.0 < 3.0` test (which is true) and
  prints `missed` — the two `L__x86ford` labels collided.
- Expected: `2.0 < 3.0` is true, so `GOTO hit` is taken; prints `hit`.

Contrast: the same program with `a` finite works (the `jp` is not taken);
`b.gt`/`b.ge`/`b.ne`/`b.hi`/`b.lt`/`b.le`/`b.vs`/`b.vc` emit no skip label and
never collide even when sharing a target.

## Root Cause

`ordered_only` (`select.rs:568`) makes the skip-label name a pure function of
`target`, with no per-branch-site uniqueness. Two ordered-only float branches to
one target therefore synthesize identically-named labels; the last-writer-wins
label map (`mod.rs:100-102`) collapses them to one offset, and both forward `jp`s
point at it.

## Goal

- Each ordered-only float branch's skip label is unique to that branch site, so
  no two collide regardless of shared targets; NaN-first-operand control flow is
  correct.

### Non-goals (must NOT change)

- Do not change the truth sets of any float relation (the `jp/jcc` mapping is
  otherwise correct).
- Do not change AArch64 lowering.

## Blast Radius

- Every `ordered_only` caller: `b.mi`, `b.lo`, `b.ls`, `b.eq` (`select.rs:579-582`)
  and the corresponding fcmp-zero variants if they route here. All share the
  target-only skip name. Fixed by this bug.

## Fix Design

Make the skip label unique per branch site. Thread a monotonic counter (or the
instruction index) through `select_x86`/`x86_float_branch`/`ordered_only` and name
the label `{target}__x86ford_{n}`. Alternatively restructure to a target-relative
short forward jump over the single following `jcc` (`jp .+len(jcc)`), removing the
named label entirely — cleaner but requires the encoder to size a relative skip.
Recommended: the per-site counter (minimal, local to selection).

## Phases

### Phase 1 — failing test + audit

- [ ] Add a selection/encoder test: two ordered-only float branches to the same
      target must emit two distinct skip-label names; assert the first `jp`
      resolves to an offset immediately after the first `jcc`, not the second.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Give each `ordered_only` skip label a per-site-unique name.

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh`; NaN-input runtime test on x86-64 (glibc+musl).

## Validation Plan

- Regression test(s): the duplicate-skip-label selection test + a NaN-input
  end-to-end run of the reproduction on linux-x86_64.
- Runtime proof: the reproduction prints `hit` (not `missed`) with `a = NaN`.
- Full suite: `scripts/artifact-gate.sh` + x86 runtime validation.

## Summary

The risk is picking a uniqueness scheme that stays byte-identical for the
single-branch case (the common path); the fix is a per-site label suffix in
`ordered_only`.
