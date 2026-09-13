# plan-135-B: `timezones::offsetAt` / `toZone` — the RFC 8536 lookup, the POSIX TZ rule evaluator, and the zoneinfo oracle

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-135-A

This letter turns plan-135-A's committed zone table into answers. It adds the two
per-instant members of `packages/timezones`:

```
timezones::offsetAt(name AS String, at AS datetime::Instant) AS Integer
timezones::toZone(name AS String, at AS datetime::Instant) AS datetime::Zone
```

It also adds the differential oracle that checks them against an independent
implementation reading the same IANA release.

Behavioral outcome for this letter:
- **Oracle mode.** `packages/timezones/oracle/run.sh '' offsets` exits 0 with zero
  mismatches between the package and Python `zoneinfo`. The oracle loads `tzdata==2026.4`
  (IANA 2026d) with the host search path disabled. The committed corpus covers every
  stored transition ±1 s of every distinct zone, and every transition through 2100 of
  every zone whose rules still change.
- **Tests.** `mfb test packages/timezones` pins these points:
  - New York: −18000 on 2026-01-15 and −14400 on 2026-07-15.
  - New York in 1850: −17762, label `LMT`.
  - An unknown name raises `errorCode::ErrNotFound`.

Prerequisites: see **plan-135-A § Prerequisites** (the family table). This letter has no
extra rows.

References — read these first:

- plan-135-A §§ 2, 4 (measured populations, the data encoding, the generator's
  assertions) — this letter's decoder reads exactly that format.
- RFC 8536 §3.2 (which time type applies to a timestamp) and §3.3 / §3.3.1 (the footer TZ
  string and its extensions) — <https://www.rfc-editor.org/rfc/rfc8536>.
- POSIX.1-2017 XBD §8.3, `TZ` — the footer grammar RFC 8536 extends.
- `packages/jwt/oracle/` (`README.md`, `run.sh`, `probe/src/main.mfb`,
  `divergences.json`) — the oracle shape this letter copies: one probe answers a whole
  job file.
- `.ai/testing-gates.md` § oracle homes — a package oracle lives in
  `packages/<pkg>/oracle/`.
- `mfb man datetime types`, `mfb man datetime inZone`, `fixedOffset`, `weekday`,
  `daysInMonth`, `civil`, `resolve`.

## 1. Goal

- `timezones::offsetAt(name, at)` returns the UTC offset in seconds (east positive) that
  the named zone applies at `at`. It uses the RFC 8536 §3.2 rules over plan-135-A's
  table, with the footer rule for timestamps on or after the last stored transition.
- `timezones::toZone(name, at)` returns `datetime::Zone[offset, 1, abbreviation]`. Here
  `offset` is exactly `offsetAt(name, at)`, and `abbreviation` is the tzdb designation in
  force (`EST`, `EDT`, `LMT`, `-03`).
- An unknown name raises `errorCode::ErrNotFound` (`77050004`). The message names the
  zone and the tzdb release.
- The oracle's `offsets` mode reports zero mismatches on the committed corpus, and a
  deliberately broken evaluator makes it report mismatches.

### Non-goals

Family non-goals are in plan-135-A § 1. For this letter:

- No wall-clock → instant member. That is plan-135-C.
- No string format. That is plan-135-D.
- No exported `LocalType`, `isDst`, transition list, or rule string. The public surface is
  the two members above.
- No cache of decoded zones in package-global state (see Open Decisions).

## 2. Current State

After plan-135-A lands, `packages/timezones/src/data.mfb` defines three package-visible
functions:

- `zoneData(name) AS String`, which is `""` for an unknown name;
- `zoneNames() AS List OF String`;
- `tzdbVersion() AS String`.

Each zone string is `types|transitions|footer` (plan-135-A § 4.3):

- `types`: `utoff,isdst,abbr` joined by `;`, in TZif order, so index 0 is RFC 8536's
  "time type 0";
- `transitions`: `unixSeconds,typeIndex` joined by `;`, ascending, possibly empty;
- `footer`: the TZif v2+ footer TZ string, possibly empty.

Measured on 2026d (commands in plan-135-A § 2):

- 17018 stored transitions and 1598 local-time types.
- 29 zones have zero transitions.
- 94 distinct footers. 30 carry a DST rule, and those 30 are used by 106 zones.
- Every DST rule date uses the `Mm.w.d` form: 60 occurrences, 0 `Jn`, 0 `n`.
- 4 rule times fall outside 0..24 h, e.g. `IST-2IDT,M3.4.4/26,M10.5.0` and
  `<-02>2<-01>,M3.5.0/-1,M10.5.0/0`.
