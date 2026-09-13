# oracle — differential-test `packages/timezones` against Python `zoneinfo`

The package's own tests (`mfb test packages/timezones`) pin hand-picked answers. They
cannot catch a misreading of RFC 8536 or of the POSIX TZ rules, and a bug in a zone
nobody thought to test goes unseen. This directory asks the same questions of an
independent implementation, over every transition of every zone.

```sh
./run.sh                               # every mode, with ../../../target/release/mfb
./run.sh path/to/mfb                   # every mode, with a compiler you name
./run.sh '' offsets                    # only some modes
```

Exit status is 0 iff every mode agreed, apart from divergences declared in
`divergences.json` (currently none).

## The oracle, and why it is independent

Python's standard-library [`zoneinfo`](https://docs.python.org/3/library/zoneinfo.html)
reads the compiled TZif files in the [`tzdata`](https://pypi.org/project/tzdata/) wheel,
pinned in `requirements.txt` to **2026.4**, which is IANA **2026d**.

- **Different code.** `zoneinfo` is CPython's own TZif reader and POSIX TZ evaluator.
  It shares no code with `src/zone.mfb` or `src/posix.mfb`.
- **Same release, different build.** The wheel's files were compiled by the `tzdata`
  maintainers, not by `tools/tzdb/gen_timezones_data.py`. So the oracle does not ratify
  whatever the generator produced.
- **Never the host.** `oracle.py` and `corpus.py` call `zoneinfo.reset_tzpath([])`
  before any lookup. `oracle.py` refuses to run unless `tzdata.IANA_VERSION` equals the
  generator's `RELEASE`.

Bump `requirements.txt` whenever the vendored release changes. The wheel's minor number
is the release letter's ordinal, so 2026d is `2026.4`. See the update procedure in
`tools/tzdb/README.md`.

## How the two sides talk

`corpus.py <mode>` writes a job file, one question per line. `oracle.py` answers it
with `zoneinfo`. `probe/` is an MFBASIC executable that imports the built
`timezones.mfp` and answers the same file. `diff.py <mode>` compares the two answer
files line by line.

| Mode | Job line | Answer line |
|---|---|---|
| `offsets` | `offset <name> <unixSeconds>` | `<utoff> <abbreviation>` from `timezones::toZone` |
| `civil` | `civil <name> Y M D h m s` | `<utcSeconds> <utoff> <abbreviation> <wall>` from `timezones::civil`; `zoneinfo` uses `fold=0` |
| `roundtrip` | `roundtrip <name> <unixSeconds>` | probe only: `ok` when `parseIso(toIso(v, 9, name))` gives back the same seconds, nanos, offset and name |
| `ixdtf` | `ixdtf <text>` | `accept <seconds> <nanos> <utoff> <zone>` or `reject`; the reference is Temporal, not `zoneinfo` |

### `roundtrip` corpus

Every `offsets` instant, with 123456789 nanoseconds added. `v` is
`datetime::inZone(instant, timezones::toZone(name, instant))`. The nanoseconds are not
zero, so a writer that drops them fails every job.

### `ixdtf` corpus — the syntax cross-check

The reference is the JavaScript Temporal API in Node 24
(`node --harmony-temporal temporal.mjs`). `package.json` pins nothing beyond
`"node": ">=24"`. Temporal's rules come from the ICU tz data bundled with Node, which
is **tz 2025b**, older than the package's 2026d. So the corpus only asks about text:

1. `corpus.py ixdtf` writes candidates. For every name: 09:00 UTC on the 1st of
   January, April, July and October 2026–2030, in the valid form. For 2026-07-01, a set
   of mutations: a dropped bracket, an uppercased key, a critical unknown key, an
   elective unknown key, a duplicated zone, a numeric zone, trailing text, a mismatched
   offset, `Z`, `+00:00`, a critical zone, `u-ca=iso8601`, `u-ca=gregory`, and a
   lowercased name. For three zones in 1850, local mean time written exactly and
   rounded to the minute.
2. `temporal.mjs filter` keeps a candidate only when Temporal's tz gives the zone the
   same offset at that instant as 2026d does. Every skip is logged to
   `jobs/ixdtf.skipped` with its reason.
3. `temporal.mjs answer` and the probe answer the kept jobs.

`divergences.json` declares, with reasons, every way the two sides differ by design:

- a critical zone annotation (Temporal rejects, the package accepts);
- an elective unknown tag (Temporal rejects, the package ignores it per RFC 9557 §3.3);
- a non-ISO calendar (Temporal accepts, the package refuses);
- a numeric zone annotation (Temporal accepts one that agrees with the offset, and
  numeric annotations are a non-goal here);
- a lowercased name containing a digit (Temporal rejects, the package ignores case);
- a link or recased name that resolves to the same instant and offset (Temporal
  reports the link's target, and the package keeps the link in tzdb spelling).

A refusal is an answer (`error <code>`), and the probe still exits 0. A non-zero probe
exit means the probe broke, and `run.sh` fails on it rather than counting mismatches.

The job inputs may come from the package's table: `corpus.py` reads `src/data.mfb` to
find the transition instants. Every answer comes from `zoneinfo`, so a wrong table
still produces jobs that catch it.

### `offsets` corpus

1. Every stored transition `t` of every distinct zone, at `t−1`, `t` and `t+1`.
2. For every name whose 2030 January and July offsets differ: every offset change from
   2026 through 2100, found by a daily scan and bisected to the second, at `c−1`, `c`
   and `c+1`. This is the only coverage of every DST footer rule.
3. For every one of the 598 names: 00:00 UTC on 1 January and 1 July of every year
   1800–2100.
4. For every name: the earliest instant Python can convert (0001-01-03T00:00Z), and its
   first stored transition − 86400.

### `civil` corpus

1. For every transition in sections 1 and 2 above, with offsets `oPrev → oNew`, the wall
   readings `t+oPrev−1`, `t+oPrev`, `t+oNew−1`, `t+oNew`, and the midpoint of the gap
   or overlap.
2. 09:00 on the 15th of every month 2026–2030, for every name.

## Measured (2026-09-13, macOS aarch64, `run.sh <mfb> offsets civil`)

| Mode | Jobs | Mismatches | Wall time (corpus + oracle + probe + diff) |
|---|---|---|---|
| `offsets` | 498,303 | 0 | 18 s |
| `civil` | 263,558 | 0 | 37 s |
| `roundtrip` | 498,303 | 0 | 37 s |
| `ixdtf` | 19,652 (of 20,300 candidates; 89 skipped) | 0 (5,341 declared divergences) | 6 s |

## Same answers on every target (plan-135-D § 4.5, 2026-09-13)

`oracle/probe` was cross-built on macOS for each Linux target, each in its own project
copy, because a second `--target` build replaces the first one's binaries. Each box
ran the `offsets` and `civil` jobs, and its answer file was `cmp`ed with the macOS
probe's answers to the same lines. Those macOS answers already agree with `zoneinfo`.

- **Native aarch64 box (2223):** the full corpus.
- **Emulated boxes (2227, 2229):** the per-zone sample, every job line whose name is the
  first name of one of the 345 distinct zones. That is 306,357 `offsets` and 183,128
  `civil` jobs. It reaches every footer rule and every stored transition.

| Box | Target | Binary | Set | `offsets` | `civil` |
|---|---|---|---|---|---|
| 2223 | linux-aarch64 (native) | glibc | full | identical, 498,303 jobs, 7 s | identical, 263,558 jobs, 31 s |
| 2229 | linux-riscv64 (emulated) | musl | per-zone sample | identical, 306,357 jobs, 69 s | identical, 183,128 jobs, 361 s |
| 2227 | linux-x86_64 (emulated) | musl | per-zone sample | identical, 306,357 jobs, 324 s | identical, 183,128 jobs, 570 s |
| — | windows-x86_64 | `tzprobe.exe` | — | built only | built only |

**Windows is built, never run.** `mfb build --target windows-x86_64` wrote
`tzprobe.exe` (2,569,728 B). No Windows box has an execution harness, so no Windows
answers exist to compare.

Box 2223 has no musl loader (`./tzprobe-musl.out: cannot execute: required file not
found`), so it ran the glibc binary. `/tmp/p135box.sh` tries musl and falls back to
glibc, and it records which one ran.

## Proof that it can fail

A green run means nothing unless a broken package turns it red. Each mutation below was
applied to a throwaway copy of the package, and the mode that should catch it was run:

| Mutation | Mode | Mismatches |
|---|---|---|
| `footerType` always returns the standard type | `offsets` | 61,559 of 498,303 |
| `civil` picks the later instant in an overlap | `civil` | 67,773 of 263,558 |
| `civil` picks the post-transition offset in a gap | `civil` | 68,457 of 263,558 |
| `parseIso` skips the offset-against-zone check | `ixdtf` | 1,234 of 19,652 (the unmutated package had 74 then, all numeric annotations since declared) |
| `toIso(dt, digits, name)` always writes 3 digits | `roundtrip` | 498,303 of 498,303 |

The first mutation's mismatches start at each zone's last stored transition. For
example, `America/Chicago 1173600000` gets `-18000 CDT` from the oracle but
`-21600 CST` from the mutant. So the corpus does reach the footer evaluator, which
answers every instant from a zone's last stored transition on. The two `civil`
mutations fail on real historical gaps and overlaps, such as Africa/Algiers in 1916
and Africa/Abidjan in 1912. Each disambiguation branch is therefore exercised by
the corpus.
