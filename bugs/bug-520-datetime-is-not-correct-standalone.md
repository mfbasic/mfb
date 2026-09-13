# bug-520: `datetime` is not correct on its own — offset text is lost or laundered, and five zone-page claims contradict the code

Last updated: 2026-09-13 (re-scoped by owner ruling; standalone audit run; bug-603 merged in as S1–S3; owner decided S1 = write seconds, S6 = keep the offset)
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: new `tests/rt-behavior/datetime/` fixtures per sub-issue (Phase 1), plus a `TZ`-pinned host-zone fixture (S10)

## Scope: owner ruling (2026-09-13)

This bug first asked for named IANA zones in `datetime`. The owner re-scoped it:

1. **`datetime` must work correctly on its own, without a zone database.** Its zone model
   is `Utc`, `FixedOffset` and the host's `Local`. If that model is not correct, that is
   this bug. If it is correct, the bug closes.
2. **Named zones are a feature, not a `datetime` defect.** They are built as a separate
   source package, `packages/timezones`, over vendored IANA data. That work is
   **plan-135** (`planning/plan-135-A-…` through `-D-…`), and nothing about it belongs
   here.

A standalone audit ran the same day. **Host-zone resolution is correct.** The text path
for offsets is not, and five doc claims contradict behavior. So the bug stays open and
now records those defects.

### What the audit found correct

It compared against Python `zoneinfo` on macOS aarch64 and Linux aarch64 glibc (box
2223), using `--target linux-aarch64` binaries run on the box:

- **Host-zone members match `zoneinfo`.** `offsetAt`, `localOffset`, `toLocal`,
  `inZone`, `civil` and `resolve` agreed on every line across 7 zones:
  - zones: America/New_York, Europe/London, Australia/Lord_Howe (30-min DST),
    Asia/Kolkata, Pacific/Chatham, Pacific/Apia, America/Santiago;
  - 19 instants: January/July, 1850/1883 LMT, 1900, 1960, 2040, 2100, 2500, and each side
    of a transition;
  - 20 civil readings, including gaps, overlaps, Apia's skipped day and Santiago's
    skipped midnight.
- **Wall-clock arithmetic re-resolves correctly with a `Local` zone.**
  - `addDays` across spring-forward gives an 82,800 s day, and across fall-back 90,000 s.
  - `addDays` into a gap gives 03:30 −04:00.
  - `startOfDay` is right, including Santiago's midnight gap.
  - `FixedOffset`/`Utc` keep 86,400 s per day.
- **`withZone` preserves the instant** for every pair of the three kinds.
- **Whole-minute text round-trips.** `toIso(dt, 9)`/`parseIso` and `format`/`parse` with
  `ZZZ` return the same second and nanos.

### What is not a defect (the old bug-520 content, retired)

- **"No named zones."** This is a feature, now plan-135.
- **"A `Local` zone does not survive leaving the host."** This is documented behavior.
  The pages state it:
  - `mfb man datetime types`, `ZoneKind`: "Local — The host system's local time zone."
  - the `local` page: two hosts in different configured time zones project the same
    instant to different fields;
  - the `toLocal` page: "the same instant can produce a different civil
    datetime::DateTime on a host configured for a different zone";
  - the `startOfDay` page: "the same dt can yield a different absolute instant on a host
    configured for a different zone or DST rule".

  The old write-up also described the mechanism wrongly. `datetime::local()` stores
  `offsetSeconds = 0` as a placeholder (probe below: `local() 0 2 Local`), not a snapshot.
  The only cached value is `DateTime.offset`. `resolve` and `withZone` use it, so a
  `DateTime`'s instant survives a host move; only wall-clock arithmetic re-resolves
  against the new host. That is S7's doc problem, not a model problem.

## Failing Reproduction

All probes: `target/release/mfb` at HEAD `edb782afe`, a throwaway `mfb init` project in
`/tmp`, macOS aarch64, 2026-09-13. `TZ` is set on the *run* where shown. Errors were read
with a function-level `TRAP(e)` that returns `toString(e.code)`.

