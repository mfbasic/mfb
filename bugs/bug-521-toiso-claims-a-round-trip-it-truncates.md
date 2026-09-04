# bug-521: `datetime::toIso` is documented as round-trippable but truncates nanoseconds

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `tests/` — new `rt_datetime_iso_nanos` fixture (Phase 1)

`mfb man datetime toIso` makes two claims a few sentences apart:

> The `nanos` of `dt` are **truncated to milliseconds** for the `fff` field.

> The output is **round-trippable**: `datetime::parseIso` parses a string
> produced by `toIso` back into an **equivalent** `datetime::DateTime`.

They are incompatible for any value with sub-millisecond precision, which is
most of them: `datetime::now()` and `datetime::time(h, m, s, nanos)` both carry
nanoseconds, and `datetime::Instant.nanos` is documented as `0..999_999_999`.
A value that goes out through `toIso` and back loses up to 999,999 ns, silently.

"Equivalent" is doing all the work in that sentence, and a reader has no way to
know it means "equal to the millisecond". There is no `toIso` overload or
format flag that preserves the full precision the type carries, so a program
that must not lose it has to abandon `toIso` and hand-roll
`datetime::format(dt, "yyyy-MM-dd'T'HH:mm:ss.fffffffffZ")`.

The single correct behavior a fix produces: `toIso` gains a way to emit full
precision, and its page stops promising a round trip it does not perform.

References:

- `src/codegen/builtins/datetime/func_to_iso.rs` — the fixed
  `yyyy-MM-dd'T'HH:mm:ss.fffZ` pattern and both claims
- `src/codegen/builtins/datetime/func_parse_iso.rs`
- `mfb man datetime types` — `Time.nanos`, `Instant.nanos` are `0..999_999_999`
- RFC 3339 §5.6 — `time-secfrac` permits any number of fractional digits
- Spike: `spikes/api-review/bug-521-toiso-nanos/`

## Failing Reproduction

```
./target/release/mfb build spikes/api-review/bug-521-toiso-nanos
./spikes/api-review/bug-521-toiso-nanos/build/mfb_project.out
```

- Observed (macOS aarch64, release):

```
nanos in  = 123456789
toIso     = 2026-06-26T01:02:03.123Z
nanos out = 123000000
lost      = 456789 ns
=> round-trip is LOSSY; there is no toIso overload that keeps nanos
```

- Expected: either the round trip is lossless, or the page does not call it a
  round trip. Preferably both — a `toIso` that can emit the full `nanos`, and a
  page that states the precision of each form.

Contrast cases, correct today:

- `datetime::format(dt, "yyyy-MM-dd'T'HH:mm:ss.fffffffffZ")` emits all nine
  digits, so the *capability* exists; `toIso` just does not reach it. Confirm
  this in Phase 1 — it decides whether the fix is a new pattern constant or new
  formatting machinery.
- `datetime::parseIso` accepts `fff..fffffffff` on input (the `parse` page
  documents the fractional token as reading its run length), so the *reader*
  side is already precision-flexible. Only the writer truncates.
- `datetime::toMillis`/`toNanos` are explicit about their unit and lose nothing
  unexpectedly. `toIso` is the only member whose name does not warn.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ |
| Linux / Windows | — | pure software formatting; expected identical. Confirm in Phase 3 |

## Root Cause

`func_to_iso.rs` is documented as "the convenience form of `datetime::format`
invoked with the fixed pattern `yyyy-MM-dd'T'HH:mm:ss.fffZ`". `fff` is exactly
three fractional digits, so the truncation is baked into the constant. There is
one `toIso` implementation and one pattern, so there is no precision to choose.

The round-trip sentence appears to have been written against `parseIso`'s
tolerance — `parseIso(toIso(dt))` does parse, and does produce a `DateTime` —
rather than against equality of the values. The claim is true for the fields it
was checked on and false for the one it was not.

## Goal

- A `DateTime` carrying sub-millisecond precision can be rendered to an
  RFC 3339 string and read back with `nanos` intact.
- `mfb man datetime toIso` states the precision of what it emits, and either
  drops the round-trip claim or qualifies it as millisecond-exact.
- The default `toIso(dt)` output is unchanged.

### Non-goals (must NOT change)

- The existing single-argument `toIso(dt)` output shape. `…​.fffZ` is a
  widely-consumed format; changing its digit count would shift every acceptance
  golden and break peers that parse a fixed-width field. The new precision must
  arrive as an overload or an argument, not as a change to the default.
- `datetime::parseIso`'s input tolerance, which is already correct.
- `datetime::format`, which already offers `fff..fffffffff`.
- **Tempting wrong fix, forbidden:** deleting the "round-trippable" sentence
  and leaving `toIso` millisecond-only. That makes the page honest and leaves
  the language with no lossless ISO writer, which is the actual gap — a caller
  who needs one is then pushed to hand-write a format string and get the
  offset-token spelling wrong.

