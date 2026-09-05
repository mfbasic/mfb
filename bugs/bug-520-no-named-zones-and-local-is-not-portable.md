# bug-520: `datetime::Zone` has no named zones, and a `Local` zone cannot be serialized portably

Last updated: 2026-09-04 (519/521/518 interaction notes added; still Open, not started)
Effort: huge (>3d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: `tests/` — new `rt_datetime_zone_roundtrip` fixture (Phase 1)

`datetime::ZoneKind` has exactly three variants:

| variant | meaning |
| --- | --- |
| `Utc` | offset 0 |
| `FixedOffset` | a constant offset, built with `datetime::fixedOffset` |
| `Local` | "the host system's local time zone" |

There is no way to say `America/New_York`. That has two consequences, and the
second is the sharper one:

1. **No future-date arithmetic across a DST boundary is expressible.** A
   `FixedOffset` zone is constant by construction, so scheduling "09:00 every
   weekday in New York" cannot be written: the offset is −05:00 for part of the
   year and −04:00 for the rest, and picking either one is wrong for half the
   calendar. `datetime::local` resolves the *host's* rules at the moment of the
   call, which answers the question only for a host that happens to be in the
   target zone.

2. **A `Local` zone does not survive leaving the host.** `datetime::Zone` is an
   ordinary record — `offsetSeconds`, `kind`, `label` — so it is
   thread-sendable, storable and printable. But `kind = Local` carries no
   identity: it says "wherever this program is", and the `offsetSeconds`
   snapshot beside it was resolved against *this* host at *that* moment. Write
   it to a file, send it to a peer, or hold it across a DST transition, and it
   silently means something else.

The single correct behavior a fix produces: a `DateTime` that names a real
civil zone can be written down and read back — on another host, at another time
of year — and still name the same civil zone.

References:

- `mfb man datetime types` — the `Zone` record and `ZoneKind` enum
- `src/codegen/builtins/datetime/func_local.rs`, `func_local_offset.rs`,
  `func_fixed_offset.rs`, `func_offset_at.rs`
- IANA tzdb; RFC 9557 (IXDTF, the `[America/New_York]` timestamp suffix)

## Failing Reproduction

The gap is structural, so the reproduction is a program that cannot be written.

```
./target/release/mfb man datetime types
```

- Observed: `ZoneKind` is `Utc | FixedOffset | Local`. No constructor takes a
  zone name; `grep -rn "IANA\|tzdb\|America/" src/codegen/builtins/datetime/`
  returns nothing.

- Expected: a way to name a zone — `datetime::zone("America/New_York")` — whose
  offset is resolved per-instant, and which round-trips through serialization.

The concrete failing case, to be written as a fixture in Phase 1:

```
' Both of these are 09:00 in New York. There is no Zone value that produces
' both, because the correct offset differs (-05:00 in January, -04:00 in July).
LET winter = <09:00 on 2026-01-15 in America/New_York>   ' needs -05:00
LET summer = <09:00 on 2026-07-15 in America/New_York>   ' needs -04:00
```

Contrast cases that work correctly today and bound the scope:

- `datetime::fixedOffset(5, 30)` is exact and portable. For a zone that has
  never observed DST, `FixedOffset` is the right answer and must stay.
- `datetime::local()` + `datetime::localOffset` correctly resolve the host's
  DST rules *for a given instant* — `mfb man datetime withZone` confirms
  "the DST-correct host offset for a local zone". The machinery to apply a
  rule set per-instant already exists; what is missing is the ability to name a
  rule set other than the host's.
- `datetime::toIso` emits a numeric offset, which is unambiguous for the
  instant it names. Nothing about *instants* is broken here; the gap is in
  naming *zones*.

## Root Cause

Not a defect — a deliberate v1 scope boundary that has become load-bearing.
The package models a zone as `(offsetSeconds, kind, label)`: a *resolved
offset* plus a tag. Two of the three tags describe how the offset was obtained
(`Utc`: it is zero; `FixedOffset`: it was given). The third, `Local`, describes
a *lookup* — and the record has nowhere to store which lookup.

Because `label` is documented as "A human-readable label for the zone (e.g.
"UTC" or "+05:30")" it is prose, not identity: nothing reads it back. So a
`Zone` value is self-describing for `Utc` and `FixedOffset` and not
self-describing for `Local`.

Adding named zones means adding a rule *source*, which is the real cost: either
the host's tzdb (present on macOS and Linux, absent on Windows, and differing
by version between hosts — which breaks the package's byte-identity contract)
or a vendored tzdb (correct and reproducible, but a data set that expires and
must be updated).

## Goal

