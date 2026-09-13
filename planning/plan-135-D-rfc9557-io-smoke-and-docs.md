# plan-135-D: `timezones::toIso` / `parseIso` (RFC 9557), link smoke, cross-target proof, docs

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-135-C

This letter makes a zoned value writable and readable without losing its zone. It also
closes the family: consumption smoke, cross-target runtime proof, complete docs, and the
final gate.

```
EXPORT TYPE ZonedDateTime        ' dateTime AS datetime::DateTime, name AS String
timezones::toIso(dt AS datetime::DateTime, name AS String) AS String
timezones::toIso(dt AS datetime::DateTime, digits AS Integer, name AS String) AS String
timezones::parseIso(text AS String) AS timezones::ZonedDateTime
```

Behavioral outcome:
- **Write.** `timezones::toIso(timezones::civil(date(2026,7,15), time(9,0,0,0), "America/New_York"), "America/New_York")`
  → `2026-07-15T09:00:00.000-04:00[America/New_York]`.
- **Read back.** `parseIso` of that string returns the same instant, offset and name.
- **Whole corpus.** Across the plan-135-B corpus, `parseIso(toIso(v, 9, name))` recovers
  every value exactly.

Prerequisites: **plan-135-A § Prerequisites**. For this letter, these rows must be MET:
- bug-520 closed, with the offset writer printing seconds;
- boxes 2223/2227/2229 reachable, for Phase 3.

## 1. Goal

- `toIso` writes RFC 9557 `date-time "[" time-zone-name "]"`. It raises
  `ErrInvalidArgument` (`77050002`) when `dt.offset` is not the zone's offset at `dt`'s
  instant, and never writes a self-contradicting string.
- `parseIso` accepts the RFC 9557 §4.1 subset in § 4.2 and returns a `ZonedDateTime`.
  - Malformed or inconsistent text raises `ErrInvalidFormat` (`77050003`).
  - An unknown zone name raises `ErrNotFound` (`77050004`).
- A link smoke proves the `.mfp` consumption path. Its executable imports only `io` and
  `timezones`.
- The oracle's `offsets` and `civil` answers are byte-identical on macOS aarch64, Linux
  aarch64, x86_64 and riscv64.

### Non-goals

- No numeric-offset time-zone annotation (`[+05:30]`): `datetime::parseIso` already
  handles numeric offsets.
- No calendars other than ISO 8601.
- No change to `datetime::toIso`/`parseIso`. bug-520 owns their fixes.

## 2. Current State

- **plan-135-B/C:** `offsetAt`, `toZone`, and `civil` exist and agree with `zoneinfo`.
- **bug-520 (prerequisite):** after it closes, `datetime::toIso` writes sub-minute
  offsets as `±HH:MM:SS`. `datetime::parseIso` then rejects trailing text,
  malformed offsets and offsets ≥ 24 h with `77050003`.
- **RFC 9557 §4.1 ABNF** (fetched 2026-09-13):
  - `time-zone = "[" critical-flag time-zone-name / time-numoffset "]"`
  - `time-zone-part = time-zone-initial *time-zone-char`, where `time-zone-initial` is
    `ALPHA / "." / "_"` and `time-zone-char` adds `DIGIT / "-" / "+"`. The parts `"."` and
    `".."` are excluded.
  - `suffix-tag = "[" critical-flag suffix-key "=" suffix-values "]"`, where
    `suffix-key = (lcalpha / "_") *(lcalpha / "_" / DIGIT / "-")` and
    `suffix-values = 1*alphanum *("-" 1*alphanum)`.
  - `suffix = [time-zone] *suffix-tag`, and `critical-flag = [ "!" ]`.
- **RFC 9557 §3.3 and §3.4:**
  - A critical tag the recipient cannot process MUST be treated as an error.
  - On an offset/zone inconsistency, the recipient MUST act if the zone is critical and
    MAY act otherwise.
  - For a duplicated key, the first occurrence wins.