## Blast Radius

`grep -rn "toIso\|to_iso" src/ examples/ benchmark/`:

- `func_to_iso.rs` — fixed by this bug.
- `func_parse_iso.rs` — unaffected (already tolerant); verify it accepts nine
  digits in Phase 1 rather than assuming it.
- Every acceptance golden containing an ISO timestamp — unaffected as long as
  the default output is unchanged. This is the reason the default must not move.
- `json`, `csv`, `http` — any in-tree serializer emitting a timestamp. List
  them in Phase 1; each is a place where the truncation is currently silent and
  where the new form may be the better default *for that consumer*.
- `datetime::now()` callers — the population most affected, since `now()` is
  where sub-millisecond values come from.

## Fix Design

Add a second overload rather than a parameter on the existing one, matching how
the package already overloads `datetime::parse` (two- and three-argument
forms):

```
datetime::toIso(dt AS DateTime) AS String                       ' unchanged, .fff
datetime::toIso(dt AS DateTime, digits AS Integer) AS String    ' 0, 3, 6 or 9
```

`digits = 0` drops the fractional field entirely (RFC 3339 permits its
absence), `3` reproduces today's output, `9` is lossless. Any other value
raises `ErrInvalidArgument`; the restricted set keeps the output aligned with
the `fff`/`ffffff`/`fffffffff` tokens `format` and `parseIso` already handle,
rather than inventing seven new widths that only this member emits.

Then correct the page: state that the one-argument form is millisecond-exact
and that the round trip is lossless only for `digits = 9`.

Rejected: changing the default to nine digits. It shifts every golden and every
downstream parser for a benefit most callers do not need.

Rejected: a `crypto`-style enum (`datetime::IsoPrecision.Millis`). The value is
a digit count and reads better as one; an enum adds a type for four integers.

Rejected: making `toIso` emit the shortest lossless form (trimming trailing
zeros). Variable-width output is exactly what makes a timestamp field hard to
parse and sort as text, which is most of why callers want ISO in the first
place.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Land `spikes/api-review/bug-521-toiso-nanos/` (done).
- [ ] Add a fixture asserting `parseIso(toIso(dt, 9)).time.nanos == dt.time.nanos`
      for `nanos = 123456789`, `1`, `999999999` and `0`. Confirm it fails to
      compile today (no such overload).
- [ ] Confirm `datetime::format(dt, "…​.fffffffff…")` really does emit nine
      digits, and that `parseIso` really does read them. Both are assumed by
      the fix design and neither has been executed.
- [ ] `grep -rn "toIso" src/ examples/ benchmark/` — list in-tree emitters.

Acceptance: the fixture fails; the two capability assumptions are confirmed by
running them, not by reading the page; the emitter list is written down.
Commit: —

### Phase 2 — the fix

- [ ] Add the two-argument `toIso` overload with the `{0,3,6,9}` digit set.
- [ ] Correct `func_to_iso.rs`'s prose: precision per form, and the round-trip
      claim qualified.
- [ ] Add an example showing the lossless form.

Acceptance: the Phase 1 fixture passes; one-argument `toIso` output is
byte-identical to before.
Commit: —

### Phase 3 — regenerate + validation

- [ ] Regenerate the `.ncodesum` goldens the new overload shifts (run the regen
      scripts under **bash**). The default output must not move — any golden
      diff containing a *timestamp* is a bug in this change, not drift.
- [ ] `cargo test --no-fail-fast`; `scripts/test-accept.sh`.
- [ ] `scripts/man-run-examples.sh datetime --run`.
- [ ] Re-run the spike on Linux and Windows.

Acceptance: full suite green; no acceptance golden's timestamp text changed.
Commit: —

## Validation Plan

- Regression test: the Phase 1 round-trip fixture across four `nanos` values.
- Runtime proof: `spikes/api-review/bug-521-toiso-nanos/` reporting `lost = 0`
  for the nine-digit form.
- Doc sync: `func_to_iso.rs`; `func_parse_iso.rs` if its tolerance needs
  stating; `src/docs/spec/**` if it fixes the ISO shape.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Whether the second argument should also select the RFC 9557
  `[America/New_York]` zone suffix once bug-520 lands. **Recommend keeping them
  separate** — precision and zone identity are independent — but design the
  parameter so a later zone-name flag does not need a third overload.

## Summary

Low risk and well bounded: a new overload, no change to the default output, and
the only real hazard is a golden diff on an existing timestamp — which would
mean the default moved and the change is wrong. The two capability assumptions
(nine-digit `format`, nine-digit `parseIso`) are the things to actually run
before building on them.