- 44 footers use quoted `<…>` designations.

The generator fails closed on a `Jn`/`n` date form (plan-135-A § 4.2). This letter
therefore implements `Mm.w.d` only, and that form is the whole population it must handle.

RFC 8536 §3.2, verbatim (fetched 2026-09-13, `sed -n 583,593p rfc8536.txt`):

> Local time for timestamps before the first transition is specified by the first time
> type (time type 0). Local time for timestamps on or after the last transition is
> specified by the TZ string in the footer (Section 3.3) if present and nonempty;
> otherwise, it is unspecified. If there are no transitions, local time for all
> timestamps is specified by the TZ string in the footer if present and nonempty;
> otherwise, it is specified by time type 0.

### Verified properties

- **Package-built zones.** A package can build `datetime::Zone[offset, 1, label]` with a
  whole-second offset. `datetime::civil`, `resolve`, `offsetAt` and `inZone` honour it to
  the second: the 2026-09-13 probe resolved `Zone[-17762, 1, "LMT"]` 1880-01-01 09:00 to
  `13:56:02Z`. `Zone.kind` is typed `Integer`, and `1` is `FixedOffset`
  (`grep -n "Zone\[" src/codegen/builtins/datetime/func_fixed_offset.rs`). This letter
  pins that ordinal with a test.
- **Decode cost.** Split-decode plus binary search on the largest zone costs 10,000
  lookups in 0.50 s real for the whole process: Asia/Hebron, 310 transitions, measured
  2026-09-13 with a throwaway consumer. No cache is needed for correctness or for the
  oracle's corpus size.
- **Oracle data.** `tzdata==2026.4` reports `IANA_VERSION` `2026d`, and
  `zoneinfo.reset_tzpath([])` keeps the host's zoneinfo out.
  `zoneinfo.available_timezones()` is set-equal to the generator's 598 names (probe
  2026-09-13).
- **UNVERIFIED: weekday ordering.** The `datetime::Weekday` enum lists `Monday` first
  (`mfb man datetime types`). How `datetime::weekday` maps to 0=Sunday is pinned by a
  Phase 1 test rather than assumed.
- **Probe job-file transport.** `packages/jwt/oracle/probe/src/main.mfb` takes one
  argument, the job file path, from `os::args()` and reads it with `fs::readText`
  (`grep -n "os::args\|fs::readText" packages/jwt/oracle/probe/src/main.mfb`).

## 3. Design Overview

Two pieces, layered:

1. **`src/posix.mfb` — the footer evaluator.** It is a pure function of `(footer,
   unixSeconds)` → `(utoff, isDst, abbreviation)`, with no zone table.
2. **`src/zone.mfb` — decode + RFC 8536 lookup.** It is `localTypeAt(name, seconds)`
   over `zoneData`, and it calls the evaluator on and after the last transition, or
   always when a zone has no transitions. `lib.mfb` exposes `offsetAt` and `toZone` over
   it.

**Correctness risk concentrates in the evaluator.** Because plan-135-A compiles with
`zic -b slim`, a zone's stored transitions end where its footer can take over. For New
York, the answer for *every current date* comes from the footer, not the table. The
evaluator is where southern-hemisphere rules (start after end), RFC 8536 §3.3.1 times
outside 0..24 h, quoted designations and year boundaries all live. It is therefore landed
first, alone, behind its own tests, and the oracle's future-transition sweep through 2100
is aimed squarely at it.

**Gate class:** new behavior. Byte-identity is not a gate here. The only byte compare in
the family is plan-135-A's generated-artifact check, and this letter does not touch
`data.mfb`.

Rejected:

- **Evaluating the tzdb rule language (`Rule`/`Zone` lines) at runtime.** That
  re-implements `zic` in MFBASIC with no oracle for the intermediate form. plan-135-A
  compiles ahead of time instead.
- **`zic -b fat` so current dates come from the table.** Fat output still ends in 2037
  (RFC 8536 32-bit-era compatibility data) and still needs the footer afterward. That
  hides the evaluator from every current-date test instead of removing it. It also costs
  23590 transitions against 17018 slim.
- **A decoded-zone cache in a package-global `MUT`.** Arena state is per-thread
  (`.ai/canvas-threading.md`), and the measured cost does not need it.

## 4. Detailed Design

### 4.1 Footer evaluator (`src/posix.mfb`)

