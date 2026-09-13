# plan-135-C: `timezones::civil` — wall clock in a named zone, with gap and overlap resolution

Last updated: 2026-09-13
Effort: medium (1h–2h)
Depends on: plan-135-B

This letter adds the member that turns a wall-clock reading in a named zone into a
zoned `datetime::DateTime`:

```
timezones::civil(date AS datetime::Date, time AS datetime::Time, name AS String) AS datetime::DateTime
```

It is the operation bug-520's original case needed: "09:00 on 2026-01-15 and on
2026-07-15 in New York". A 09:00 reading always exists, but twice a year a reading is
skipped (spring forward) or happens twice (fall back). This letter fixes one documented
rule for those two cases: **compatible**. A skipped time moves forward by the length of
the gap. A repeated time resolves to the earlier instant.

Behavioral outcome:
- **Core cases.** `timezones::civil(date(2026,1,15), time(9,0,0,0), "America/New_York")`
  renders `2026-01-15T09:00:00.000-05:00`, and July renders `-04:00`. `2026-03-08 02:30`
  renders `03:30:00.000-04:00`, and `2026-11-01 01:30` renders `01:30:00.000-04:00`.
- **Oracle mode.** `oracle/run.sh '' civil` agrees with Python `zoneinfo`
  (`fold=0`) on every gap and overlap edge of every zone.

Prerequisites: see **plan-135-A § Prerequisites**. No extra rows.

References:

- plan-135-B §§ 4.2–4.4 — `offsetAt`, `toZone` and the oracle this letter extends.
- plan-135-A § 2 — the measured spacing and jump bounds this algorithm relies on, and the
  generator assertions that keep them true.
- RFC 9557 §3.4 and the Temporal "compatible" disambiguation — the named rule.
- `mfb man datetime civil`, `resolve`, `inZone`, `date`, `time` — the builtin members
  used. The 2026-09-13 audit (bug-520) found them correct for Utc and FixedOffset zones.
- bug-519 (`bugs/completed/`) — every datetime input boundary validates its calendar
  fields.

## 1. Goal

- `civil(date, time, name)` returns the `datetime::DateTime` whose instant is the one
  `zoneinfo` produces for `datetime(y,m,d,h,mi,s, tzinfo=ZoneInfo(name), fold=0)`.
  Its `zone` is `toZone(name, thatInstant)`, and its `offset` is that zone's offset.
- A calendar field out of range raises `errorCode::ErrInvalidArgument` (`77050002`),
  exactly as `datetime::date` / `datetime::time` do.
- An unknown name raises `errorCode::ErrNotFound` (`77050004`), via plan-135-B.

### Non-goals

- **No disambiguation parameter.** The rule is fixed at compatible (Open Decisions).
- **No wall-clock arithmetic members** (`addDays` etc.). The README documents the
  composition instead (§ 4.3).
- **No change to `datetime::civil`.**

## 2. Current State

- `timezones::offsetAt` / `toZone` exist (plan-135-B), agreeing with `zoneinfo` on the
  `offsets` corpus.
- **Transition spacing.** Measured on 2026d (plan-135-A § 2):
  - the minimum spacing between consecutive transitions of one zone is **597,600 s**
    (America/Cambridge_Bay);
  - the largest single offset change is **86,400 s** (Kwajalein 1993).
  - plan-135-A's generator asserts spacing > 345,600 s and every |utoff| < 172,800 s, so
    a future release that breaks the window below fails generation rather than
    `civil`.
- **Verified: `zoneinfo` `fold=0` is compatible.** Probe 2026-09-13 with `tzdata`
  2026.4 and `reset_tzpath([])`:
  - the gap `2026-03-08 02:30` New York, `fold=0`, gives offset −05:00 →
    `07:30Z` → `03:30-04:00`, forward;
  - the overlap `2026-11-01 01:30`, `fold=0`, gives offset −04:00 → `05:30Z`, the
    earlier instant.
- **Verified: builtin conversion is exact.** `datetime::civil(d, t, datetime::utc())`
  then `resolve` yields the local-seconds count exactly. The 2026-09-13 audit found
  civil/resolve correct for UTC and fixed offsets on macOS and Linux box 2223.

## 3. Design Overview

One function over plan-135-B's lookup. The only design content is the candidate-offset
window. It is correct *because of* two measured bounds that plan-135-A turns into
generator assertions, so the risk lives in those assertions staying in place, not in
the code.

Rejected:

- **Scanning all transitions for the local time.** It is O(n) per call, needs a second
  decode path, and gives the same answer.
- **Temporal's `earlier`/`later`/`reject` choices now.** The requested surface is
  `civil(date, time, name)`, and a parameter is additive later.
- **Building on `datetime::civil` with a Local zone.** That reads the host's rules
  (owner ruling 1).

## 4. Detailed Design

### 4.1 Algorithm (`src/civil.mfb`)

```
W = 172800
d = datetime::date(date.year, date.month, date.day)            ' raises 77050002 on a bad field
t = datetime::time(time.hour, time.minute, time.second, time.nanos)
L = datetime::resolve(datetime::civil(d, t, datetime::utc())).seconds
oBefore = offsetAt(name, instant(L - W, 0))
oAfter  = offsetAt(name, instant(L + W, 0))
c1 = L - oBefore ; ok1 = offsetAt(name, instant(c1, 0)) = oBefore
c2 = L - oAfter  ; ok2 = oAfter <> oBefore AND offsetAt(name, instant(c2, 0)) = oAfter
chosen = ok1 AND ok2 ? min(c1, c2)    ' overlap: earlier instant
       : ok1 ? c1 : ok2 ? c2
       : c1                            ' gap: pre-transition offset, wall clock moves forward
at = instant(chosen, t.nanos)
RETURN datetime::inZone(at, toZone(name, at))
```

