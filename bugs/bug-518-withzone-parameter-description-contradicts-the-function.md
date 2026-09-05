# bug-518: `datetime::withZone`'s `zone` parameter description says the opposite of what the function does

Last updated: 2026-09-04
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `spikes/api-review/bug-518-withzone-doc/` promoted to a `tests/` fixture

`mfb man datetime withZone` contains two statements that cannot both be true.

The `zone` parameter row says:

> The new zone. This **reinterprets** the same wall-clock reading in a different
> zone, so it names **a different instant** — use `datetime::inZone` to keep the
> instant and change only how it is displayed.

The Description says:

> The underlying point on the UTC timeline is unchanged … `withZone` is an
> identity on the absolute moment and changes only its civil presentation.

The implementation settles it in one line
(`src/codegen/builtins/datetime/func_with_zone.rs:59`):

```
FUNC __datetime_withZone(dt AS DateTime, z AS Zone) AS DateTime
  RETURN __datetime_inZone(__datetime_resolve(dt), z)
END FUNC
```

`resolve` collapses to the instant; `inZone` re-projects it. The instant is
preserved. The parameter row is wrong, and it is wrong in the most damaging
possible direction: it tells the reader to reach for `datetime::inZone`
instead — which is a function taking an `Instant`, not a `DateTime`, so the
advice does not even typecheck as a substitution.

The single correct behavior a fix produces: the `zone` parameter description
agrees with the function, and the reader is not sent to a different member on
a false premise.

References:

- `src/codegen/builtins/datetime/func_with_zone.rs:83` — the wrong `desc`
- `src/codegen/builtins/datetime/func_with_zone.rs:59-62` — the `BODY` that disproves it
- Spike: `spikes/api-review/bug-518-withzone-doc/`

## Failing Reproduction

```
./target/release/mfb man datetime withZone
./target/release/mfb build spikes/api-review/bug-518-withzone-doc
./spikes/api-review/bug-518-withzone-doc/build/mfb_project.out
```

- Observed:

```
civil before   = 2026-06-26T12:00:00.000Z
civil after    = 2026-06-26T17:30:00.000+05:30
instant before = 1782475200
instant after  = 1782475200
=> the instant is PRESERVED. The `zone` parameter description is wrong.
```

- Expected: the parameter description describes this — the same instant, read
  by an observer in the new zone.

Contrast case, documented correctly: `datetime::inZone`'s own page describes
instant-preserving projection accurately, which is why the cross-reference in
the wrong sentence is doubly confusing — it points at a function that does the
same thing `withZone` does, as if it were the alternative.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ (prose defect; target-independent) |

## Root Cause

`src/codegen/builtins/datetime/func_with_zone.rs:83` sets the `zone`
parameter's `desc` to the "reinterprets … a different instant" text. Registry
prose fields are `&'static str` the compiler never reads, so no gate catches a
descriptor whose text contradicts the `BODY` two screens above it — `mfb man`
output is the only verification, per `.ai/man-content.md`.

The text reads like it was written for a *different* function: one that keeps
the civil fields and swaps the zone (a `replaceZone`), which is a real and
useful operation this package does not have. That is worth noting, because the
temptation will be to "fix" the code to match the doc.

## Goal

- `mfb man datetime withZone`'s `zone` row states that the instant is
  preserved and that the civil fields are re-derived for the new zone.
- The row no longer redirects to `datetime::inZone` as an alternative with
  different semantics.
- A test pins the instant-preserving property, so a future change to
  `withZone` breaks a test rather than silently making the old prose true.

### Non-goals (must NOT change)

- `withZone`'s behavior. `inZone(resolve(dt), z)` is the correct and documented
  semantic; the Description, the "exactly the composition of" paragraph, and
  the two examples are all consistent with it.
- `datetime::inZone`, which is fine.
- **Tempting wrong fix, forbidden:** changing the implementation so the
  parameter row becomes true. That would silently move every existing caller's
  instant, and the Description, the composition paragraph, and both examples
  would all become wrong instead. The prose is the defect.

## Blast Radius

`grep -rn "reinterpret\|different instant" src/codegen/builtins/datetime/`:

- `func_with_zone.rs:83` — fixed by this bug.
- `func_in_zone.rs` — unaffected; verify in Phase 1 that it does not carry a
  mirrored version of the same sentence.
- `func_to_utc.rs`, `func_to_local.rs` — both are zone re-projections and will
  have described the same property. Check each for the same confusion and fix
  in the same change if present.
- `src/docs/spec/**` — gated by nothing (`.ai/man-content.md`); grep for a
  restatement of `withZone`'s contract.

## Fix Design

Replace the `desc` at `func_with_zone.rs:83` with text that matches the
Description: the new zone to read the same moment in; the civil fields and
offset are re-derived, the instant is unchanged; and — since the reader's real
question is probably "how do I change the zone *without* moving the reading?" —
say plainly that the package has no such operation, rather than pointing at
`inZone`.

Then add the regression pin: a fixture asserting
`resolve(withZone(dt, z)) == resolve(dt)` for a UTC, a fixed-offset and a local
zone. Prose cannot be gated, but the property it describes can be.

Rejected: leaving the sentence and adding a clarifying note. Two contradictory
statements plus a note is worse than one correct statement.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-518-withzone-doc/` (done).
- [ ] Add the `resolve(withZone(dt, z)) == resolve(dt)` fixture. It passes
      today — it is a *pin*, not a red test; the red artefact here is the man
      page, which no harness can assert on.
- [ ] Read `func_in_zone.rs`, `func_to_utc.rs` and `func_to_local.rs` for the
      same mirrored sentence; record verdicts in Blast Radius.

Acceptance: the pin passes; the three sibling pages have verdicts.
Commit: —

### Phase 2 — the fix

- [ ] Rewrite the `zone` `desc` at `func_with_zone.rs:83`.
- [ ] Apply the same correction to any sibling found in Phase 1.

Acceptance: `mfb man datetime withZone` reads consistently top to bottom; the
pin still passes.
Commit: —

### Phase 3 — validation

- [ ] `scripts/man-run-examples.sh datetime --run`.
- [ ] `scripts/man-census.sh --memory-scope`.
- [ ] `cargo test --no-fail-fast -- datetime`; scope the run to the blast
      radius — this is a prose change plus one new pin.

Acceptance: examples compile and run; census clean; datetime tests green.
Commit: —

## Validation Plan

- Regression test: the instant-preservation pin.
- Runtime proof: `spikes/api-review/bug-518-withzone-doc/`.
- Doc sync: `func_with_zone.rs`; `src/docs/spec/**` if it restates the contract.
- Full suite: scoped — `cargo test --no-fail-fast -- datetime` plus the man
  harness. A one-string prose change does not warrant the full suite.

## Open Decisions

- Whether to add the operation the wrong sentence describes — a `replaceZone`
  that keeps the civil fields and re-derives the instant. **Recommend
  deferring**: it is a genuine gap (constructing "9am in Tokyo" from a
  `DateTime` currently requires going out through `civil`), but it is a feature,
  and this bug should not grow one.

## Summary

Trivial to fix and easy to fix *wrongly*: the sentence describes a plausible
operation, so the risk is someone changing the code to match it. The pin added
in Phase 1 is the guard against that. One `&'static str`, one new test.
