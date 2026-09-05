# bug-519: `datetime::parse` silently rolls an out-of-range date into a different valid date, and its man page misdescribes the behavior

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: **FIXED** — `datetime::parse` and `datetime::parseIso` now apply the
`date`/`time` bounds and raise `ErrInvalidFormat` (77050003).
Regression Test: `tests/rt-behavior/datetime/datetime-parse-range-rt` (19 rejection
cases + 17 positive pins + an `addDays`/`addMonths` rollover pin)

`datetime::parse` performs no range check on the calendar fields it decodes.
Parsing `"2026-13-45 25:70:99"` with `"yyyy-MM-dd HH:mm:ss"` succeeds and
returns `2027-02-15T02:11:39Z` — a date the caller never wrote, seven weeks and
several hours away from anything in the input, with no error and no warning.

The same fields handed to the constructor are refused:
`datetime::date(2026, 13, 45)` raises `ErrInvalidArgument` (77050002). So the
package holds two incompatible positions on whether month 13 is a date, and the
one that accepts it is the one fed by untrusted text.

The man page is wrong about this too, in a way that hides it. It says:

> `parse` does not range-check the decoded calendar fields the way
> `datetime::date` and `datetime::time` do: an out-of-range component in `value`
> (for example month 13) is **carried into** the resulting `datetime::DateTime`
> rather than rejected.

A reader takes that as a promise that `dt.date.month == 13` — inspectable,
detectable, defensible as "garbage in, garbage out". It is not what happens.
The out-of-range field is *normalized away*, so by the time the value reaches
the caller there is nothing left to detect.

The single correct behavior a fix produces: `datetime::parse` applies the same
range checks as `datetime::date`/`datetime::time` and raises `ErrInvalidFormat`
on `"2026-13-45"`. This is the silent-wrong-value class — nothing downstream
ever reports it.

References:

- `src/codegen/builtins/datetime/func_parse.rs` — the decoder and its "does not
  range-check" paragraph
- `src/codegen/builtins/datetime/func_date.rs`, `func_time.rs` — the checks
  that `parse` bypasses
- `src/codegen/builtins/datetime/helper_days_from_civil.rs`,
  `helper_build_from_fields.rs` — where normalization happens
- Spike: `spikes/api-review/bug-519-parse-normalizes/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-519-parse-normalizes
./spikes/api-review/bug-519-parse-normalizes/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
input text: 2026-13-45 25:70:99

datetime::parse  -> year=2027 month=2 day=15
                    hour=2 minute=11 second=39
                    toIso = 2027-02-15T02:11:39.000Z

datetime::date(2026, 13, 45) -> REJECTED code=77050002
```