### S1 — an offset's seconds are dropped on write *(was bug-603)*

```
' TZ=America/New_York
LET old AS datetime::DateTime = datetime::toLocal(datetime::instant(-3771144000, 0))
io::print(toString(old.offset) & " " & datetime::toIso(old))
io::print(toString(datetime::resolve(datetime::parseIso(datetime::toIso(old))).seconds))
```

- Observed: `-17762 1850-07-01T07:03:58.000-04:56`, then `-3771144002`, which is 2 s from
  the value.
- Also reachable with a hand-built zone: `datetime::Zone[-17762, 1, "LMT"]` at 1880-01-01
  09:00 renders `-04:56` and reads back as `-2840090640` instead of `-2840090638`.
- Expected: text never names a different instant from its `DateTime`.
- Source: `helper_offset_label_sep.rs:__datetime_offsetLabelSep` formats `hh` and `mm`
  only. It feeds `toIso` (both arities, via `helper_iso_zone.rs:__datetime_isoZone`) and
  `format`'s `Z`/`ZZ`/`ZZZ` (`helper_format_token.rs`). Census:
  `grep -rn "__datetime_offsetLabel\(Compact\|Sep\)\?(\|__datetime_isoZone(" src/codegen/builtins/datetime/`.

### S2 — an offset's `:SS` is read as leftover text and ignored *(was bug-603)*

```
datetime::parseIso("1880-01-01T09:00:00-04:56:02")   ' resolves to -2840090640
```

- Observed: `-2840090640`, the instant for `-04:56`, with no error.
- Expected: `-2840090638`, or `ErrInvalidFormat`; never a third instant.
- Source: `helper_read_offset.rs:__datetime_readOffset` stops after the minutes.

### S3 — both readers accept trailing text *(was bug-603)*

```
datetime::parseIso("2026-01-01T00:00:00Zgarbage")                    ' → 2026-01-01T00:00:00.000Z
datetime::parse("2026-01-01 junk", "yyyy-MM-dd", datetime::utc())    ' → 2026-01-01T00:00:00.000Z
```

- Expected: `ErrInvalidFormat` (`77050003`), the spec's code for every structural
  mismatch (`src/docs/spec/stdlib/02_datetime.md` § "Parse grammar").
- Source:
  - `func_parse_iso.rs:__datetime_parseIso` returns right after `__datetime_readOffset`
    without checking for end of input.
  - `helper_parse_fields.rs:__datetime_parseFields` returns its final position, and
    `helper_build_from_fields.rs:__datetime_buildFromFields` never compares it with
    `len(value)` (`grep -n "len(" …/helper_build_from_fields.rs` → empty).

### S4 — impossible offset minutes and 1-digit offset fields are accepted

```
datetime::parseIso("2026-01-01T00:00:00+05:75")   ' offset 22500 (+06:15)
datetime::parseIso("2026-01-01T00:00:00+05:60")   ' offset 21600 (+06:00)
datetime::parseIso("2026-01-01T00:00:00+5:30")    ' offset 19800
datetime::parse("2026-01-01 00:00 +05:75", "yyyy-MM-dd HH:mm Z")   ' offset 22500
```

- Observed: each returns a value with no error. The audit also saw
  `parseIso("…+05:3")` → `+05:03`.
- Expected: `ErrInvalidFormat`. The documented grammar is `±HH:MM`/`±HHMM`, and RFC 3339
  `time-minute` is `00`–`59`. Spec § "Decoded fields are range-checked" (bug-519) exists
  so that text is never laundered into a valid-looking value; this is the same failure
  for the offset field.
- Source: `__datetime_readOffset` reads `mm` with no range check, and `__datetime_readNum`
  accepts fewer digits than the field width
  (`grep -n "hh.value \* 3600 + mm.value" src/codegen/builtins/datetime/helper_read_offset.rs`).

