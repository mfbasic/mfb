# bug-1014: `field_kinds.tsv` is not what `field_kinds_gen.py` generates

Last updated: 2026-10-19
Effort: small (<1h)
Severity: LOW
Class: Other

Status: Closed
Regression Test: tests/guards/inplace_self_update_census.rs (`field_kinds_tsv_is_its_generators_output`)

## STATUS: FIXED (landed with plan-176, branch `worktree-P-176`)

Both sides were wrong, each in different lines. The generator lacked three rules the
compiler gained after plan-145-A (bug-766 D, bug-781, plan-162-C), and the table held
two kinds whose truth those changes moved (`http::Route`, `canvas::Text`) plus two
check expressions written by hand. The generator now encodes all three rules, the table
is its output byte for byte, and a guard holds the two equal. No compiler change: the
compiler is right at every line that moved (evidence below).

Deviations from the plan below: the 15 `canvas::Text` grid failures (filed nowhere) are
folded in here — same root cause, a kind whose truth moved under a hand-kept line. The
sibling `field_expect_gen.py` was found broken (it read plan-144's findings from
`planning/`, archived to `planning/completed/`) and is fixed; once it runs, its output
equals `field_expect.tsv` byte for byte. It gets no guard: its header sanctions hand
flips. `cases.tsv` has no generator (it is hand-kept and held to `mfb man` by
`every_self_update_shaped_overload_has_a_runtime_case`).

`tests/runtime/inplace_self_update/field_kinds.tsv` is documented as the output of
`field_kinds_gen.py`. Regenerating it changed lines that no source change accounted
for (35 at `fc530a22e`; 41 at `7fa2dc5dc`), so the file could not be regenerated
without re-baselining rows nobody meant to move.

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

- Observed: `35` at `fc530a22e`; `41` at `7fa2dc5dc` (2026-10-18, release compiler of
  that commit). Expected: `0`.

The grid fails 19 pairs at `7fa2dc5dc`:

```
MFB_SELF_UPDATE_FILTER='canvas::Text|http::Route' cargo test --release \
  --test rt_inplace_self_update every_field_kind -- --ignored
```