- `datetime::zone(name AS String) AS datetime::Zone` accepts an IANA zone name
  and raises on an unknown one.
- The resulting `Zone` resolves its offset **per instant**, so
  `withZone(dt, zone("America/New_York"))` is correct on both sides of a DST
  transition.
- A `Zone` can be serialized and restored without losing its identity — round
  trip through a String and back yields a `Zone` that resolves identically.
- `datetime::local()` continues to work, and gains a documented statement that
  a `Local` zone is host-scoped and not portable.

### Non-goals (must NOT change)

- `datetime::fixedOffset`, `datetime::utc`, and the `Utc`/`FixedOffset`
  variants. They are correct, cheap, and the right answer for a fixed offset.
- `datetime::Instant`, `resolve`, `toIso`, `toMillis` — instants are already
  unambiguous and must not acquire a zone.
- The cross-platform byte-identity contract. Whatever rule source is chosen
  must give the same answer on macOS, Linux and Windows for the same
  `(zone, instant)` pair, or the package's central guarantee is broken.
- **Tempting wrong fix, forbidden:** storing an IANA name in the existing
  `label` field and resolving it lazily. `label` is documented as
  human-readable prose; overloading it makes an unvalidated string
  load-bearing, and every existing `Zone` constructed with a decorative label
  becomes a potential lookup failure.

## Blast Radius

- `src/codegen/builtins/datetime/func_local.rs`, `func_local_offset.rs` — the
  existing per-instant resolution path; the named-zone lookup should reuse its
  shape.
- `func_offset_at.rs` — already resolves an offset for a zone at an instant.
  This is the seam a named zone plugs into; check in Phase 1 whether it is
  general enough.
- `func_with_zone.rs`, `func_in_zone.rs`, `func_to_local.rs`, `func_to_utc.rs`
  — every consumer of a `Zone`. All must keep working for the three existing
  kinds.
- `func_format.rs`, `func_to_iso.rs` — emit offsets, not names. A named zone
  raises the question of whether `toIso` should gain the RFC 9557
  `[America/New_York]` suffix; that is a separate decision (see Open Decisions).
- `datetime::Zone` as a thread-sendable record — `Zone` crosses threads today
  as a plain record. A named zone must not become a resource, or every
  `DateTime` stops being thread-sendable. This is the hard constraint on the
  design.
- `src/docs/spec/**` — the zone model is language surface and will need a spec
  section.

## Fix Design

Three sub-decisions, in dependency order. This is large enough that it should
become a `plan-NN` rather than being executed straight from this document; the
bug records the defect and the constraints.

**1. Rule source.** Vendored tzdb (reproducible, byte-identical, expires) vs.
host tzdb (free, absent on Windows, version-skewed between hosts). The
package's byte-identity contract effectively forces **vendored** — the same
choice already made for the Unicode tables, and the same maintenance
consequence. Note the existing precedent and its known hazard: the pinned
Unicode tables already disagree with a newer external source, and a tzdb
disagrees far faster.

**2. Representation.** A `Zone` must stay a plain, thread-sendable, copyable
record. That argues for `kind = Named` plus the IANA name in a **new** field
(not `label`), with the rule table looked up by name at each resolution.
Storing an index into a vendored table would be cheaper but makes the value
meaningless across a table version bump — exactly the serialization failure
this bug is about.

**3. Serialization.** Once a zone has identity, `toIso` truncating to a numeric
offset becomes a real loss. RFC 9557's `[America/New_York]` suffix is the
standard answer and composes with bug-521's nanosecond question — both are
"`toIso` cannot represent everything a `DateTime` holds".

Rejected: a `datetime::zoneOffsetAt(name, instant)` free function with no
`Zone` value. It answers question 1 (DST-correct arithmetic) and not question 2
(a `DateTime` that remembers its zone), and it puts the burden on every caller
to carry the name alongside every value.

Rejected: shipping only "serialize a Local zone as its resolved offset". That
is what already effectively happens, and it is what makes the value wrong on
the other side of a DST transition.

## Phases

### Phase 1 — establish the failure, and decide the rule source

- [ ] Write the fixture that cannot be satisfied today: 09:00 in
      `America/New_York` on 2026-01-15 and 2026-07-15, asserting offsets
      −05:00 and −04:00. Confirm no `Zone` value produces both.
- [ ] Write the serialization fixture: a `Local` zone, printed and restored,
      asserted to resolve identically on a host in a different zone
      (`TZ=` override is enough to demonstrate it).
- [ ] Decide the rule source (vendored vs. host) and record the decision, with
      the byte-identity argument, in this file.
- [ ] Audit `func_offset_at.rs` for whether it generalizes to a named zone.