Grammar accepted (RFC 8536 §3.3 over POSIX `TZ`; anything else FAILs as a corrupt table,
which the generator makes unreachable):

```
footer  = std offset [ dst [ offset ] [ "," rule "," rule ] ]
std/dst = "<" 1*( ALPHA / DIGIT / "+" / "-" ) ">" / 3*ALPHA
offset  = [ "+" / "-" ] hh [ ":" mm [ ":" ss ] ]          ; hh 0..24
rule    = "M" month "." week "." day [ "/" time ]          ; month 1..12, week 1..5, day 0..6
time    = [ "+" / "-" ] hhh [ ":" mm [ ":" ss ] ]         ; hours -167..167 (RFC 8536 §3.3.1)
```

Semantics:

- **Sign.** A POSIX offset is positive *west* of Greenwich, so `utoff = −offset`. `EST5`
  → −18000.
- **Default DST offset.** When the DST offset is omitted, `dstUtoff = stdUtoff + 3600`.
- **Default rule time.** When `/time` is omitted, the time is `02:00:00`.
- **Date form.** `Mm.w.d` is day `d` (0 = Sunday) of week `w` of month `m`; `w = 5` means
  the last such day. Compute it as follows:
  1. `first = weekdayIndex(datetime::date(y, m, 1))`, with 0 = Sunday.
  2. `day = 1 + (d − first + 7) MOD 7 + 7 × (w − 1)`.
  3. While `day > datetime::daysInMonth(y, m)`, subtract 7.
- **Transition instants for year `y`.** Let `localMidnight(y, rule)` be
  `datetime::resolve(datetime::civil(date, time(0,0,0,0), datetime::utc())).seconds`.
  - `start_y = localMidnight(y, startRule) + startTime − stdUtoff`, because the start is
    given in standard local time.
  - `end_y = localMidnight(y, endRule) + endTime − dstUtoff`, because the end is given in
    daylight local time.
- **Choosing the type at instant `T`.**
  1. Let `y` = the year of `T` in standard time:
     `datetime::inZone(datetime::instant(T, 0), datetime::fixedOffset(stdUtoff)).date.year`.
  2. Build the six events `(start_k, DST)` and `(end_k, STD)` for `k ∈ {y−1, y, y+1}`.
  3. Sort the events by instant and take the last one ≤ `T`. Its kind is the answer.
  4. If no event is ≤ `T`, the answer is the kind *opposite* to the earliest event's.

  Checking three years keeps the evaluator correct at year boundaries and for times
  outside 0..24 h, without special-casing southern-hemisphere rules.
- **Output.** The result is `LocalType[utoff, isDst, abbreviation]`, with the `<>`
  stripped from a quoted designation.

`TYPE LocalType` (package-visible, not exported) holds `utoff AS Integer`,
`isDst AS Boolean` and `abbreviation AS String`.

### 4.2 Lookup (`src/zone.mfb`)

`FUNC localTypeAt(name AS String, seconds AS Integer) AS LocalType`:

1. `data = zoneData(name)`. If it is `""`:
   `FAIL error(ERR_UNKNOWN_ZONE, "timezones: unknown zone name \"" & name & "\" (tzdb " & tzdbVersion() & ")")`.
2. Split `data` on `|` into `types`, `transitions` and `footer`.
3. If `transitions` is empty: return `footer <> ""` ? `footerType(footer, seconds)` :
   `types[0]`.
4. Otherwise, binary-search for the last transition with time ≤ `seconds`:
   - none → `types[0]`;
   - the last stored transition, with a non-empty `footer` →
     `footerType(footer, seconds)`;
   - otherwise → `types[typeIndex]`.

`at.nanos` is ignored. Transitions are whole seconds and `nanos ≥ 0`, so the second
containing `at` is `at.seconds`.

### 4.3 Public members (`src/lib.mfb`)

```
PUBLIC LET ERR_UNKNOWN_ZONE AS Integer = 77050004   ' 7-705-0004 errorCode::ErrNotFound

EXPORT FUNC offsetAt(name AS String, at AS datetime::Instant) AS Integer
EXPORT FUNC toZone(name AS String, at AS datetime::Instant) AS datetime::Zone
```

- **Error constant.** Declare it with the same comment style as `packages/jwt/src/core.mfb`'s
  error block: a registry code, exported by name.