- **Node 24.12 `--harmony-temporal`** (ICU tz **2025b**, not 2026d) on 2026-09-13:

  | input | Temporal |
  |---|---|
  | `…-04:56:02[America/New_York]` (1850) | accepted, printed `-04:56` |
  | `…-04:56[America/New_York]` (1850) | accepted (minute-rounded LMT) |
  | `…Z[America/New_York]` | accepted, instant kept |
  | `…+00:00[America/New_York]` | rejected |
  | `…-04:00[!America/New_York]` | **rejected** |
  | `…[America/New_York][u-ca=gregory]` | accepted |
  | `…[America/New_York][!foo=bar]` | rejected |
  | `…[America/New_York][foo=bar]` | **rejected** |
  | `…-04:00[+05:30]` | rejected |
  | `…[america/new_york]` | accepted, canonicalised to `America/New_York` |
  | `…[US/Eastern]` | accepted, canonicalised to `America/New_York` |

  Temporal is therefore a **syntax** cross-check only. Its offsets come from a different
  tz release, and five rows differ from § 4.2 by design; § 4.4 declares them.
- **Consumer without `IMPORT datetime`:** it can hold, field-access and pass back a
  `datetime::DateTime` from a package (plan-135-A § 2, verified).
- **Target names:** `linux-aarch64`, `linux-x86_64`, `linux-riscv64`, `windows-x86_64`
  (`grep -rhno '"\(linux\|windows\|macos\)-[a-z0-9_]*"' src/target src/cli`). A Linux
  console build emits `-glibc.out` and `-musl.out`.

## 3. Design Overview

- **Two members, one record.** `toIso` is a checked wrapper over `datetime::toIso`.
  `parseIso` is an annotation scanner in front of `datetime::parseIso`, followed by a
  consistency check against `offsetAt`.
- **Correctness risk:** the consistency rule, especially minute-rounded sub-minute
  offsets and transition edges. That work lands behind the round-trip sweep over the
  whole plan-135-B corpus.
- **Rejected: `Temporal` as the reference for accept/reject.** Its tz release differs,
  and it rejects a valid critical zone annotation (measured above).
- **Rejected: honouring elective inconsistencies silently.** A zoned string whose offset
  disagrees with its zone is data corruption, so the package always acts.

## 4. Detailed Design

### 4.1 `toIso` (`src/iso.mfb`)

```
at = datetime::resolve(dt)
expected = offsetAt(name, at)                     ' 77050004 for an unknown name
IF dt.offset <> expected THEN FAIL error(77050002, "timezones: the DateTime's offset " & toString(dt.offset) & " is not " & name & "'s offset " & toString(expected) & " at that instant")
RETURN datetime::toIso(dt[, digits]) & "[" & canonicalName(name) & "]"
```

`digits` validation is `datetime::toIso`'s: `{0,3,6,9}`, else `77050002`.

### 4.2 `parseIso` (`src/iso.mfb`)

1. **Split the base.** `base` = the text before the first `[`. There must be at least one
   `[`; a missing bracket raises `77050003` with a message pointing to
   `datetime::parseIso`.
2. **Scan annotations to the end of the text.** Each annotation is `[`, an optional `!`,
   the content, then `]`. Any character after the last `]`, or a `]` missing, raises
   `77050003`.
3. **Read the zone annotation.**
   - The first annotation must be a time-zone name: its content has no `=` and matches
     `time-zone-name`, excluding the parts `.` and `..`.
   - A `time-numoffset` content raises `77050003` with a message pointing to
     `datetime::parseIso`.
   - A second zone annotation raises `77050003`.
4. **Read the suffix tags.** Each later annotation must be `suffix-key=suffix-values`,
   else `77050003`.
   - Key `u-ca`: value `iso8601` is accepted and any other value raises `77050003`, even
     when elective, because a calendar changes the meaning. Only the first `u-ca` counts
     (§3.3).
   - Any other key: critical raises `77050003`; elective is ignored.
5. **Check the zone exists.** If `zoneData(name) = ""`, raise `77050004` with the tzdb
   version.
6. **Parse the base.** `p = datetime::parseIso(base)`. After bug-520 this raises
   `77050003` on any malformed base.
7. **Check the offset against the zone.**
   - If `base` ends with `Z`/`z`: `instant = datetime::resolve(p)`. The local offset is
     unknown, so there is no inconsistency to check.
   - Otherwise, with `wall = datetime::resolve(p).seconds + p.offset` and
     `zo = offsetAt(name, datetime::resolve(p))`:
     - if `p.offset = zo`, then `instant = datetime::resolve(p)`;
     - else if `zo MOD 60 <> 0` and `p.offset` equals `zo` rounded to the nearest minute
       (half away from zero), then take `instant = wall − zo` (nanos kept) and require
       `offsetAt(name, instant) = zo`;
     - else raise `77050003`, naming both offsets. This applies whether the zone
       annotation is critical or elective.
