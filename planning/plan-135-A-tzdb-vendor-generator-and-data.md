# plan-135-A: `packages/timezones` — vendored IANA tzdb 2026d, the generator, and the zone table

Last updated: 2026-09-13
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing

plan-135 builds **`timezones`**, a source MFBASIC package in `packages/timezones/`. It
answers named-zone questions from the IANA Time Zone Database, and the tzdb copy is
**committed to this repository**. It never reads the host's zone database. It is not a
compiler builtin: nothing under `src/` changes. The owner made both rulings on
2026-09-13, while re-scoping bug-520.

The family's public surface, as agreed with the owner:

| member | letter |
| --- | --- |
| `timezones::offsetAt(name AS String, at AS datetime::Instant) AS Integer` | B |
| `timezones::toZone(name AS String, at AS datetime::Instant) AS datetime::Zone` | B |
| `timezones::civil(date AS datetime::Date, time AS datetime::Time, name AS String) AS datetime::DateTime` | C |
| `timezones::toIso(dt AS datetime::DateTime, name AS String) AS String` (+ digits form) | D |
| `timezones::parseIso(text AS String) AS timezones::ZonedDateTime` | D |

Family outcome:
- **Core case.** `timezones::civil(date(2026,1,15), time(9,0,0,0), "America/New_York")`
  is `-05:00`, and the same reading in July is `-04:00`. That holds on macOS and on each
  Linux target, and the answer comes from bytes in this repo.
- **Round trip.** A zoned value written with `timezones::toIso` and read back with
  `timezones::parseIso` has the same instant, offset and zone name.

**This letter** lands the data. The unmodified IANA 2026d release tarballs are vendored.
`tools/tzdb/gen_timezones_data.py` compiles them with the release's own `zic` and emits
`packages/timezones/src/data.mfb`. `scripts/check-generated.sh` regenerates that file and
fails on any difference. A package skeleton proves every zone name reaches its data.

Behavioral outcome for this letter:
- `sh scripts/check-generated.sh` prints
  `ok: packages/timezones/src/data.mfb matches tools/tzdb/gen_timezones_data.py` and
  exits 0.
- The generator's statistics line equals the measured
  `names 598 distinct 345 transitions 17018 types 1598 footers 94`.
- `target/release/mfb test packages/timezones` passes. It shows that all 598 names
  resolve, links share their target's data, and an unknown or empty name resolves to
  nothing.

References — read these first:

- **bug-520** — the origin, re-scoped the same day. It records the owner's ruling that
  `datetime` must be correct without a zone database, and that named zones are this
  plan's job.
- **IANA tz** — <https://www.iana.org/time-zones>; release files under
  <https://data.iana.org/time-zones/releases/>. Read `theory.html` (inside the data
  tarball) for zone-name rules.
- **RFC 8536** — TZif. §3.1 header, §3.2 data block and time-type selection, §3.3
  footer.
- **`tools/unicode-tables/README.md`, `third_party/unicode/`,
  `scripts/check-generated.sh`** — the vendored-data plus generator plus drift-gate
  precedent this letter copies.
- **`packages/jwt/` (`project.json`, `src/core.mfb` error block, `src/test_*.mfb`,
  `oracle/`) and `packages/logger/runtime-smoke.sh`** — the source-package precedent.
- **`.ai/testing-gates.md` § oracle homes** — a package oracle lives in
  `packages/<pkg>/oracle/`.
- **`.ai/resources-packages.md`** — package/import subsystem.
- **`mfb spec language modules-and-packages`, `mfb spec language functions`** —
  visibility: `PRIVATE` is file-scoped, a bare `FUNC` is package-visible, `EXPORT` is
  public.

## Prerequisites

The whole plan-135 family (A–D) is gated here. Letters B–D point to this table.