- **`toZone`.** It returns `datetime::Zone[t.utoff, 1, t.abbreviation]`.
- **DOC blocks.** Each member gets a `DOC` block (`DESC`, one-line `ARG`/`RET`,
  `EXAMPLE`). The `EXAMPLE` must compile: `scripts/man-run-examples.sh` does not cover
  packages, so Phase 2 builds each example.

### 4.4 Oracle (`packages/timezones/oracle/`)

Files:

- **`README.md`.** What is compared, why `zoneinfo` + `tzdata` 2026.4 is independent of
  the package (different code, same pinned release), how to run it, and how to bump the
  pin with the release (plan-135-A § 4.5).
- **`requirements.txt`.** `tzdata==2026.4`.
- **`corpus.py`.** Writes `jobs/offsets.txt` deterministically, one `offset <name>
  <unixSeconds>` per line:
  1. Every stored transition `t` of every distinct zone: `t−1`, `t`, `t+1`, read from
     `packages/timezones/src/data.mfb`. The *inputs* may come from the package; the
     *answers* come from `zoneinfo`.
  2. For every name whose 2030 January and July offsets differ in `zoneinfo`, every
     offset change from 2026-01-01 through 2100-12-31: a daily scan, bisected to the
     second, emitting `c−1`, `c`, `c+1`.
  3. For every one of the 598 names, 00:00 UTC on 1 January and 1 July of every year
     1800–2100.
  4. For every name, instants before its first transition: `−2^40` and the first stored
     transition − 86400.
  5. The corpus is capped at the `datetime` range Python represents: years 1..9999.
- **`oracle.py`.**
  1. Call `zoneinfo.reset_tzpath([])` before any lookup.
  2. Assert `tzdata.IANA_VERSION == RELEASE`, where `RELEASE` is read from
     `tools/tzdb/gen_timezones_data.py`.
  3. Answer each job line with `<utoff> <abbreviation>`, computed from
     `(datetime(1970,1,1,tzinfo=UTC) + timedelta(seconds=s)).astimezone(ZoneInfo(name))`
     via `.utcoffset()` and `.tzname()`.
- **`probe/`.** An MFBASIC executable project that depends on `../../timezones.mfp`
  through the same `project.json` `packages` shape as `packages/logger/runtime-smoke.sh`.
  It reads a job file and writes one answer line per job line
  (`toZone(name, instant(s, 0))` → `offsetSeconds label`, or `error <code>`). It uses
  jwt's job-file transport. A refusal is an answer; a non-zero exit means the probe broke.
- **`diff.py`.** Compares the two answer files line by line. It prints the first 20
  mismatches with their job lines, subtracts the entries declared in
  `divergences.json` (expected: empty), and exits 1 on any remaining mismatch.
