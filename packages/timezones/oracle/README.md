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

## Proof that it can fail

A green run means nothing unless a broken package turns it red. Each mutation below was
applied to a throwaway copy of the package, and the mode that should catch it was run:

| Mutation | Mode | Mismatches |
|---|---|---|
| `footerType` always returns the standard type | `offsets` | 61,559 of 498,303 |
| `civil` picks the later instant in an overlap | `civil` | 67,773 of 263,558 |
| `civil` picks the post-transition offset in a gap | `civil` | 68,457 of 263,558 |

The first mutation's mismatches start at each zone's last stored transition. For
example, `America/Chicago 1173600000` gets `-18000 CDT` from the oracle but
`-21600 CST` from the mutant. So the corpus does reach the footer evaluator, which
answers every instant from a zone's last stored transition on. The two `civil`
mutations fail on real historical gaps and overlaps, such as Africa/Algiers in 1916
and Africa/Abidjan in 1912. Each disambiguation branch is therefore exercised by
the corpus.