### S5 — an offset of 24 h or more raises the argument code, not the format code

```
datetime::parseIso("2026-01-01T00:00:00+24:00")                     ' error 77050002
datetime::parse("2026-01-01 00:00 +24:00", "yyyy-MM-dd HH:mm Z")    ' error 77050002
```

- Expected: `77050003`. Spec § "Decoded fields are range-checked": "fails
  `ErrInvalidFormat` … not the constructors' `ErrInvalidArgument`, because the argument
  is a well-formed `String` and it is the *text* that is malformed." The `parse` page:
  "one TRAP catches every flavour of bad text".
- Source: both readers pass the parsed offset to `__datetime_fixedOffset1`, which raises
  `77050002`
  (`grep -n "__datetime_fixedOffset1(" src/codegen/builtins/datetime/helper_build_from_fields.rs src/codegen/builtins/datetime/func_parse_iso.rs`).

### S6 — adding zero days changes the instant of a repeated hour

```
' TZ=America/New_York
LET a5 AS datetime::DateTime = datetime::toLocal(datetime::instant(1793514600, 0))   ' 01:30 -05:00, the SECOND 01:30
LET b5 AS datetime::DateTime = datetime::addDays(a5, 0)
```

- Observed: `a5` = `2026-11-01T01:30:00.000-05:00` at `1793514600`, and `b5` =
  `2026-11-01T01:30:00.000-04:00` at `1793511000`. The instant moved −3600 s.
- The audit saw the same from `addMonths(a5, 0)`. Phase 1 re-verifies it.
- Expected, per the `addDays` page: "Adding zero days returns a `datetime::DateTime`
  equal to `dt`" (`grep -n "Adding zero days" src/codegen/builtins/datetime/func_add_days.rs`).
- Source: `func_add_days.rs:__datetime_addDays` and `func_add_months.rs` re-resolve the
  wall clock through `__datetime_civil`, and `__datetime_resolveLocal` always picks the
  earlier offset in an overlap. Python's `dt + timedelta(0)` also resets `fold`. Which
  side is wrong is Open Decision 2.

### S7 — "pure" is false for a `Local` zone

```
LET st AS datetime::DateTime = datetime::DateTime[datetime::date(2026, 7, 15), datetime::time(9, 0, 0, 0), datetime::local(), -14400]
io::print(toString(datetime::resolve(datetime::addDays(st, 0)).seconds))
```

- Observed: `1784120400` under `TZ=America/New_York`, and `1784102400` under
  `TZ=Europe/London`.
- Doc: "`addDays` is pure: the same `datetime::DateTime` and day count always yield the
  same result", and the matching `addMonths` sentence
  (`grep -n "is pure" src/codegen/builtins/datetime/func_add_days.rs src/codegen/builtins/datetime/func_add_months.rs`).
  The `startOfDay` page states the host dependence correctly and is the model.

### S8 — pages omit errors the members raise

Every one of these descriptors declares `errors: vec![]`: `func_local_offset.rs`,
`func_offset_at.rs`, `func_fixed_offset.rs` (both arities), `func_civil.rs`,
`func_to_local.rs` and `func_in_zone.rs`
(`grep -c 'errors: vec!\[\]' <file>` → 1, 1, 2, 1, 1, 1). But:

```
datetime::fixedOffset(0, -30)     ' error 77050002
datetime::fixedOffset(5, 60)      ' error 77050002
datetime::fixedOffset(24, 0)      ' error 77050002
datetime::fixedOffset(86400)      ' error 77050002
datetime::localOffset(9223372036854775807)   ' error 77050002
```

The audit additionally reported `fixedOffset(9223372036854775807, 0)` → `77050010`, and
that `toLocal` raises `ErrInvalidArgument` for an out-of-range instant (the spec says
so; `func_datetime_localOffset_valid` pins it). Which inputs make `offsetAt`, `inZone`
and `civil` raise is **UNVERIFIED**. Phase 1 establishes it before any descriptor is
edited, so each page lists exactly what that member raises.