8. **Return** `ZonedDateTime[datetime::inZone(instant, toZone(name, instant)), name]`.

Matching ignores case (owner decision 2026-09-13). The returned `name` is
`canonicalName(name)`, the tzdb spelling. A link stays a link, so `[us/eastern]` returns
`US/Eastern`.

### 4.3 Link smoke — `packages/timezones/runtime-smoke.sh`

Shape: `packages/logger/runtime-smoke.sh`.

1. Build the package.
2. Create a temporary executable project whose `packages` entry points at the `.mfp`.
3. Its `src/main.mfb` imports **only** `io` and `timezones`:

   ```
   LET z = timezones::parseIso("2026-07-15T09:00:00.000-04:00[America/New_York]")
   io::print(timezones::toIso(z.dateTime, z.name))
   LET again = timezones::civil(z.dateTime.date, z.dateTime.time, z.name)
   io::print(timezones::toIso(again, 9, z.name))
   ```

4. Assert both stdout lines exactly. Use a `fail()` helper, not a bare `[[ ]]`.

### 4.4 Oracle additions

- **`roundtrip` mode (probe only, no Python).** For every `offsets` job line:
  `v = datetime::inZone(instant, toZone(name, instant))`, then
  `parseIso(toIso(v, 9, name))`. Compare seconds, nanos, offset and name. Any difference
  is a mismatch.
- **`ixdtf` mode.**
  - Add `oracle/package.json`, pinning nothing beyond Node ≥ 24, and `temporal.mjs`,
    which runs under `node --harmony-temporal`.
  - The corpus is hand-written strings plus mutations: drop a bracket, uppercase a key,
    add `!` to an unknown key, duplicate the zone, a numeric zone, trailing text, a
    mismatched offset, `Z`, `+00:00`, and a minute-rounded LMT.
  - Instants are restricted to 2026–2030 in zones whose 2025b and 2026d offsets agree for
    that instant. The corpus generator checks agreement with `zoneinfo` and logs every
    skipped case with its reason.
  - Compare the verdict (accept/reject) and, on accept, epoch seconds and offset.
  - `divergences.json` declares, with reasons: critical zone annotation (Temporal
    rejects, we accept); elective unknown key (Temporal rejects, we ignore per §3.3);
    `u-ca=gregory` (Temporal accepts, we reject); link name (Temporal canonicalises
    a link to its target; we keep the link, in its tzdb spelling).

### 4.5 Cross-target proof

1. On macOS, build `oracle/probe` with `--target linux-aarch64`, `linux-x86_64` and
   `linux-riscv64`.
2. Copy each binary plus `jobs/offsets.txt` and `jobs/civil.txt` to its box:
   - 2223 (aarch64 glibc, native): full corpus;
   - 2227 (x86_64 musl, emulated): the **per-zone sample**;
   - 2229 (riscv64 musl, emulated): the **per-zone sample**.

   The per-zone sample is every job line for the first name of each of the 345 distinct
   zones. It still reaches every footer and every stored transition, which is where
   codegen differences would show; the full corpus on emulation is UNMEASURED and
   expected to be slow.
3. `cmp` each answers file against the macOS answers for the same lines.
4. **Windows is not run.** No execution harness exists on 2230. Record that, and prove
   only that `mfb build --target windows-x86_64 packages/timezones/oracle/probe`
   succeeds.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the work;
> `- [~]` partial; strike moot tasks with evidence; fill `Commit:`. **An unticked box
> means NOT DONE.**

### Phase 1 — `toIso`, `parseIso`, `ZonedDateTime`

- [x] Write `src/iso.mfb` per §§ 4.1–4.2. (`zoneAnnotation` for the write check;
      `zonedParseIso` for the reader. A probe of the post-bug-520 `datetime::parseIso`
      confirmed what § 4.2 assumes: `…Z` and `…+00:00` → offset 0; `…-04:56:02` →
      −17762 at −3771144000; `…-04:56` → −17760 at −3771144002; `""` → `error 77050003`.)