- **`run.sh [mfb] [modes…]`.**
  1. Build the package (`mfb build -q packages/timezones`) and the probe.
  2. Create `oracle/.venv` and install `requirements.txt`.
  3. Run each mode (`offsets` here; `civil` in C; `ixdtf` in D).
  4. Exit 0 iff every mode agrees.

  `oracle/.gitignore` ignores `.venv/`, `jobs/`, `probe/build/` and `probe/packages/`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` for partial with what remains; moot tasks struck through with
> evidence, never deleted; fill `Commit:` when the phase lands. **An unticked box means
> NOT DONE.**

### Phase 1 — the footer evaluator, alone

It is a pure function with no callers yet, so it lands safely: it changes no existing
behavior, and it concentrates the letter's risk behind its own tests.

- [x] `packages/timezones/src/posix.mfb`: `TYPE LocalType`, `FUNC footerType(footer AS
      String, seconds AS Integer) AS LocalType`, and the parse/date/event helpers of
      § 4.1 as `PRIVATE FUNC`s. A malformed footer FAILs with
      `error(77050003, "timezones: corrupt footer rule …")`. (`ruleDayOfMonth` and
      `weekdayOfFirst` are package-visible so the weekday pin can test them; see
      Corrections.)
- [x] `packages/timezones/src/test_posix.mfb`. Every expected value in these cases comes
      from `zoneinfo` with `reset_tzpath([])` in `oracle/.venv`, and the exact one-liner
      is pasted above each `TCASE`. Cases (`mfb test packages/timezones` → `* posix`,
      9 `[P]`, `Tests: 16  Pass: 16  Fail: 0`):
  - [x] `EST5EDT,M3.2.0,M11.1.0` at 2026-03-08 06:59:59Z / 07:00:00Z, and at
        2026-11-01 05:59:59Z / 06:00:00Z. (1772953199/1772953200, 1793512799/1793512800)
  - [x] A southern-hemisphere rule, `<+1030>-10:30<+11>-11,M10.1.0,M4.1.0` (Lord Howe),
        either side of both 2026 changes. (1775314799/800, 1791041399/400)
  - [x] Both RFC 8536 §3.3.1 extension footers from § 2, either side of each 2030
        change. (Asia/Jerusalem `…/26` 1900972799/800, 1919285999/6000; America/Nuuk
        `…/-1` 1901149199/200, 1919293199/200)
  - [x] A no-DST footer: `<-05>5` → −18000 `-05`. (America/Lima at 1780000000, and at
        −2^40)
  - [x] The last-week rule `M10.5.0` in a month where week 5 does not exist. (Europe/London
        `GMT0BST,M3.5.0/1,M10.5.0`: `ruleDayOfMonth(2026,10,5,0)` = 25; the switch at
        1792890000)
  - [x] Year boundary: 1 January 00:00:00 local, for a southern rule. (Lord Howe
        1767185999/1767186000 → +11 both)
  - [x] Weekday-mapping pin: the first Sunday of March 2026 is the 1st.
        (`weekdayOfFirst(2026,3)` = 0, `ruleDayOfMonth(2026,3,1,0)` = 1, plus 1970-01 → 4
        and 1969-12 → 1 across the epoch)
  - [x] A malformed footer traps `77050003`, written in the `expectTrap` style of
        `packages/jwt/src/test_verify.mfb`. (`J60` date, a leading digit, DST with no
        rule, month 13, empty `<>`)

Acceptance: every evaluator case passes with values independently produced by `zoneinfo`.
  Check: `target/release/mfb test packages/timezones` → the `posix` group all `[P]`, `Fail: 0` (est. 1 min).
Commit: 28b6af197

### Phase 2 — lookup and the public members

- [x] `packages/timezones/src/zone.mfb`: `localTypeAt` per § 4.2. (Also holds the shared
      `unknownZone(name) AS Error`, so the 77050004 message has one spelling.)
- [x] `packages/timezones/src/lib.mfb`: `ERR_UNKNOWN_ZONE`, `offsetAt` and `toZone`, with
      `DOC` blocks (§ 4.3). Build each `EXAMPLE` in a scratch project that imports the
      built `.mfp`, and record the command. (`bash /tmp/p135ex/build.sh offsetat tozone`:
      `mfb build -q packages/timezones`, copy `timezones.mfp` into each scratch project's
      `packages/`, `mfb build -q`, run → `offsetat` prints `-18000` / `-14400`; `tozone`
      prints `EDT -14400` / `2026-07-15T09:00:00.000-04:00`)
- [x] `packages/timezones/src/test_offsets.mfb`. Every expected value comes from a
      `zoneinfo` one-liner pasted above the case. Cases (`mfb test packages/timezones` →
      `* offsets`, 9 `[P]`, `Tests: 25  Pass: 25  Fail: 0`):
  - [x] New York 2026-01-15 09:00 local and 2026-07-15 09:00 local: −18000 / −14400,
        labels `EST` / `EDT`. This is the original bug-520 case. (1768485600 /
        1784120400)
  - [x] New York 1850-07-01: −17762 `LMT`, before the first transition. (−3771169438;
        first stored transition −2717650800)
  - [x] New York 2100-07-01: −14400, on the footer path. (4118097600 → `EDT`)
  - [x] `Etc/GMT+5` at 0 and at 4102444800: −18000, a zone with no transitions.
  - [x] `US/Eastern` equals `America/New_York` at three instants. (−3771144000,
        1784106000, 4118083200)
  - [x] `Asia/Kolkata` 2026: 19800. (1782844200 → `IST`)
  - [x] Exactly at a zone's last stored transition, and one second before it. (New York's
        last stored transition is 1173596400: 1173596399 → −18000 `EST` from the table,
        1173596400 → −14400 `EDT` from the footer)
  - [x] `offsetAt("Nowhere/Bogus", …)` and `offsetAt("", …)` trap `77050004`, and the
        message contains `2026d`. (`toZone` too; the message also names the zone)
  - [x] Ordinal pin: `toZone("UTC", instant(0,0)).kind = 1` and
        `datetime::fixedOffset(1, 0).kind = 1`.
- [x] README section "Offsets at an instant": `offsetAt`/`toZone` with the New York
      example, and the rule that a `datetime::Zone` from `toZone` is a snapshot for that
      instant.

Acceptance: the public members return `zoneinfo`'s answers on the hand cases, and an
unknown name raises `ErrNotFound`.
  Check: `target/release/mfb test packages/timezones` → `Fail: 0`, with the `offsets` group present (est. 1 min).
Commit: bd0c5a22b

### Phase 3 — the oracle, and proof that it can fail

- [ ] Add `oracle/README.md`, `requirements.txt`, `corpus.py`, `oracle.py`, `probe/`,
      `diff.py`, `divergences.json` (`[]`), `run.sh` and `.gitignore`, per § 4.4.
- [ ] Record the corpus size. It is UNMEASURED until `corpus.py` runs. Put the line
      count and the `run.sh` wall time in the oracle README, and in this plan's
      Corrections if they change an estimate.
- [ ] Mutation proof: temporarily make `footerType` return the standard type
      unconditionally, run `run.sh '' offsets`, confirm a non-zero mismatch count, and
      revert. Record the count in the README. Without this, a probe that never reaches
      the evaluator would pass.
- [ ] `.ai/testing-gates.md` § oracle homes: add `packages/timezones/oracle` (Python
      `zoneinfo`, `tzdata` pinned to the vendored release) to the package row.

Acceptance: the package agrees with `zoneinfo` on the whole corpus, and a broken
evaluator is caught.
  Check: `packages/timezones/oracle/run.sh '' offsets; echo EXIT=$?` → `0 mismatches`, `EXIT=0`; the mutation run → mismatches > 0 (est. UNMEASURED — set from the corpus line count × the measured 50 µs per lookup, plus the Python side; if > 10 min, record why the full corpus is needed: the future-transition sweep is the only coverage of every DST footer).
Commit: —

## Validation Plan

- **Tests:** `test_posix.mfb` and `test_offsets.mfb` (Phases 1–2), including the error
  cases.
- **Coverage check:** the mutation run in Phase 3 proves the corpus reaches the
  evaluator. A green oracle without it proves nothing.
- **Runtime proof:** the oracle's probe is a real executable consuming the `.mfp`.
  Cross-target runs are plan-135-D's.
- **Doc sync:** the package README section, `DOC` blocks, and the `.ai/testing-gates.md`
  row.
- **Final gate:** plan-135-D § Validation Plan. This letter runs only its scoped checks.

## Open Decisions

- **Cache decoded zones.** Recommend **no**. Measured 50 µs worst case per lookup, and
  package-global mutable state interacts with per-thread arenas. Revisit only with a
  measured consumer need. (§ 3)
- **Export `isDst`.** Recommend **no** for this plan. The user's surface is offset and
  zone, and `datetime::Zone` has no field for it. A later `timezones::isDst(name, at)` is
  additive.

## Corrections

- **Phase 1: the weekday comes from epoch days, not `datetime::weekday`.** § 4.1 step 1
  called for `weekdayIndex(datetime::date(y, m, 1))`, and § 2 marked the enum mapping
  UNVERIFIED. `weekdayOfFirst` computes `floorMod(floorDiv(utcMidnight(y, m, 1), 86400)
  + 4, 7)` instead, because 1970-01-01 was a Thursday. That needs no enum ordinal. The
  pin the plan asked for tests it: 2026-03 → 0 (Python: `date(2026,3,1)` is `Sunday`),
  2026-10 → 4 (`Thursday`), and 1969-12 → 1 (Monday), a pre-epoch month.
- **Phase 1: MFBASIC `/` and `MOD` truncate toward zero.** A probe printed
  `div -3 3 -1` for `-7 / 2`, `7 / 2`, `-86401 / 86400`, and `mod -1 1` for `-7 MOD 3`,
  `7 MOD -3`. The evaluator therefore uses its own `floorDiv`/`floorMod` for pre-epoch
  dates. § 4.1 had assumed floor semantics without saying so.
- **Phase 1: § 4.1's grammar marks `"," rule "," rule` optional after `dst`.** RFC 8536
  allows a DST footer without rules, but plan-135-A's generator rejects one. The
  evaluator therefore FAILs `77050003` on it (a test covers `EST5EDT`), rather than
  guessing POSIX's implementation-defined default rules.

## Summary

The real risk in this letter is the POSIX footer evaluator. Every current-date answer for
106 zones passes through it, because the table is compiled slim. It lands first, alone,
with `zoneinfo`-sourced cases, and the oracle's future sweep plus a mutation run prove it
end to end. Everything outside `packages/timezones/` is untouched, apart from one row in
`.ai/testing-gates.md`.