### S9 — `fixedOffset(hours, mins)`'s formula is wrong at `hours = 0`

```
datetime::fixedOffset(0, 30)      ' 1800 "+00:30"
datetime::fixedOffset(0, -30)     ' error 77050002
datetime::fixedOffset(-1800)      ' -1800 "-00:30"
```

- The page gives `sign(hours) * (abs(hours) * 3600 + mins * 60)`
  (`grep -n "sign(hours)" src/codegen/builtins/datetime/func_fixed_offset.rs`). That is
  0 for `(0, 30)`, yet the call returns `1800`.
- A `−00:30` zone can only be built with the one-argument form. The page does not say
  so.

### S10 — nothing pins host-zone DST behavior

- No fixture sets `TZ` (`grep -rln "\bTZ=" tests` → nothing, 2026-09-13).
- The existing host-zone fixtures assert only host-neutral properties:
  `datetime-clock-offset`, `func_datetime_localOffset_valid`,
  `datetime-withzone-instant-rt`.
- So the correct DST resolution the audit measured, and the gap/overlap choice S6 turns
  on, are unprotected.

### Not reproduced, recorded for completeness

- **Windows host-zone path.** `SystemTimeToTzSpecificLocalTime(NULL, …)` ignores `TZ`.
  The audit's reading of the Microsoft documentation suggests it may apply only the
  current year's DST rules to historical instants. That is a **guess**: no Windows binary
  can be executed here (no harness on box 2230). It is not a sub-issue until it is run.
- **`TZ=Nowhere/Bogus` silently behaves as UTC.** This matches libc (`TZ=Nowhere/Bogus
  date`) and is host behavior. It is not a sub-issue; Open Decision 4 covers whether the
  `local` page should say so.

## Root Cause

- **S1–S5.** The offset text path was written for offsets that are whole minutes and
  well-formed. `__datetime_offsetLabelSep` has no seconds term. `__datetime_readOffset`
  neither range-checks nor requires two digits, and it returns a position nobody checks.
  Its out-of-range offsets reach the constructor-side validator, which uses the
  constructor's error code. Local-zone history (LMT) produces sub-minute offsets, so S1
  is reachable from ordinary host-zone use.
- **S6–S9.** Page prose was written for the fixed-offset case and not re-read against the
  `Local` path, which re-resolves.
- **S10.** Host-zone tests avoid `TZ` so they pass on any host, and in doing so test
  nothing zone-specific.

## Goal

- No `datetime` writer emits text that names a different instant from its `DateTime`.
- `parseIso` and `parse` reject any offset text that is not a well-formed offset (two
  digits per field, minutes and seconds in 0..59, magnitude under 24 h). They reject any
  text left over after the grammar or pattern, always with `ErrInvalidFormat`.
- Every zone-related page states what the member does, including the `Local`
  dependence and the errors it raises.
- Host-zone DST resolution, gaps, overlaps and re-resolving arithmetic are pinned by
  fixtures with `TZ` set.

### Non-goals (must NOT change)

- **No zone database in `datetime`, and no named zones** (owner ruling; plan-135).
- **The output of any whole-minute offset**, which is every existing golden and example
  that shows an offset.
- **`toIso`'s arity split and `digits` contract** (bug-521), and
  `parseIso(toIso(dt, 9))` recovering `nanos`
  (`tests/rt-behavior/datetime/datetime-iso-nanos-rt`).
- **bug-519's range checks and their `77050003` code.**
- **`Local` semantics**: host-scoped, re-resolving per instant.
- **`withZone` preserving the instant** (bug-518,
  `tests/rt-behavior/datetime/datetime-withzone-instant-rt`).

## Blast Radius

- **Offset text path:** `helper_offset_label_sep.rs`, `helper_offset_label.rs`,
  `helper_offset_label_compact.rs`, `helper_iso_zone.rs`, `helper_format_token.rs`,
  `func_to_iso.rs`, `helper_read_offset.rs`, `helper_read_num.rs` (if S4 is fixed there),
  `func_parse_iso.rs`, `helper_parse_fields.rs`, `helper_build_from_fields.rs` and
  `func_parse.rs`.