- [x] In `lib.mfb`, add the `EXPORT` declarations and `DOC` blocks, declaring the type
      the way `packages/jwt` declares its exported records. Build each `EXAMPLE` against
      the `.mfp`.
      - Both `toIso` overloads share one `DOC` block, the way `sqlite3` documents
        `query`. A second `DOC FUNC toIso` failed with `error[2-205-0003 DOC_DUPLICATE]`.
      - `ZonedDateTime` has a `DOC TYPE` block without `GROUP`. With it, the build failed
        with `DOC_GROUP_INVALID_CONTEXT`.
      - `bash /tmp/p135ex/build.sh toiso parseiso` printed:
        - `toiso`: `2026-07-15T09:00:00.123-04:00[America/New_York]` and
          `2026-07-15T09:00:00.123456789-04:00[America/New_York]`
        - `parseiso`: `America/New_York 1784120400 EDT`
- [x] In `src/test_iso.mfb`, cover (`mfb test packages/timezones` → `* iso`, 14 cases
      all `[P]`, `Tests: 48  Pass: 48  Fail: 0`):
  - [x] the New York January and July writes; (`…09:00:00.000-05:00[America/New_York]` /
        `…-04:00[…]`, plus the July read → `1784120400 0 -14400 America/New_York`)
  - [x] the 1850 New York LMT round trip (`-04:56:02` written, read back exact);
        (`1850-07-01T07:03:58.000-04:56:02[America/New_York]` → `-3771144000 0 -17762
        America/New_York`, label `LMT`)
  - [x] a minute-rounded `-04:56` read, recomputed to the exact instant; (→ −3771144000,
        −17762)
  - [x] `Z[America/New_York]`; (→ 1784120400, −14400, and rendered `09:00:00.000-04:00`)
  - [x] `+00:00[America/New_York]` → `77050003`; (also `-05:00` in July, elective and
        critical)
  - [x] `[!America/New_York]` accepted;
  - [x] `[u-ca=iso8601]` accepted, `[u-ca=gregory]` → `77050003`; (the first `u-ca` counts:
        `[u-ca=gregory][u-ca=iso8601]` refused, `[u-ca=iso8601][u-ca=gregory]` accepted)
  - [x] `[foo=bar]` ignored, `[!foo=bar]` → `77050003`; (and `[Foo=bar]`, not a valid key,
        → `77050003`)
  - [x] numeric zone `[+05:30]` → `77050003`;
  - [x] two zone annotations → `77050003`;
  - [x] trailing text after `]` → `77050003`;
  - [x] missing `]` → `77050003`; (also no annotation, `[]`, a tag before the zone, an
        `..` zone part, and a malformed base)
  - [x] `[Nowhere/Bogus]` → `77050004`;
  - [x] `[america/new_york]` accepted, `name` = `America/New_York`; `[us/eastern]` → `US/Eastern`;
  - [x] `toIso(dt, "america/new_york")` writes `[America/New_York]`;
  - [x] `toIso` with a wrong offset → `77050002`; (a January 09:00 at fixed −14400; an
        unknown zone → `77050004`)
  - [x] `toIso(dt, 5, name)` → `77050002`; (`digits = 0` writes
        `2026-07-15T09:00:00-04:00[America/New_York]`)
  - [x] nanos preserved at `digits = 9`. (`…09:00:00.123456789-04:00[America/New_York]`
        → `1784120400 123456789 -14400 America/New_York`)
- [x] Add the README section "Writing and reading a zoned time", including the
      case-insensitive name rule and the always-act inconsistency rule.

Acceptance: every listed case passes.
  Check: `target/release/mfb test packages/timezones` → `Fail: 0` with the `iso` group present (est. 1 min).
Commit: —

### Phase 2 — oracle `roundtrip` and `ixdtf` modes, and the link smoke

- [ ] Add the `roundtrip` mode (§ 4.4) to the probe and `run.sh`.
- [ ] Add the `ixdtf` mode (§ 4.4): `temporal.mjs`, the corpus, the declared divergences
      and the skipped-case log.
- [ ] Mutation proof: make `parseIso` skip the consistency check. `ixdtf` must report
      mismatches; revert.
- [ ] Mutation proof: make `toIso` drop nanos. `roundtrip` must report mismatches;
      revert.
- [x] Write `packages/timezones/runtime-smoke.sh` (§ 4.3). (`runtime-smoke.sh
      /Users/…/mfb/target/release/mfb` → `timezones runtime smoke passed`, `EXIT=0`. The
      consumer imports only `io` and `timezones`, reads the zoned string, passes
      `z.dateTime.date`/`.time` back to `civil`, and asserts both lines exactly.)