- Expected: `datetime::parse("2026-13-45 25:70:99", …)` raises
  `ErrInvalidFormat` (77050003), matching the way the page already describes
  every *other* mismatch ("Text that does not match the pattern raises
  `ErrInvalidFormat`").

Contrast cases, correct today, that bound the bug:

- `datetime::date(2026, 13, 45)` raises. The constructor path is right.
- The offset token *is* range-checked — the page says "The one validated
  numeric range is the offset token, whose magnitude must be under 24 hours."
  So the decoder already has a place to put a range check; it is used once.
- `datetime::parse("not-a-date", "yyyy-MM-dd")` raises `ErrInvalidFormat`. A
  *shape* mismatch is caught; only a *value* mismatch is not.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | `parse` is a pure `Body::mfb` software core; expected identical. Confirm in Phase 3 |

## Root Cause

Two separate defects that compound.

**1. No range check.** `func_parse.rs` decodes each token into an integer and
hands the tuple to the shared field-assembly helper without validating it.
`datetime::date` and `datetime::time` validate before assembling; `parse` calls
the assembly path directly.

**2. The assembly path normalizes.** The civil-days helper
(`helper_days_from_civil.rs`) converts `(year, month, day)` to a day count using
arithmetic that is *total* — month 13 is simply "twelve months plus one", day 45
is "day 1 plus 44 days". That totality is correct and desirable for
`datetime::addMonths`/`addDays`, which need exactly that rollover. Reached
without a prior range check, it silently launders invalid input into valid
output.

This is why the man page's claim is wrong: whoever wrote "carried into the
resulting `DateTime`" described the *intent* of skipping validation, not the
behavior of the code that skipping validation reaches.

## Goal

- `datetime::parse` raises `ErrInvalidFormat` for any decoded calendar field
  outside the range `datetime::date`/`datetime::time` accept: month 1..12,
  day 1..`daysInMonth(year, month)`, hour 0..23, minute 0..59, second 0..59,
  nanos 0..999_999_999.
- The "does not range-check" paragraph is deleted from
  `mfb man datetime parse` and replaced by a statement of the check.
- `datetime::addMonths`/`addDays` keep their rollover arithmetic unchanged.

### Non-goals (must NOT change)

- The rollover semantics of `addDays`, `addMonths`, `plus`, `minus`, and the
  helpers they share. Those *must* normalize; the fix belongs in `parse`, not
  in `helper_days_from_civil.rs`.
- The offset token's existing ±24h check.
- The weekday token's documented laxity ("the letters are read but not
  validated" — `EEE`/`EEEE` deliberately do not cross-check against the date).
  That is a stated design choice about a redundant field, not a range check.
- `datetime::parseIso`, unless Phase 1 finds it shares the decoder — see
  Blast Radius.
- **Tempting wrong fix, forbidden:** correcting only the man page, so that it
  accurately describes the normalization. That documents a silent wrong value
  instead of removing it, and leaves `parse` and `date` disagreeing about what
  a date is.

## Blast Radius

`grep -rn "days_from_civil\|build_from_fields" src/codegen/builtins/datetime/`:

- `func_parse.rs` — fixed by this bug.
- `func_parse_iso.rs` — **must be checked in Phase 1.** If it routes through
  the same unvalidated assembly, `parseIso("2026-13-45T00:00:00Z")` has the
  identical defect and is fixed in the same change. ISO input is even more
  likely to be machine-generated and untrusted.
- `func_date.rs`, `func_time.rs` — unaffected; they already validate.
- `func_add_days.rs`, `func_add_months.rs`, `func_plus.rs`, `func_minus.rs` —
  unaffected and must stay so; rollover is their contract.
- `func_civil.rs` — check whether it validates or trusts its `Date`/`Time`
  arguments. If those can only come from `date`/`time`/`parse`, fixing `parse`
  closes the hole.
- `csv`, `json`, `http` — any in-tree caller that parses a timestamp from
  input. `grep -rn "datetime::parse\|datetime.parse" src/ examples/` in Phase 1;
  each becomes a place the fix turns a silent wrong date into a raise, which is
  a behavior change those callers must be ready for.

## Fix Design

Validate in `func_parse.rs`, immediately after decoding and before assembly,
using the same bounds `func_date.rs`/`func_time.rs` use. The day bound is
month- and year-dependent, so it must call the existing
`datetime::daysInMonth` logic rather than a hardcoded 31 — otherwise
`"2026-02-30"` still slips through and the fix is half-done.

Raise `ErrInvalidFormat` (77050003), not `ErrInvalidArgument` (77050002). The
argument is fine; the *text* is malformed, and `ErrInvalidFormat` is already
what `parse` raises for a shape mismatch. A caller wrapping `parse` in a TRAP
should not have to catch two codes for two flavours of bad input.

The correctness risk is in the ordering: the check must run *after* the offset
token has been applied to select the zone but *before* the civil fields are
converted to a day count, or a legitimate value near a month boundary could be
rejected.

Rejected: adding a `strict AS Boolean` parameter defaulting to `FALSE`. It
keeps the dangerous behavior as the default, which is the whole problem, and
adds a parameter every caller must now think about.

Rejected: making the range check a compile-time warning. The input is runtime
text; there is nothing to see at compile time.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-519-parse-normalizes/` (done).
- [ ] Add fixtures asserting the desired raise for: month 13, month 0, day 45,
      day 30 in February, day 32, hour 25, minute 70, second 99. Confirm each
      currently succeeds with a rolled-over value, and record what each
      currently returns.
- [ ] Read `func_parse_iso.rs`: does it share the unvalidated assembly? Record
      the verdict.
- [ ] `grep -rn "datetime::parse" src/ examples/ benchmark/` — list every
      in-tree caller and whether the new raise affects it.

Acceptance: every fixture fails with a documented rolled-over value; the
`parseIso` verdict and the caller list are written into this file.
Commit: —

### Phase 2 — the fix

- [ ] Range-check the decoded fields in `func_parse.rs` before assembly, using
      the month-aware day bound.
- [ ] Apply the same fix to `func_parse_iso.rs` if Phase 1 found it shares the path.
- [ ] Delete the "does not range-check" paragraph; state the check instead.
- [ ] Update any in-tree caller from Phase 1 that now needs a TRAP.

Acceptance: all Phase 1 fixtures pass; `datetime::parse("2026-06-26 …")` and
every existing valid-input test still succeed; `addDays`/`addMonths` rollover
tests untouched and green.
Commit: —

### Phase 3 — regenerate + full validation

- [ ] Regenerate the `.ncode`/`.ncodesum` goldens the new checks shift; confirm
      the delta is datetime's only.
- [ ] `cargo test --no-fail-fast` (full — this changes semantics of a widely
      used member, so the scoped-run exemption does not apply).
- [ ] `scripts/test-accept.sh`.
- [ ] Re-run the spike on Linux and Windows; complete the matrix.
- [ ] `scripts/man-run-examples.sh datetime --run`.

Acceptance: full suite green; golden deltas are only datetime's; the
reproduction raises on every platform.
Commit: —

## Validation Plan

- Regression tests: the eight out-of-range fixtures from Phase 1, plus a
  positive fixture per token proving valid input still parses.
- Runtime proof: `spikes/api-review/bug-519-parse-normalizes/` — the parse must
  raise where it currently prints `2027-02-15T02:11:39.000Z`.
- Doc sync: `func_parse.rs` (and `func_parse_iso.rs` if in scope);
  `src/docs/spec/**` if it restates `parse`'s laxity.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Whether `datetime::civil` should also validate its `Date`/`Time` arguments
  defensively. **Recommend not** — if `date`, `time` and `parse` all validate,
  an invalid `Date` cannot be constructed, and a redundant check costs every
  caller. Revisit only if Phase 1 finds a fourth way to build one.

## Summary

The engineering risk is the behavior change, not the code: `parse` currently
succeeds on input that will now raise, so any in-tree or user code relying on
the rollover breaks loudly. That is the correct outcome — the alternative is
what the spike prints — but the Phase 1 caller audit is what makes it a managed
change rather than a surprise. The rollover helpers that `addDays`/`addMonths`
depend on are deliberately untouched.


## Resolution

### Phase 1 findings

**`parseIso` shares the defect** and is fixed in the same change. It does not
route through `__datetime_buildFromFields` — it builds `Date[...]`/`Time[...]`
record literals itself — but record literals do not validate either, so the
outcome is the same class with a different shape: `parseIso` *does* carry the
bad field into the returned value (`parseIso("2026-13-01T00:00:00Z")` printed
`2026-13-01T00:00:00.000Z`), and the normalization happens later, the first time
anything calls `resolve` on it. Worse than `parse`, not better: the wrong value
is latent instead of immediate.

**In-tree callers of `parse`/`parseIso`** (`grep -rn "datetime::parse" src
examples benchmark tests spikes`): `src/ir/tests.rs:4907,5805` (arity-padding
lowering assertions), `benchmark/mfb/src/datetimeb.mfb:71`,
`tests/rt-behavior/datetime/datetime-parse-valid`,
`tests/rt-behavior/datetime/datetime-parse-trap-rt`,
`tests/rt-behavior/datetime/datetime-invalid`,
`tests/rt-behavior/general/stdlib-error-code-contracts-rt`,
`tests/byte-identity/datetime`, and the three `spikes/api-review` projects.
**Every one passes an in-range date**, so no caller needed a new `TRAP`. The
full acceptance run confirms it: not one `.run` golden moved.

**Pre-fix behavior of each fixture case**, recorded from the RED run:

```
month13  -> 2027-01-01   month0 -> 2025-12-01   day45 -> 2026-07-15
day0     -> 2026-05-31   day32  -> 2026-02-01   feb30 -> 2026-03-02
feb29common -> 2026-03-01 apr31 -> 2026-05-01   hour25 -> 2026-06-27T01:00
hour24   -> 2026-06-27   minute70 -> 00:70->01:10  second99 -> 00:00:99->00:01:39
allbad   -> 2027-02-15T02:11:39Z                 pm25 -> 2026-06-27T01:00
isoMonth13 -> 2026-13-01T00:00:00.000Z (carried, not normalized)
isoFeb30/isoHour24/isoMinute70/isoSecond99 -> likewise carried verbatim
```

### The contract, and which side was wrong

Both sides were wrong, in different ways, and the code was the one worth
changing:

- The man page said an out-of-range field "is **carried into** the resulting
  `DateTime` rather than rejected". For `parse` that was simply false — the
  field was normalized away, leaving nothing to inspect. For `parseIso` it was
  accidentally true, and detectable only until the value was resolved.
- A *deliberate* silent-normalization contract would have to be stated on the
  page and give the caller a way to detect it. Neither existed, and the
  package's own constructors (`date`, `time`) already refuse the same fields —
  as does `parse`'s one existing numeric bound, the offset token. Two positions
  on what a date is, with the permissive one fed by untrusted text.

So the code was changed to match the package's stated position, and the prose
was replaced by a statement of the check.

### The fix

- New shared helper `__datetime_checkFields(year, month, day, hour, minute,
  second, nanos)` (`src/codegen/builtins/datetime/helper_check_fields.rs`),
  bounds identical to `date`/`time`, raising **`ErrInvalidFormat` (77050003)**
  rather than the constructors' `ErrInvalidArgument`: the argument is a
  well-formed `String`, the *text* is malformed, and 77050003 is what both
  readers already raise for a shape mismatch, so one `TRAP` still catches every
  flavour of bad text.
- `__datetime_buildFromFields` calls it **after** the 12-hour/AM-PM fold and
  before the `Date`/`Time` literals, so `hh`+`a` is bounded as the hour actually
  stored. `parseIso` calls it after the offset read, at the same point.
- `helper_days_from_civil.rs` and every rollover member
  (`addDays`/`addMonths`/`plus`/`minus`) are untouched — the fixture pins
  `addMonths(2026-01-31, 1) = 2026-02-28` and `addDays(2026-01-31, 1) =
  2026-02-01` unchanged.

### Doc sync

- `mfb man datetime parse` — the "does not range-check" paragraph is replaced by
  the bounds and a "there is no rollover" sentence, plus a new compiled-and-run
  example whose printed output (`rejected: datetime: month out of range`) is on
  the page.
- `mfb man datetime parseIso` — the mirrored paragraph, likewise.
- `mfb spec stdlib datetime` — new "Decoded fields are range-checked" subsection
  under Parse grammar, and the Validation section now names three input
  boundaries rather than two.
- `spikes/api-review/bug-519-parse-normalizes` re-run: all three of `parse`,
  `parseIso` and `date` now refuse the same fields.

### Gates

- `datetime-parse-range-rt`: 19 rejections all FALSE→TRUE; **17 positive pins
  and both rollover pins byte-identical to the pre-fix run.**
- `scripts/man-run-examples.sh datetime --run`: 113 built, 113 ran, 0 failed.
- `scripts/test-accept.sh` (full, 1394 tests): 15 mismatches, **all `.ir`, all
  datetime-importing fixtures, zero `.run` goldens moved** — that containment is
  the semantics-preservation proof. Regenerated; re-run green.
- `scripts/regen-ncodesum.sh`: 141 goldens refreshed, only datetime's 5 changed.
- `scripts/artifact-gate.sh target/release/mfb all`: 1372 tests, 1902 goldens,
  **0 diffs**.
- `cargo test --no-fail-fast`: 4710 passed, 0 failed across 126 targets.

### Not done here

- bug-520 (no named zones) is untouched. The fix does not interact with it: the
  bound is on calendar fields, not on zone identity.
- The weekday token's documented laxity, the offset token's ±24h check, and
  `civil`'s trust in its `Date`/`Time` arguments are all unchanged. `civil` is
  now provably safe to leave trusting: with `date`, `time`, `parse` and
  `parseIso` all bounded, an out-of-range `Date`/`Time` is not constructible
  from outside the package.