- **Arithmetic:** `func_add_days.rs`, `func_add_months.rs`, and `helper_resolve_local.rs`
  if Open Decision 2 picks the behavior fix.
- **Docs:** the descriptors of `add_days`, `add_months`, `fixed_offset`, `local_offset`,
  `offset_at`, `civil`, `to_local` and `in_zone`, plus `src/docs/spec/stdlib/02_datetime.md`
  § "Parse grammar" and the `format` token table.
- **Callers relying on prefix matching (S3):** census before landing with
  `grep -rn "datetime::parse(\|datetime::parseIso(" tests/ examples/ packages/ src/docs/`.
  A fixture that depends on trailing text being ignored is pinning this bug.
- **Goldens:** whole-minute outputs must not change (non-goal). Any golden diff outside a
  new fixture is a bug-hunt trigger.
- **plan-135-D** is gated on this bug closing with the writer printing seconds (plan-135-A
  § Prerequisites).

## Fix Design

- **S1/S2 — writer and reader agree on seconds.** When `offset MOD 60 <> 0`, emit
  `±HH:MM:SS` (and `ZZZ`+ `±HHMMSS`). `__datetime_readOffset` accepts an optional `[:]SS`
  after the minutes. Whole-minute output is byte-identical. Measured 2026-09-13: Node 24
  `Temporal.ZonedDateTime.from("1850-07-01T07:03:58-04:56:02[America/New_York]")`
  accepts the form, and RFC 3339 alone does not. See Open Decision 1.
- **S3.** After the last token, `parseIso` and `parse` fail `77050003` unless the
  position equals `len(value)`.
- **S4.** `__datetime_readOffset` requires exactly two digits for `HH`, `MM` and `SS`,
  with `MM`/`SS` ≤ 59, else `77050003`.
- **S5.** The readers check `|offset| < 86400` before building the zone, and fail
  `77050003`.
- **S6.** Keep `dt.offset` when it is still valid for the resulting wall clock (owner
  decision 2026-09-13), so `addDays(dt, 0)` and `addMonths(dt, 0)` return `dt`. Only
  when the offset is not valid there does the wall clock re-resolve, with the existing
  earlier-offset rule.
- **S7.** Replace "pure" with the `startOfDay` page's wording: the result depends on the
  host's rules for a `Local` zone, and is fixed for `Utc` and `FixedOffset`.
- **S8.** After Phase 1 establishes each member's raising inputs, list them in its
  descriptor `errors`.
- **S9.** State the real rule: `hours` carries the sign, `mins` is 0..59 and never
  negative, and `hours = 0` is positive. A negative sub-hour offset needs
  `fixedOffset(seconds)`. See Open Decision 3.
- **S10.** Add `rt-behavior` fixtures that run with `TZ=America/New_York`. Whether the
  harness can set a fixture's environment is **UNVERIFIED** (Phase 1).

## Phase 1 findings (2026-09-13)

**Harness `TZ` answer.** `scripts/test-accept.sh` ran a fixture binary with no way to
set its environment (`run_with_watchdog "$run_path"`). Added: an optional `run.env`
beside `project.json` (`NAME=value` lines) applied to the run only, through `env`, so
`argv[0]` and the logged `$ <exe>` line do not change. `scripts/linux-runtime-proof.sh`
applies it too. `datetime-tz-new-york-rt/run.env` is `TZ=America/New_York`.

**RED at HEAD** (`bash scripts/test-accept.sh target/release/mfb /tmp/b520-actual <name>`,
`f011d27b9`, macOS aarch64):

