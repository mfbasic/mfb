# synthetic-grib-fixture

Generates `examples/weather/tests/fixtures/valid-analysis.grb2` — the
"known-good cached file" fixture bug-711's regression test
(`examples/weather/tests/cache_corruption_test.sh`) plants at a synthetic
cycle's cache path, to prove a valid cached file still decodes and is kept
(as opposed to the corrupt-magic-prefixed fixture the same test synthesizes
inline).

## Why a generator, not a captured real file

The fixture originally committed here was a byte-for-byte copy of a real,
previously-fetched GFS analysis hour: 926 KB, because a real half-degree
global grid (720x361 points, four fields) costs that much even with GRIB2's
own packing. `examples/weather/src/grib.mfb:decodeFile` only needs a file that
satisfies its own decode rules — it does not care whether the grid is
720x361 or 1x1 — so a synthetic message with a single grid point per field
decodes exactly the same way and is enough to prove the thing this fixture
exists to prove: `windsFrom`'s eight `pick()` calls (UGRD and VGRD at 10 m,
850 mb and 250 mb, TMP 2 m, PRATE surface) all find their field and decode successfully.

`generate.py` builds eight minimal, spec-legal GRIB2 messages (section 0, a
minimal section 1, section 3 grid template 3.0 at 1x1, section 4 product
template 4.0 with the right category/parameter/surface/height, section 5
data representation template 5.0 with bit width 0 — a constant field, so
section 7 needs no packed data at all, only its header — section 6's
all-present bitmap, and the "7777" terminator), one per field
`windsFrom`/`pick` requires. The result is 1248 bytes, decodes with the real
`examples/weather/src/grib.mfb` unchanged, and is small enough to commit without
the "why is there a megabyte binary in this test fixture" question a
captured real file invites.

Checked before choosing this over keeping the real file: no smaller or
already-in-tree GRIB2 asset exists anywhere in the repository
(`find . -iname '*.grb2'` — only this fixture); the real captured file's four
messages (one per required field) were already the minimum message count, so
the only way to shrink it further was a smaller grid, which meant an
encoder.

## Regenerating

```
$ python3 tools/synthetic-grib-fixture/generate.py examples/weather/tests/fixtures/valid-analysis.grb2
wrote 1248 bytes to examples/weather/tests/fixtures/valid-analysis.grb2
```

Verify it still decodes with the real decoder by re-running bug-711's
regression test, which plants it at a synthetic cache path and asserts a
successful decode:

```
$ ./examples/weather/tests/cache_corruption_test.sh target/release/mfb
```

Only regenerate this if `grib.mfb`'s decode rules change in a way that make
the existing fixture stop decoding (a new required section, a stricter
check, …), or `windsFrom` starts requiring another field (the 850 and 250 mb
winds did) — the fixture itself does not need to track `examples/weather/src`
day to day, since it exists to be *a* valid file, not *the current* real
NOAA output.