Why the window is sound:

1. **Both candidates fall inside the window.** Every candidate satisfies
   `|c − L| = |utoff| < W`.
2. **The window holds at most one transition.** Transitions are > 2W apart.
3. **The two probes bracket that transition.** `offsetAt(L − W)` is the offset before it
   and `offsetAt(L + W)` the offset after it.
4. **Zero-length changes are harmless.** When a transition changes only the abbreviation
   or `isdst`, `oBefore = oAfter` and the single candidate is used.

### 4.2 Oracle `civil` mode

- **`corpus.py`** adds `jobs/civil.txt` lines `civil <name> Y M D h m s`. For every
  transition used by the `offsets` corpus, with `oPrev` → `oNew`, it emits the wall
  readings `t + oPrev − 1`, `t + oPrev`, `t + oNew − 1` and `t + oNew`, plus the
  midpoint of the gap or overlap. It also adds 09:00 on the 15th of every month
  2026–2030 for every name.
- **`oracle.py`** answers each line with `datetime(…, tzinfo=ZoneInfo(name), fold=0)` as
  `<utcSeconds> <utoff> <abbr> <wall Y-M-D h:m:s after normalisation>`.
- **The probe** answers the same fields from `timezones::civil`.

### 4.3 README

Add a section "A clock reading in a zone":

- the New York example;
- the compatible rule, with the 02:30 and 01:30 examples;
- the rule that a returned `DateTime` carries a fixed offset snapshot, so
  `datetime::addDays` on it keeps that offset across a DST change. To move by calendar
  days *in the zone*, call
  `timezones::civil(datetime::addDays(dt, n).date, dt.time, name)`.

  Before writing that sentence, check that `datetime::addDays` on a fixed-offset
  `DateTime` shifts the date and keeps the time. The bug-520 audit measured
  86,400 s/day for FixedOffset.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work;
> `- [~]` partial; strike moot tasks with evidence; fill `Commit:`. **An unticked box
> means NOT DONE.**

### Phase 1 — `civil`

- [ ] `packages/timezones/src/civil.mfb`: the algorithm of § 4.1.
- [ ] `packages/timezones/src/lib.mfb`: `EXPORT FUNC civil(...)` with a `DOC` block.
      Build its `EXAMPLE` against the `.mfp`.
- [ ] `packages/timezones/src/test_civil.mfb`. Every expected value comes from a pasted
      `zoneinfo` `fold=0` one-liner. Cases:
  - [ ] New York 2026-01-15 09:00 → `-05:00`, and 2026-07-15 09:00 → `-04:00`.
  - [ ] Gap 2026-03-08 02:30 → 03:30 `-04:00`, and the gap edges 02:00:00 / 02:59:59.
  - [ ] Overlap 2026-11-01 01:30 → `-04:00`, the earlier instant; edges 01:00:00 /
        01:59:59.
  - [ ] Australia/Lord_Howe's 30-minute overlap.
  - [ ] Pacific/Apia's skipped 2011-12-30.
  - [ ] Kwajalein's 1993 jump.
  - [ ] Nanos preserved (`time(9,0,0,123456789)`).
  - [ ] `datetime::Date[2026, 2, 30]` literal → traps `77050002`.
  - [ ] Unknown name → `77050004`.
- [ ] README § 4.3.

Acceptance: `civil` gives `zoneinfo`'s `fold=0` answer on each hand case, including both
New York transitions and the 24-hour Apia gap.
  Check: `target/release/mfb test packages/timezones` → `Fail: 0`, with the `civil` group present (est. 1 min).
Commit: —

### Phase 2 — oracle `civil` mode, and proof that it can fail

- [ ] `corpus.py`, `oracle.py` and the probe gain the `civil` mode of § 4.2. `run.sh`
      runs it.
- [ ] Record the `jobs/civil.txt` line count and wall time in the oracle README.
- [ ] Mutation proof: swap the overlap choice to `max(c1, c2)`, run `run.sh '' civil`,
      confirm mismatches > 0, and revert. Then change the gap choice to `c2`, confirm
      mismatches > 0, and revert. Record both counts.

Acceptance: `civil` agrees with `zoneinfo` on every transition edge of every zone, and
each of the two disambiguation branches is shown to be exercised.
  Check: `packages/timezones/oracle/run.sh '' civil; echo EXIT=$?` → `0 mismatches`, `EXIT=0`; the two mutation runs → mismatches > 0 each (est. UNMEASURED until the corpus exists — set it from the line count; the edge set is the only coverage of both branches, so it is not sampled down).
Commit: —

## Validation Plan

- **Tests:** `test_civil.mfb` (Phase 1).
- **Coverage check:** the two mutation runs (Phase 2).
- **Runtime proof:** the oracle probe executable. Cross-target runs are plan-135-D's.
- **Doc sync:** the README section and `DOC` block.
- **Final gate:** plan-135-D § Validation Plan.

## Open Decisions

- **Disambiguation parameter.** Recommend **none now**: compatible is fixed and
  documented. A later `civil(date, time, name, policy)` overload taking
  `"compatible"`/`"earlier"`/`"later"`/`"reject"` is additive, and `reject` is the only
  one that cannot be composed from `offsetAt` today.

## Corrections

## Summary

A small function whose correctness rests on two measured properties of the tzdb:
transitions are more than four days apart, and offsets stay under two days.
plan-135-A's generator enforces both. The oracle's edge corpus and two mutation runs
prove both branches. Nothing outside `packages/timezones/` changes.