gave `19 of 45 case/site pair(s) failed`: `http::Route` at S4/S5/S9/S10 ("marked `arm`,
but 2000 more runs allocated 2000 more blocks (want < 250)"), and `canvas::Text` at all
15 sites (marked `na:TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`; 13 sites compile, S7/T7 fail
with `TYPE_FOR_EACH_REQUIRES_COLLECTION`'s "FOR EACH source has type canvas.Text").

## Root Cause

Hypothesis 1 holds: the table was hand-edited and the generator's rules were not.

1. **bug-766 D's 144 hand flips (36 kinds then, 39 now).** `b309807ec` flipped every
   `InlinedVariable` kind's S4/S5/S9/S10 from `rebuild:size-varies` to `arm` by hand,
   from a measured grid run, following the table's then-header ("a letter that lands a
   kind flips its sites by hand"). `field_kinds_gen.py:expects` still said
   `rebuild:size-varies` for the class. The arm is real
   (`builder_control.rs:replace_field_admitted`: the owner's last inlined `String` or
   record field, replaced in place when its 16-byte granule is unchanged, for a plain
   local or global owner; S3/S6 and STATE owners keep the rebuild). Lines 3 kinds later
   added by hand (plan-150, -164, -172-K) copied the flipped form.
2. **`http::Route` — bug-781 (`f37874271`).** A function value became owned by its one
   holder; a `FUNC` field became a pointer field, so `http::Route` (which holds its
   `handler`) stopped being inlined and is now class Pointer. bug-781 re-baselined
   `FIELD_KIND_TABLE` (`self_update.rs`: `("http.Route", FieldKindRow::Arm('C'))`, "a
   pointer field … replaced by arm C's pointer store") but neither the generator
   (`cls` still classed `FUNC` as `InlinedVariable`) nor the table. The table's line
   kept bug-766 D's `kindSame(x)` statement: storing a value borrowed from the field
   into an owning pointer field deep-copies it — the 1 block per run the grid reported,
   not a rebuild of the owner. Measured on `7fa2dc5dc`: the same line with bound
   `arm+value` still fails S4/S5/S9/S10 with "allocated 2000 more blocks (want <= 1.125
   x the control's 0)"; with a fresh value (`x = http::Route[pattern := "/", handler :=
   kindRoute]`, what every other Pointer kind's `kindFresh` gives), bound `arm+value`,
   and `arm` at S3–S6/S9/S10, the grid passes all 15 sites. So arm C serves it at every
   record site, as `FIELD_KIND_TABLE` says.
3. **`canvas::Text` — plan-162-C (`d2a8c3420`).** `Text.font` (`RES canvas::Font`)
   became `Text.fonts` (`List OF RES canvas::Font`). A collection is always defaultable
   (`src/ir/verify/resources.rs:is_defaultable`, bug-434: the default is the empty
   collection, no element is materialised), so `MUT x AS canvas::Text` now compiles —
   the compiler is right. The generator still listed `canvas::Text` in `UNBUILDABLE`
   ("they hold a live `RES` handle"), and the table's line was never regenerated. Not
   plan-172-N (enum defaults): `git log -S'name: "fonts"'` names only `d2a8c3420`.
4. **Two check expressions.** `canvas::Gradient`'s `toString(len(before.stops))` was
   generated by plan-145-A before plan-163-D (`7cedfaabd`) added `focalRadius`, its
   first printable field, and was never regenerated; `canvas::Emission`'s
   `toString(before.rate)` was hand-written by `f6b0b6d47` (the generator picks the first
   printable field, `maxParticles`). Four other rows (`canvas::ParticleSystem`,
   `Emission`, `Motion`, `Appearance`) sat in a hand-chosen order, not the `mfb man`
   order the generator emits.

## Goal

- The generator's output equals the committed file.
- A test fails when they differ.

### Non-goals (must NOT change)

- What the compiler does. This is a test-data bug.
- Do not regenerate the file to make the diff disappear without first establishing,
  from `rt_inplace_self_update`, which side is right.

## Blast Radius

- `tests/runtime/rt_inplace_self_update.rs` reads the `.tsv` (`include_str!`) — the
  moved lines are measured by it (Validation Plan).
- `tests/guards/inplace_self_update_census.rs` reads its kind column — unchanged set.
- `FIELD_KIND_TABLE` (`self_update.rs`), the unit twin — already right (bug-781 updated
  it); unaffected.
- `field_expect_gen.py` / `field_expect.tsv` — the generator could not run (stale
  findings path); fixed, output byte-identical to the table.
- `cases.tsv` — no generator; unaffected.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Guard `field_kinds_tsv_is_its_generators_output`: RED on the base files (the
      bug-766 D lines differ first), GREEN after.
- [x] Hypothesis 1 confirmed (`b309807ec`); the sibling pair checked.

Commit: `4010338aa` (the `field_expect_gen.py` path) and the bug-1014 fix commit after it

### Phase 2 — the fix

- [x] `field_kinds_gen.py`: bug-766 D's S4/S5/S9/S10 arm for `InlinedVariable`;
      `FUNC` is Pointer (bug-781); a Pointer kind with no default stores a fresh literal;
      `canvas::Text` leaves `UNBUILDABLE`; the header now forbids hand edits.
- [x] `field_kinds.tsv` regenerated: only `canvas::Text`, `http::Route`, the two check
      expressions, the header and the four-row order move.
- [x] `field_expect_gen.py`: the findings path.

Commit: `4010338aa` (the `field_expect_gen.py` path) and the bug-1014 fix commit after it

## Validation Plan

- `cargo test --test inplace_self_update_census`: 3 passed (RED, 1 failed, on
  the base generator and table).
- `MFB_SELF_UPDATE_FILTER='canvas::Text|http::Route|canvas::Emission|canvas::Gradient|canvas::ParticleSystem|canvas::Motion|canvas::Appearance' cargo test --release --test rt_inplace_self_update every_field_kind -- --ignored`:
  `ok. 1 passed` in 882s on `worktree-P-176` (2026-10-19) — every line that changed or moved.
- `bash scripts/test-fast.sh --test rt_inplace_self_update --test inplace_self_update_census` → `test-fast: ok`
  (census `3 passed`, `rt_inplace_self_update` `1 passed; 3 ignored`).

## Summary

A generated file that had drifted from its generator because the table, not the
generator, was where each landing letter recorded what it changed. The generator now
carries those rules and a guard stops the next hand edit.