| Fixture | Line at HEAD | Sub-issue |
| --- | --- | --- |
| `datetime-offset-seconds-rt` | `toIso=1880-01-01T09:00:00.000-04:56`, `isoRoundTrip=FALSE`, `isoSep=-2840090640` | S1, S2 |
| `datetime-parse-strict-rt` | `isoTrailZ=ACCEPTED`, `patTrail=ACCEPTED`, `isoMin75=ACCEPTED … off=22500`, `isoHour1=ACCEPTED`, `isoPlus24=77050002` | S3, S4, S5 |
| `datetime-tz-new-york-rt` | `addDays0.second=…-04:00 @1793511000`, `addMonths0.second=…-04:00`, `lmt.roundTrip=-3771144002` | S1, S6 |

S6 re-verified: `addMonths(a5, 0)` moved the instant too (`addMonths0.second` above).
S7 and S9 are page text; no fixture can show S7, because it needs two `TZ` values.

**S8 error table** (probe `/tmp/b520-s8`, `TZ=America/New_York`; each line is a call
and its trapped code):

| Member | Raises | Inputs |
| --- | --- | --- |
| `localOffset` | `ErrInvalidArgument` | `9223372036854775807`, `-9223372036854775808`, `10^17` (`10^15` is fine) |
| `offsetAt` | `ErrInvalidArgument` | Local zone with the same out-of-range seconds; a fixed zone never raises |
| `toLocal` | `ErrInvalidArgument` | same out-of-range seconds |
| `inZone` | `ErrInvalidArgument`, `ErrOverflow` | Local zone out of range → 77050002; `fixedOffset(3600)` at `Integer` max, or `Zone[max, 1, "x"]` → 77050010 |
| `civil` | `ErrInvalidArgument`, `ErrOverflow` | Local zone, year `±3·10^9` → 77050002; UTC year `3·10^14` → 77050010 |
| `fixedOffset(s)` | `ErrInvalidArgument` | `±86400`, `Integer` max |
| `fixedOffset(h, m)` | `ErrInvalidArgument`, `ErrOverflow` | `(0, -30)`, `(5, 60)`, `(24, 0)`, `(-5, -30)` → 77050002; `(max, 0)`, `(min, 0)` → 77050010 |

The audit found more than S8 named: **every** datetime descriptor declared
`errors: vec![]`, including `date`, `time`, `toIso(dt, digits)`, `parse` and `parseIso`,
which `FAIL` explicitly (`grep -c 'errors: vec!\[\]' src/codegen/builtins/datetime/func_*.rs`).
Those five now list their codes too. The arithmetic members that can only overflow
(`add`, `addDays`, `addMonths`, `plus`, `between`, `toMillis`, …) are not probed and
still list none; see bug-611.

**S3 census.** `grep -rn "datetime::parse(\|datetime::parseIso(" tests/ examples/ packages/ src/docs/`:
no caller relies on trailing text being ignored. Every existing datetime fixture's
`build.log` is byte-identical after the fix.

## Phases

### Phase 1 — RED fixtures and the missing facts

- [ ] One `tests/rt-behavior/datetime/` fixture per sub-issue, S1–S7 and S9, each failing
      at HEAD for the documented reason. A new rt fixture needs its four goldens.
- [ ] Find whether the rt-behavior harness can set `TZ` for a fixture. If not, S1/S6/S7/S10
      need the harness change named here before their fixtures can exist.
- [ ] Re-verify `addMonths(a5, 0)` (S6).
- [ ] For `offsetAt`, `inZone`, `civil`, `toLocal`, `localOffset` and `fixedOffset`,
      establish the complete set of raising inputs and codes by reading each body and
      probing (S8).
- [ ] Census the prefix-matching callers (S3).

Acceptance: every fixture fails at HEAD for its stated reason, and the S8 error table
and the harness `TZ` answer are written into this file.
Commit: —

### Phase 2 — readers (S2–S5)

- [ ] `__datetime_readOffset`: two digits per field, ranges, optional seconds.
- [ ] End-of-input check in `parseIso` and `parse`.
- [ ] An offset magnitude ≥ 24 h fails `77050003` in both readers.

