# bug-518: `datetime::withZone`'s `zone` parameter description says the opposite of what the function does

Last updated: 2026-09-04
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: **FIXED** — the `zone` parameter row now states that the instant is
preserved and names the operation that does the opposite.
Regression Test: `tests/rt-behavior/datetime/datetime-withzone-instant-rt`

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


## Resolution

### Which side was wrong, from evidence

The behaviour was right; the prose was wrong. Four independent pieces of
evidence, none of them "the code is easier to leave alone":

1. The `BODY` is one line — `RETURN __datetime_inZone(__datetime_resolve(dt), z)`
   — which collapses to the instant and re-projects it.
2. The spike measures it: instant `1782475200` before and after.
3. Every *other* sentence on the page already agrees with the body: the `intro`
   ("preserving the absolute instant"), the Description ("the underlying point
   on the UTC timeline is unchanged … an identity on the absolute moment"), the
   "exactly the composition of `resolve` and `inZone`" paragraph, and both
   examples. One sentence disagreed with five.
4. The wrong sentence's own advice does not typecheck: it said to "use
   `datetime::inZone` to keep the instant", but `inZone` takes an `Instant`, not
   a `DateTime`, so it is not a substitution — and it is the *second half of
   what `withZone` already does*.

Changing the code to match the row would have moved every existing caller's
instant and made the other five statements wrong instead. The prose was the
defect, exactly as the bug's Non-goals said.

### Sibling verdicts (Phase 1)

`grep -rn "reinterpret\|different instant\|names a different"` over
`src/codegen/builtins/datetime/` and `src/docs/spec/stdlib/02_datetime.md`:

- `func_in_zone.rs` — **clean.** Its `at` row says "the instant itself does not
  change; only the zone it is read in does". No mirrored copy.
- `func_to_utc.rs` — **clean** ("The instant to read in UTC.").
- `func_to_local.rs` — **clean** ("The instant to read in the host's local zone.").
- `func_civil.rs:97` — a hit, but **correct**: "the same date and time in two
  zones are two different instants" is true of `civil`, which is precisely the
  operation the `withZone` row was describing by mistake.
- `src/docs/spec/**` — no restatement of the wrong claim; the spec's one
  `withZone` sentence was already right, and now also draws the contrast.

That last verdict is what improved the fix over the one the bug proposed. The
bug recommended saying "the package has no such operation". It has one:
`datetime::civil(dt.date, dt.time, zone)`, confirmed at runtime —

```
dt    = 2026-06-26T12:00:00.000Z        inst=1782475200
withZone(dt, +05:30) = 2026-06-26T17:30:00.000+05:30  inst=1782475200   ' instant kept
civil(dt.date, dt.time, +05:30) = 2026-06-26T12:00:00.000+05:30  inst=1782455400  ' reading kept
```

so the row names it instead of leaving the reader with a dead end. The
`replaceZone` feature the Open Decision floats is therefore **not needed** —
that was the deferral's premise, and it is now a documented one-liner.

### The change

- `func_with_zone.rs` — the `zone` `desc`, plus a module-doc note recording why
  the code must not be "fixed" to match the old sentence.
- `src/docs/spec/stdlib/02_datetime.md` — the projection section now states the
  `resolve(withZone(dt, z)) = resolve(dt)` identity, names `civil` as the
  opposite operation, and cites the pin.
- `spikes/api-review/bug-518-withzone-doc` — extended to print the contrast.

### The pin

`tests/rt-behavior/datetime/datetime-withzone-instant-rt` asserts
`resolve(withZone(dt, z)) == resolve(dt)` for **every** `ZoneKind` — UTC, a
positive and a negative fixed offset, and the host's `Local` zone — plus a
re-projection back to UTC, and that `nanos` and the carried `zone.kind` survive.
Its assertions are invariants (`TRUE`), not host-zone-dependent values, so the
golden is portable. It also pins the *contrast*: `civil(dt.date, dt.time, z)`
keeps the reading and moves the instant by 19800 s.

This is a pin, not a red test — the red artefact was the man page, which no
harness can assert on. What the pin buys is that a future change to `withZone`
that would make the old sentence true breaks a test.

### Gates

- `scripts/test-accept.sh` (full, 1396 tests): **passed, 0 mismatches.** Worth
  recording: a `Parameter.desc` is man-page-only prose and drifted no `.ir`
  golden, unlike a `RegistryFunction.desc`/`intro` edit — both sides were
  checked, not assumed.
- `scripts/regen-ncodesum.sh`: 141 refreshed, **0 changed**.
- `scripts/artifact-gate.sh target/release/mfb all`: 1906 goldens, **0 diffs**.
- `cargo test --no-fail-fast`: exit 0, 4711 passed / 0 failed.
  `cargo check --all-targets`: no warnings.
- `scripts/man-run-examples.sh datetime --run`: 114 built, 114 ran, 0 failed.
- `scripts/man-census.sh --memory-scope`: **0 unclassified** memory-vocabulary
  hits; datetime's 15 are the pre-existing arithmetic-borrow carve-out.

### Open Decision, answered

Add a `replaceZone`? **No.** The gap the Open Decision named — "constructing
'9am in Tokyo' from a `DateTime` currently requires going out through `civil`" —
is one call, `civil(dt.date, dt.time, zone)`, and the page now says so. A new
member for it would be a third spelling of an operation the package already
has. See bug-520 for the constraint this places on a future `Named` zone kind.