Acceptance: both fixtures fail for the documented reason; the rule-source
decision is written down with its consequences.
Commit: —

### Phase 2 — promote to a plan

- [ ] This is feature work spanning a vendored data set, a new enum variant, a
      new record field, and a spec section. Write `plan-NN` from the Phase 1
      findings and execute there.

Acceptance: a plan document exists with the rule source fixed and the
representation decided.
Commit: —

## Validation Plan

- Regression tests: the two Phase 1 fixtures — DST-correct offsets on both
  sides of a transition, and a zone that survives serialization.
- Runtime proof: the same `(zone, instant)` pair resolving identically on
  macOS, Linux and Windows.
- Doc sync: `mfb man datetime types` (`Zone`, `ZoneKind`), a new
  `datetime::zone` page, `func_local.rs`'s portability statement, and a
  `src/docs/spec/**` section on the zone model.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Vendored tzdb vs. host tzdb. **Recommend vendored**, for byte-identity and
  for Windows, accepting the update burden. Record the tzdb version the way the
  Unicode tables are pinned.
- Whether `toIso` gains the RFC 9557 `[Zone/Name]` suffix. **Recommend a
  separate `toIso` overload** rather than changing the existing one, and
  resolving it together with bug-521's nanosecond overload — they are the same
  question asked twice.
  **Resolved on the 521 side (2026-09-04, `6ab026ea4`), and NOT the same
  question after all.** 521 landed `datetime::toIso(dt, digits AS Integer)` with
  `digits` in `{0, 3, 6, 9}`. Precision and zone identity turned out to be
  independent axes: `digits` names a fractional width and nothing else, so a
  later RFC 9557 zone-name flag must arrive as its own parameter or its own
  member, **not** by overloading that integer. Two constraints this bug now
  inherits:
  1. The arity slots `toIso(DateTime)` and `toIso(DateTime, Integer)` are both
     taken. A zone-suffix form needs a third shape — `toIso(dt, digits,
     zoneName AS Boolean)` is the cheap one, and it keeps `digits` meaning what
     it means today.
  2. `toIso`'s page now states, and
     `tests/rt-behavior/datetime/datetime-iso-nanos-rt` pins, that
     `parseIso(toIso(dt, 9))` recovers `dt`'s `nanos` exactly. Whatever this bug
     does to the ISO writer must keep that round trip true, and a `[Zone/Name]`
     suffix means `parseIso` has to learn to skip or read it.
- Whether `Local` should be *deprecated* in favour of resolving the host's zone
  to a name at `datetime::local()` time. **Recommend yes if the host lookup is
  reliable** — it turns the one non-portable value into a portable one — but
  it depends on the rule source, so it belongs in the plan.

### Interaction with the landed datetime fixes (2026-09-04)

Three sibling bugs landed while this one stayed open. None of them starts it,
and none of them is blocked by it, but each leaves a constraint here:

- **bug-519** (`250df247e`) — `parse` and `parseIso` now range-check their
  decoded *calendar* fields via `__datetime_checkFields`. The bound is on
  calendar fields only and says nothing about zone identity, so a named-zone
  `Zone` does not interact with it. What it does establish is a rule to keep:
  the package now has exactly **three input boundaries** (`date`, `time`, and
  the two parse readers) and all three enforce the same ranges, which is why
  `civil` is safe to leave trusting its arguments. A named-zone constructor
  (`datetime::zone("America/New_York")`) becomes a **fourth** boundary and owes
  the same treatment: an unknown zone name must raise, not fall back to UTC or
  to `Local`.
- **bug-521** (`6ab026ea4`) — see the `toIso` Open Decision above.
- **bug-518** (this pass) — `withZone`'s `zone` parameter row now states that
  the instant is preserved and names `datetime::civil(dt.date, dt.time, zone)`
  as the operation that keeps the wall-clock reading instead. Both sentences
  are about `Zone` *values*, so if this bug adds a `Named` `ZoneKind` they must
  stay true for it: `withZone` into a named zone must still preserve the
  instant, and `civil` into a named zone must resolve that zone's own DST rules
  rather than the host's. `tests/rt-behavior/datetime/datetime-withzone-instant-rt`
  pins the first of those across every existing `ZoneKind`; a `Named` variant
  should be added to that fixture's list rather than getting a new one.

## Summary

This is the largest item in the review and the one most likely to be deferred:
the defect is real (a `Local` zone silently changes meaning when it leaves the
host, and DST-correct future scheduling is inexpressible), but the fix drags in
a vendored, expiring data set and a public record change. Phase 1 is cheap and
worth doing regardless — it turns "we know this is missing" into two failing
fixtures and a written rule-source decision.