Acceptance: round trips are exact across the whole corpus, syntax verdicts match
Temporal except for the declared divergences, and the `.mfp` links into an executable
that imports only `io` and `timezones`.
  Check: `packages/timezones/oracle/run.sh '' roundtrip ixdtf; echo EXIT=$?` → `EXIT=0`; mutation runs → mismatches > 0; `packages/timezones/runtime-smoke.sh; echo EXIT=$?` → `EXIT=0` (est. UNMEASURED for `roundtrip`; set from the `offsets` corpus time recorded in plan-135-B).
Commit: —

### Phase 3 — cross-target proof

- [ ] Run § 4.5 on 2223, 2227 and 2229. Record each `cmp` result and wall time in
      `oracle/README.md`.
- [ ] Record the Windows build-only result, and that no Windows execution was possible.

Acceptance: the same answers on every executed target.
  Check: `cmp` per box → exit 0; `target/release/mfb build --target windows-x86_64 packages/timezones/oracle/probe` → `Wrote executable` (est. UNMEASURED on the emulated boxes; the per-zone sample is the smallest input that still reaches every footer and transition).
Commit: —

### Phase 4 — docs and family close

- [ ] Complete `packages/timezones/README.md`: an API table of all five members plus the
      record, every example compiled, where the rules come from, how to update, what the
      package never does, and `timezones` vs `datetime::local()`.
- [ ] Check that `target/release/mfb pkg doc packages/timezones/timezones.mfp` renders
      all five members and `ZonedDateTime`.
- [ ] In `planning/bug-backlog.md`, update the datetime line: named zones are delivered
      by plan-135.
- [ ] Move `plan-135-A` through `plan-135-D` to `planning/completed/` as each letter
      completes.

Acceptance: the package docs render and every example builds.
  Check: `target/release/mfb pkg doc packages/timezones/timezones.mfp | grep -c "timezones::"` → at least 6 (est. 1 min).
Commit: —

## Validation Plan

- **Tests:** `test_data`, `test_posix`, `test_offsets`, `test_civil`, `test_iso`.
- **Coverage check:** the mutation runs in B, C and D. Each proves its oracle mode
  reaches the code.
- **Runtime proof:** the link smoke (§ 4.3), plus the cross-target `cmp` (§ 4.5).
- **Doc sync:** the package README and `DOC` blocks, `tools/tzdb/README.md`,
  `third_party/tzdb/README.md`, the `.ai/testing-gates.md` oracle row (B), and
  `planning/bug-backlog.md`.
- **Final gate** (run ONCE, after Phase 4; est. UNMEASURED, dominated by the oracle
  corpus):
  - `sh scripts/check-generated.sh; echo EXIT=$?` → `EXIT=0`
  - `target/release/mfb test packages/timezones` → `Fail: 0`
  - `packages/timezones/oracle/run.sh; echo EXIT=$?` → `EXIT=0` (all modes)
  - `packages/timezones/runtime-smoke.sh; echo EXIT=$?` → `EXIT=0`
  - `git diff --stat <family base>..HEAD -- src tests src/docs` → empty. This proves
    owner ruling 2 held, and it is why `cargo test` and `scripts/test-accept.sh` are not
    part of this gate: they cover only paths this family does not touch.

## Open Decisions

- **Name matching — DECIDED 2026-09-13 (owner): ignore case.** Names match
  case-insensitively and are returned and written in tzdb spelling. Links are kept as
  links (plan-135-A § 4.3 `canonicalName`).
- **`toIso` digits overload.** Recommend **both arities**, mirroring
  `datetime::toIso`. Only `digits = 9` round-trips nanos.
- **Export `timezones::version()` and `names()`.** Recommend **yes, as additive
  members**. Callers storing zoned strings need the release, for example to explain a
  `77050004` after an update.

## Corrections

## Summary

The risk sits in `parseIso`'s consistency rule: sub-minute offsets, `Z`, and transition
edges. A whole-corpus round-trip sweep and a Temporal syntax cross-check cover it, with
every divergence declared. The letter ends the family with the link smoke, byte-identical
answers on three Linux architectures, and a final gate that also proves nothing under
`src/` changed.