Acceptance: the S2–S5 fixtures pass, and every existing datetime rt fixture is unchanged.
Commit: —

### Phase 3 — writer (S1)

- [ ] `__datetime_offsetLabelSep` emits seconds when non-zero.

Acceptance: the S1 fixture round-trips the 1850 New York instant exactly, and
whole-minute goldens are unchanged.
Commit: —

### Phase 4 — S6: keep the offset when it is still valid

Acceptance: the S6 fixture passes under the decided semantics, and the page states them.
Commit: —

### Phase 5 — docs (S7–S9) and host-zone pins (S10)

- [ ] Descriptor prose and `errors` for the S7–S9 members, plus the spec
      § "Parse grammar" and `format` token table (seconds offsets, whole-string rule).
- [ ] `TZ`-pinned fixtures: New York gap/overlap `civil`, `addDays` across both
      transitions, and `startOfDay` on 2026-03-08.

Acceptance: every edited page renders and its examples run, and the host-zone fixtures
pass.
Commit: —

## Validation Plan

- **Regression tests:** the Phase 1 fixtures, each with its four goldens.
- **Doc sync:** `mfb man datetime toIso|parseIso|parse|format|addDays|addMonths|fixedOffset|localOffset|offsetAt|civil|toLocal|inZone`,
  `scripts/man-run-examples.sh datetime --run`, `scripts/man-census.sh --memory-scope`
  (0 unclassified), and the spec section.
- **Runtime proof:** the S1 and S10 fixtures on Linux box 2223 as well as macOS; the audit
  found host-zone behavior identical there.
- **Full suite, once at the end:** `cargo test --no-fail-fast` and
  `scripts/test-accept.sh`. `format`/`toIso` feed many goldens.

## Open Decisions

1. **S1 writer — DECIDED 2026-09-13 (owner): write seconds.** A sub-minute offset is
   written `±HH:MM:SS` (`ZZZ`+ `±HHMMSS`), and the reader accepts it. plan-135-D's
   prerequisite depends on this.
2. **S6 — DECIDED 2026-09-13 (owner): keep the offset.** The code keeps `dt.offset`
   when it is still valid for the resulting wall clock, as Java's
   `ZonedDateTime.plusDays` does, so the page's zero-day identity becomes true. Results
   change only when the target wall clock falls in an overlap *and* the current offset
   is one of that overlap's two offsets.
3. **S9.** Recommended: document the real rule. The alternative is accepting
   `fixedOffset(0, -30)`, but that makes a pair like `(-5, -30)` ambiguous.
4. **Unknown `TZ` values.** Recommended: add one sentence to the `local` page saying an
   unrecognised `TZ` behaves as the host's C library does, which on macOS and glibc is
   UTC. The alternative is leaving it unstated.

## Interaction with other work

- **bug-603** was filed earlier on 2026-09-13 for S1–S3 and is merged here. It is not a
  separate document.
- **plan-135-D** (`timezones::toIso`/`parseIso`) builds its RFC 9557 reader and writer on
  `datetime::parseIso`/`toIso`, so it cannot start until this bug closes with Open
  Decision 1 = (a).
- **bug-519** (`250df247e`): calendar-field range checks. S4/S5 extend the same rule to
  the offset field and must use the same code.
- **bug-521** (`6ab026ea4`): `toIso(dt, digits)`. S1 must keep the `digits = 9` nanos
  round trip.
- **bug-518** (`ebaa824c6`): `withZone` preserves the instant. Nothing here changes that.

## Summary

`datetime`'s zone *model* is correct standalone: host-zone resolution matched `zoneinfo`
everywhere it was measured. Its offset *text* is not. A sub-minute offset is written
without its seconds. Offset seconds, impossible minutes and trailing text are accepted
and silently misread. A too-large offset raises the wrong code. Five page claims contradict
the code (zero-day identity, purity, missing error lists, the `fixedOffset` formula), and
no fixture sets `TZ`. Named zones are no longer part of this bug; they are plan-135.