| Must be true | Command | Status |
|---|---|---|
| plan-135 number unclaimed elsewhere | `git log --all --oneline --grep plan-135; ls planning planning/completed \| grep plan-135` → only this family | MET (2026-09-13, follow-plan re-run: `git log` → only `93500e25d`, the commit that wrote this family; `ls` → only the four plan-135-A..D files) |
| No `timezones` package exists | `ls -d packages/timezones` → `No such file or directory` | MET (2026-09-13, follow-plan re-run: `No such file or directory`) |
| The release compiler builds and tests a source package | `target/release/mfb init-pkg /tmp/p135 && target/release/mfb build -q /tmp/p135` → `Wrote package` | MET (2026-09-13, follow-plan re-run: `Wrote package to /tmp/p135/p135.mfp`) |
| The generator host has Python ≥ 3.9 (`zoneinfo`), `cc`, `make` | `python3 --version; cc --version; make --version` | MET locally (2026-09-13, follow-plan re-run: Python 3.14.5, Apple clang 17.0.0, GNU Make 3.81) |
| **Gates D only:** bug-520 closed, with the offset writer printing seconds | `ls bugs/completed/bug-520-*` → one file, **and** a probe printing `datetime::toIso(datetime::toLocal(datetime::instant(-3771144000, 0)))` under `TZ=America/New_York` → `1850-07-01T07:03:58.000-04:56:02` | NOT MET (2026-09-13, follow-plan re-run: `bugs/completed/` has no bug-520, `bugs/bug-520-datetime-is-not-correct-standalone.md` is open; the probe, built with the main checkout's release `mfb`, prints `1850-07-01T07:03:58.000-04:56`) |
| **Gates D's cross-target proof only:** boxes 2223, 2227, 2229 reachable | `for p in 2223 2227 2229; do ssh -o ConnectTimeout=8 -o BatchMode=yes -p $p test@127.0.0.1 true && echo $p ok; done` → three `ok` | MET (2026-09-13, follow-plan re-run: `2223 ok`, `2227 ok`, `2229 ok`) |

Everything below assumes the rows gating a letter hold before that letter starts. There
are no hedges for the world where they don't.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every command and update every status before you continue, and again before you decide
> to stop. **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- **Vendored release.** `third_party/tzdb/2026d/` holds the unmodified
  `tzdata2026d.tar.gz` and `tzcode2026d.tar.gz`, their IANA `.asc` signatures, and a
  `SHA256SUMS`. A `README.md` records the source URLs and the verification commands with
  their output.
- **Generator.** `python3 tools/tzdb/gen_timezones_data.py` writes
  `packages/timezones/src/data.mfb` to stdout, deterministically, and statistics to
  stderr. It verifies the checksums, compiles with the vendored `zic`, and fails closed
  on any input the later letters do not handle (§ 4.2).
- **Drift gate.** `scripts/check-generated.sh` checks that file.
- **Package skeleton.** `packages/timezones/` builds. Its tests prove that every one of
  the 598 names has data, that a link and its target share data, and that unknown input
  has none.

### Non-goals (family-wide; every letter inherits these)

- **Nothing under `src/`, `tests/` or `src/docs/` changes.** No builtin package, registry
  descriptor, `ZoneKind` variant or `Zone` field is added (owner ruling 2).
- **No part of any answer comes from the host.** No `/usr/share/zoneinfo`, no `TZ`, no
  `localtime`, no Windows time-zone API, and no fallback to them (owner ruling 1).
- **No value stores an index into the table.** A zone is identified by its name, so
  values stay meaningful across a table update.
- **No `RES`.** Every package type is a plain record, so zoned values stay
  thread-sendable.
- **The builtin `datetime` types are consumed unchanged.** The package builds
  `datetime::Zone[offset, 1, abbreviation]` snapshots.
- **No backzone data.** The default IANA build excludes `backzone`, and the `tzdata`
  wheel the oracle uses matches that set: 598 = 598, measured below.
- **No committed `.mfp`.** `git ls-files 'packages/*/*.mfp'` → empty (2026-09-13). The
  existing packages do not commit theirs.

## 2. Current State

- **No zone names or tzdb data exist in the tree.**
  `grep -rl 'America/New_York' src packages tests` → 2 files. Both are prose examples in
  `src/codegen/builtins/datetime/func_civil.rs` and `func_add_days.rs`.
  `ls third_party` → `unicode`, `utf8proc`.
- **`datetime` has three zone kinds** (`mfb man datetime types`): `Utc` = 0,
  `FixedOffset` = 1, `Local` = 2. `datetime::Zone` is
  `[offsetSeconds AS Integer, kind AS Integer, label AS String]`. The 2026-09-13
  standalone audit (bug-520) found its UTC, fixed-offset and host-local resolution
  correct on macOS and Linux box 2223. It also found offset text I/O defects, which gate
  letter D.
- **Generated-artifact precedent.** `scripts/check-generated.sh` defines `check
  <generator> <artifact>`: it runs `python3 <generator>` with stderr discarded and
  `cmp`s stdout against the artifact. CI runs it in `.github/workflows/coverage.yml` job
  `build` (ubuntu-latest, Python 3.14) before the Rust build. The script's
  `scripts/README.md` entry does not list artifacts, so adding a row needs no README edit
  (read 2026-09-13).
- **Package-test precedent.** `packages/jwt` keeps tests in `src/test_*.mfb` `TESTING`
  blocks, run by `mfb test packages/jwt`. No CI job runs any package's tests:
  `grep -rn "mfb test packages" .github/workflows` → no match. Package gates are run by
  hand and recorded, as `packages/jwt/oracle/README.md` does.

### Measured populations

These numbers are for IANA **2026d**, the latest release on 2026-09-13
(`curl -sSLO https://data.iana.org/time-zones/tzdata-latest.tar.gz` → `version` file
`2026d`). They were measured from the two tarballs extracted into one directory. Source
set `S = africa antarctica asia australasia europe northamerica southamerica etcetera
backward factory`.

| What | Count | Command |
|---|---|---|
| `Zone` lines | 345 | `awk '$1=="Zone"' $S \| wc -l` → 345 |
| `Link` lines | 253 | `awk '$1=="Link"' $S \| wc -l` → 253 |
| `Rule` lines | 1988 | `awk '$1=="Rule"' $S \| wc -l` → 1988 |
| Compiled names | 598 | `make zic && ./zic -b slim -d out $S && find out -type f \| wc -l` → 598 (0 symlinks) |
| Distinct compiled zones | 345 | `find out -type f -exec stat -f %i {} \; \| sort -u \| wc -l` → 345 |
| Names in `tzdata==2026.4` (IANA 2026d) | 598, set-equal | `zoneinfo.reset_tzpath([]); available_timezones()` vs `out` → 598 / 598, both differences empty |
| Stored transitions, slim | 17018 | throwaway RFC 8536 v2 parser over `out`, 64-bit block, deduplicated by inode |
| Stored transitions, fat | 23590 | same, over `zic -b fat` output |
| Local-time types, slim | 1598 | same parser |
| Distinct footers | 94 (30 with a DST rule, used by 106 zones) | same parser |
| DST rule date forms | 60 `Mm.w.d`, 0 `Jn`, 0 `n` | same parser |
| Rule times outside 0..24 h | 4 (`IST-2IDT,M3.4.4/26,M10.5.0`; `<-02>2<-01>,M3.5.0/-1,M10.5.0/0`) | same parser |
| Footers with quoted `<…>` designations | 44 | same parser |
| Zones with 0 stored transitions | 29 | same parser |
| Most transitions in one zone | 310 (Asia/Hebron) | same parser |
| Minimum spacing between consecutive transitions of a zone | 597,600 s (America/Cambridge_Bay, at 973400400) | same parser, slim and fat |
| Largest offset change at one transition | 86,400 s (Kwajalein, at 745934400) | same parser |
| Tarball sizes | tzdata 479,409 B; tzcode 328,712 B | `ls -la *.tar.gz` |
| Extracted data files | 856 KB | `du -ck $S version LICENSE` |
| `zic` sources | `zic.c` 4,334 lines; `zic.c private.h tzfile.h Makefile` 212 KB | `wc -l zic.c`; `du -ck …` |
| Probe `data.mfb`, `MATCH` dispatch (§ 4.3 format, 20 first-letter buckets) | 279,910 B; payload 237,696 chars | throwaway generator `/tmp/tzmeasure/gen_probe_match2.py` |
| Probe package build / `.mfp` / test | 0.05 s real; 363,531 B; `mfb test` 0.46 s | `/usr/bin/time -p mfb build -q .`, `mfb test .` |
| Consumer executable (`IF`-chain probe) | 545,452 B vs 66,600 B `io`-only baseline (+478,852) | `mfb build` of a consumer importing the probe `.mfp` |
| Lookup cost | 10,000 decode-and-search lookups on Asia/Hebron: 0.50 s real, whole process | `/usr/bin/time -p ./build/tzconsumer.out` |

The transition, type, footer and spacing counts came from a throwaway parser. Phase 2's
generator prints the same counts on stderr, and its acceptance requires them to match
this table. That match is the independent re-measurement.

### Verified properties

- **`PRIVATE` is file-scoped; a bare `FUNC` is package-visible.** A probe calling a
  `PRIVATE FUNC zoneData` from another file failed with "Callable `zoneData` is not a
  top-level function". Changing it to bare `FUNC` built.
- **Imports do not cross files.** A generated file using `strings::left` without its own
  `IMPORT strings` fails `2-201-0014 SYMBOL_UNKNOWN_IMPORT`.
- **Comma-separated `MATCH` arms scale to the whole table.** `CASE "a", "b" : …` over 598
  string literals in 20 functions builds, and the dispatch test passes (probe above).
  `strings::left("", 1)` returns `""` (probe 2026-09-13). The generated dispatcher still
  guards `len(name) = 0` explicitly.
- **A consumer can use `datetime` values without importing `datetime`.** A consumer
  importing only `io` and the probe package held a `datetime::DateTime` returned by the
  package (`LET dt = pkg::sample()`), read `dt.offset`, and passed it back. It printed
  `-14400 2026-07-15T09:00:00.000-04:00`. Letter D's link smoke relies on this.
- **`tzcode`'s `make zic` builds on the host.** It needs the data and code tarballs
  extracted into one directory. Extracted apart, it failed with "No rule to make target
  `africa`".

## 3. Design Overview

Four letters, one per concern, in dependency order:

1. **A — data.** Vendored tarballs → `zic -b slim` in a temp dir → RFC 8536 parse →
   `data.mfb`.
2. **B — `offsetAt`/`toZone`.** RFC 8536 §3.2 lookup, the POSIX TZ footer evaluator, and
   the `zoneinfo` oracle.
3. **C — `civil`.** Wall clock → instant with compatible disambiguation, plus the oracle
   `civil` mode.
4. **D — RFC 9557 `toIso`/`parseIso`.** Also the link smoke, cross-target proof, docs and
   the final gate.

**Design uncertainty** was whether an MFBASIC package can carry the table at all. It was
measured before this plan was written: the table builds in 0.05 s, the `.mfp` is 363,531
bytes, and a lookup costs 50 µs. No experiment is left to schedule. Phase 3 re-checks the
build on the real generated file.

**Correctness risk** concentrates in B's footer evaluator. Slim output means every
current-date answer for 106 zones comes from it. D's inconsistent-offset handling is
second. This letter's risk is only reproducibility, and the drift gate plus a second-host
regeneration covers it.

**Gate class:** new code and new data. There is exactly one byte-identity gate in the
family: the regenerated `data.mfb` must equal the committed one. That is the right gate
for a generated artifact. A diff there is a generator or input change to explain, never
a baseline to accept.

Rejected:

- **Host tzdb** (owner ruling 1) and **a builtin package** (owner ruling 2).
- **Vendoring compiled TZif files instead of the release source.** They are not the
  artifact IANA signs, and a reviewer cannot diff a rule change in them.
- **The `tzdata` PyPI wheel as generator input.** The oracle (B) reads that wheel. Using
  it here would give the generator and the oracle the same compiled bytes.
- **Parsing the tzdb rule language in Python.** That re-implements `zic`. The release's
  own `zic` is the reference implementation.
- **`zic -b fat`.** It has 23590 transitions against 17018, and still needs the footer
  after 2037 (see plan-135-B § 3).
- **One `IF` chain or one giant `MATCH` over 598 names.** 20 first-letter buckets keep
  every function small. Oversized functions overflow the ±1 MiB branch range and stress
  the optimizer, and a first-letter split costs nothing.

## 4. Detailed Design

### 4.1 Vendored input — `third_party/tzdb/2026d/`

- `tzdata2026d.tar.gz`, `tzdata2026d.tar.gz.asc`, `tzcode2026d.tar.gz` and
  `tzcode2026d.tar.gz.asc`, byte-for-byte from
  `https://data.iana.org/time-zones/releases/`.
- `SHA256SUMS`, written by `shasum -a 256 tzdata2026d.tar.gz tzcode2026d.tar.gz`.
- `third_party/tzdb/README.md`: the release, the URLs, the `gpg --verify` commands and
  their observed output (signer and fingerprint), the licence (the tz data and code are
  public domain; quote the `LICENSE` file), and a pointer to `tools/tzdb/README.md` for
  updates.

### 4.2 Generator — `tools/tzdb/gen_timezones_data.py`

Constants: `RELEASE = "2026d"`, and `SOURCES` = the § 2 source set `S`.

1. **Verify.** Hash both tarballs with `hashlib.sha256` and compare with `SHA256SUMS`.
   Any mismatch → exit 1 naming the file.
2. **Build `zic`.** Extract both tarballs into one `tempfile.TemporaryDirectory()` and run
   `make zic` there (`subprocess.run(check=True, env={**os.environ, "LC_ALL": "C"})`,
   output to stderr).
3. **Compile.** Run `./zic -b slim -d <tmp>/out <SOURCES…>`.
4. **Parse every file** under `out` per RFC 8536:
   - require the `TZif` magic and version ≥ `2`;
   - skip the v1 block using its header counts, and read the v2 header;
   - require `leapcnt = 0` and `typecnt ≥ 1`;
   - read the 64-bit transition times, the type indices, the `ttinfo` records
     `(utoff int32, isdst uint8, desigidx uint8)` and the designation bytes;
   - read the footer between the final newlines.
5. **Group names by identical file bytes, not by inode.** A name's group is the set of
   names with byte-identical TZif. Group order is by the lexicographically first name;
   names within a group are sorted.
6. **Fail closed (exit 1 with the zone and value) unless all of these hold:**
   - every footer DST date is `Mm.w.d`;
   - every footer that names a DST designation has a `,start,end` rule;
   - no designation contains a character outside `[A-Za-z0-9+-]`;
   - no footer contains `|` or `;`;
   - every `|utoff| < 172800`;
   - consecutive transitions in each zone are more than 345,600 s apart;
   - no name contains `"` or `\`.

   These are exactly the premises plan-135-B § 4.1 and plan-135-C § 4.1 rely on.
7. **Emit to stdout** in § 4.3's format.
8. **Print one statistics line to stderr:**
   `names 598 distinct 345 transitions 17018 types 1598 footers 94`.

`tools/tzdb/README.md` covers what the generator does, its inputs and outputs, the run
command, and the **update procedure**:

1. Download the new release's four files.
2. Run `gpg --verify` on both signatures and record the result.
3. Replace `third_party/tzdb/<release>/` and `SHA256SUMS`.
4. Bump `RELEASE`.
5. Regenerate `data.mfb`.
6. Bump `packages/timezones/oracle/requirements.txt` to the matching `tzdata` wheel
   (2026d = `2026.4`: the letter's ordinal).
7. Run `oracle/run.sh`.
8. Bump `project.json` `version`.

### 4.3 Generated file — `packages/timezones/src/data.mfb`

```
' GENERATED by tools/tzdb/gen_timezones_data.py from third_party/tzdb (IANA tzdb 2026d).
' Do not edit: `sh scripts/check-generated.sh` fails on any hand change.
IMPORT strings

FUNC tzdbVersion() AS String
  RETURN "2026d"
END FUNC

FUNC zoneNames() AS List OF String
  RETURN ["Africa/Abidjan", …]                 ' all 598, sorted
END FUNC

FUNC zoneData(name AS String) AS String         ' "" for an unknown name; case-insensitive
  IF len(name) = 0 THEN RETURN ""
  LET key AS String = strings::lower(name)
  MATCH strings::left(key, 1)
    CASE "a" : RETURN zonesA(key)
    …
    CASE ELSE : RETURN ""
  END MATCH
END FUNC

PRIVATE FUNC zonesA(key AS String) AS String
  MATCH key
    CASE "africa/abidjan", "africa/accra", … : RETURN zone0()
    …
    CASE ELSE : RETURN ""
  END MATCH
END FUNC

PRIVATE FUNC zone0() AS String
  RETURN "<types>|<transitions>|<footer>"
END FUNC
```

The zone string format is plan-135-B's decoding contract:

- `types` = `utoff,isdst,abbr` records joined by `;`, in TZif order;
- `transitions` = `unixSeconds,typeIndex` records joined by `;`, ascending, empty when
  there are none;
- `footer` = the TZif footer, possibly empty.

Integers are decimal with an optional leading `-`.

**Name matching ignores case** (owner decision 2026-09-13). The generator emits every
`MATCH` literal lower-cased, and `zoneData` looks up `strings::lower(name)`. It also emits
`FUNC canonicalName(name AS String) AS String`, built the same way: it returns the
tzdb spelling (`"america/new_york"` → `"America/New_York"`, `"us/eastern"` →
`"US/Eastern"`, since a link stays a link), or `""` for an unknown name. The generator
fails closed if two names are equal ignoring case.

If `zoneNames()` as one literal list of 598 strings fails to build, split it by bucket
and concatenate. Record which happened in Corrections.

### 4.4 Package skeleton

- **`packages/timezones/project.json`.** `"kind": "package"` and `"version": "0.1.0"`,
  with sources `src/**/*.mfb`, as in `packages/jwt/project.json`. Its description says:
  named IANA zones from a committed tzdb release, never the host's zone database.
- **`packages/timezones/src/lib.mfb`.** A header comment in jwt's style, plus
  `DOC PACKAGE`. The `DESC` covers what the package answers, that the rules are
  vendored tzdb 2026d, that it never consults the host, and that
  `datetime::local()` is the host-scoped alternative. It has no `EXPORT` yet; letters
  B–D add them.
- **`packages/timezones/README.md`.** Title and one-paragraph purpose, "Where the rules
  come from" (release, vendoring, gate, update procedure link), and "What it never does"
  (read the host's zone database). Later letters add a section per member.
- **`packages/timezones/.gitignore`.** `build/` and `*.mfp`, if the root `.gitignore`
  does not already cover them. Check with `git check-ignore -v`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` partial with what remains; strike moot tasks with evidence, never
> delete; fill `Commit:` when a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — vendor 2026d

Data files only. Safe alone: nothing reads them yet.

- [ ] Download the four release files into `third_party/tzdb/2026d/`, then write
      `SHA256SUMS`.
- [ ] Verify both signatures with `gpg --verify` against the tz maintainers' key. Record
      the exact output lines in `third_party/tzdb/README.md`. If the key cannot be
      fetched on this host, that is a blocker to report. Never skip verification.
- [ ] Write `third_party/tzdb/README.md` (§ 4.1).

Acceptance: the committed bytes are the signed IANA release.
  Check: `cd third_party/tzdb/2026d && shasum -a 256 -c SHA256SUMS` → two `OK` lines; `gpg --verify tzdata2026d.tar.gz.asc tzdata2026d.tar.gz` and the tzcode pair → `Good signature` each (est. 2 min).
Commit: —

### Phase 2 — generator and drift gate

- [ ] `tools/tzdb/gen_timezones_data.py` per § 4.2, emitting § 4.3.
- [ ] `tools/tzdb/README.md` per § 4.2, including the update procedure.
- [ ] Generate `packages/timezones/src/data.mfb`:
      `python3 tools/tzdb/gen_timezones_data.py > packages/timezones/src/data.mfb`.
- [ ] `scripts/check-generated.sh`: add
      `check tools/tzdb/gen_timezones_data.py packages/timezones/src/data.mfb` with a
      comment naming plan-135-A and the reason (nobody reviews 17018 transitions by
      eye).
- [ ] Mutation proof for the gate: append one space to `data.mfb`, run
      `sh scripts/check-generated.sh`, confirm `DRIFT` and exit 1, then revert.
- [ ] Mutation proof for the fail-closed checks: temporarily lower the spacing threshold
      in a *copy* of the generator to 600,000, confirm it exits 1 naming
      America/Cambridge_Bay, and discard the copy.
- [ ] Cross-host determinism. On box 2223, native aarch64 Linux: copy
      `tools/tzdb/gen_timezones_data.py` and `third_party/tzdb/2026d/`, run the
      generator, and `cmp` against the committed `data.mfb`. The CI gate runs on Linux
      and this file is generated on macOS, so a platform-dependent `zic` or `make` would
      otherwise surface only as a red CI row. If 2223 lacks `cc`/`make`, record that and
      use the first box that has them.

Acceptance: the generator reproduces the measured populations, the gate accepts the
committed file and rejects a changed one, and a second OS produces identical bytes.
  Check: `python3 tools/tzdb/gen_timezones_data.py 2>&1 >/dev/null` → `names 598 distinct 345 transitions 17018 types 1598 footers 94`; `sh scripts/check-generated.sh; echo EXIT=$?` → the `ok: packages/timezones/src/data.mfb …` line and `EXIT=0`; on 2223 `cmp` → no output, exit 0 (est. 5 min).
Commit: —

### Phase 3 — package skeleton and data tests

- [ ] `packages/timezones/project.json`, `src/lib.mfb`, `README.md` and `.gitignore` as
      needed, per § 4.4.
- [ ] `packages/timezones/src/test_data.mfb`, with these cases:
  - [ ] `len(zoneNames())` is 598, and every name has non-empty `zoneData`.
  - [ ] `zoneData("US/Eastern") = zoneData("America/New_York")`.
  - [ ] `zoneData("Etc/UTC") = zoneData("Zulu")`.
  - [ ] `zoneData` is `""` for `"Nowhere/Bogus"`, `""` and `"America/New_York "`.
  - [ ] `zoneData("america/new_york") = zoneData("America/New_York")` and
        `zoneData("AMERICA/NEW_YORK") = zoneData("America/New_York")`.
  - [ ] `canonicalName("america/new_york")` is `"America/New_York"`,
        `canonicalName("us/eastern")` is `"US/Eastern"`, and
        `canonicalName("Nowhere/Bogus")` is `""`; every name in `zoneNames()` is its
        own `canonicalName`.
  - [ ] Every returned string has exactly two `|` separators.
  - [ ] `tzdbVersion()` is `"2026d"`.
- [ ] Record the real `mfb build -q packages/timezones` time and `.mfp` size in
      Corrections if they differ materially from the probe's 0.05 s / 363,531 B.

Acceptance: every tzdb name reaches data through the generated dispatcher, and nothing
else does.
  Check: `target/release/mfb build -q packages/timezones && target/release/mfb test packages/timezones` → `Wrote package`, then `Fail: 0` with the `data` group present (est. 1 min).
Commit: —

## Validation Plan

- **Tests:** `test_data.mfb` (Phase 3). Letters B–D add theirs.
- **Coverage check:** the gate mutation and the fail-closed mutation (Phase 2) show that
  both protections can fail.
- **Runtime proof:** none for this letter. The data has no public member until B.
- **Doc sync:** `third_party/tzdb/README.md`, `tools/tzdb/README.md`,
  `packages/timezones/README.md`.
- **Final gate:** plan-135-D § Validation Plan, run once at the end of the family.

## Open Decisions

- **Historical range.** Recommend **full history (LMT onward)**, as compiled. A cut-off
  silently answers old instants wrongly, and it saves little: the whole table is 237,696
  payload characters.
- **Vendor the tarballs vs extracted files.** Recommend **tarballs, unmodified**. They
  are the signed artifact, so `gpg --verify` is meaningful. Extracted files would cut
  roughly 800 KB of committed binary at the cost of that verification.

## Corrections

## Summary

This letter is plumbing with one sharp edge: it must be reproducible. The gate, the
second-host `cmp`, and fail-closed generator checks for every premise letters B and C
rely on cover that. Nothing outside `third_party/tzdb/`, `tools/tzdb/`,
`packages/timezones/` and one row in `scripts/check-generated.sh` changes.
