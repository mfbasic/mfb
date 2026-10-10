# bug-1014: `field_kinds.tsv` is not what `field_kinds_gen.py` generates

Last updated: 2026-10-10
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Open
Regression Test: none yet — see Phase 1

`tests/runtime/inplace_self_update/field_kinds.tsv` is documented as the output of
`field_kinds_gen.py`. Regenerating it changes 35 lines that no source change accounts
for, so the file cannot be regenerated without re-baselining rows nobody meant to move.

**The single correct behavior a fix produces:** running the generator against the tree's
own compiler reproduces the committed file byte for byte, and a test holds it there.

References:

- `tests/runtime/inplace_self_update/field_kinds_gen.py` (its header gives the command).
- Found during plan-172-B, which had to hand-edit the four canvas rows instead of
  regenerating.

## Failing Reproduction

```
python3 tests/runtime/inplace_self_update/field_kinds_gen.py target/release/mfb > /tmp/fk.tsv
diff tests/runtime/inplace_self_update/field_kinds.tsv /tmp/fk.tsv | grep -c '^<'
```

- Observed: `35`, measured at `fc530a22e` (before plan-172-B touched either file) with
  that commit's release compiler. The rows are not canvas ones: for example
  `audio::AudioDevice`, `big::Int` and `big::DivResult` are committed as
  `S4=arm S5=arm` and generated as `S4=rebuild:size-varies S5=rebuild:size-varies`.
- Expected: `0`.

## Root Cause

Unknown. Two hypotheses, to be told apart by `git log -p` on both files:

1. The `.tsv` was hand-edited when the compiler gained arms (plan-145-C to -H) and the
   generator's prediction rules (`expects`) were not updated to match.
2. The generator was updated and the `.tsv` was never regenerated.

`tests/runtime/rt_inplace_self_update.rs` measures the real behaviour, so whichever
side it agrees with is the correct one.

## Goal

- The generator's output equals the committed file.
- A test fails when they differ.

### Non-goals (must NOT change)

- What the compiler does. This is a test-data bug.
- Do not regenerate the file to make the diff disappear without first establishing,
  from `rt_inplace_self_update`, which side is right.

## Blast Radius

- `tests/runtime/rt_inplace_self_update.rs` and
  `tests/guards/inplace_self_update_census.rs` read the `.tsv` — in scope.
- `cases.tsv` and its generator, the sibling pair — not checked; check it in Phase 1.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a guard test that runs the generator and diffs. Confirm it fails with 35.
- [ ] Decide between the two hypotheses; check the `cases.tsv` pair the same way.

Acceptance: the guard fails for the documented reason; the cause is recorded here.
Commit: —

### Phase 2 — the fix

- [ ] Correct whichever side is wrong.

Acceptance: the guard passes; `cargo test --test rt_inplace_self_update` passes.
Commit: —

## Validation Plan

- Regression test: the Phase 1 guard.
- Full suite: `cargo test --test rt_inplace_self_update --test inplace_self_update_census`.

## Summary

A generated file that has drifted from its generator. Low risk; the cost is that every
plan touching a record type must edit it by hand until it is fixed.
