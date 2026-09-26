# bug-711: `wind` trusts a cached GRIB file on its magic bytes alone, and a corrupt one takes down the whole analysis

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Fixed

STATUS: FIXED (d437854b9)

`Attempt` now carries the decoded `Winds` directly (the Open Decisions'
recommended option), not raw bytes — `windsFrom` is called exactly once per
candidate, inside `cycleData`, covered by `attemptCycle`'s trap. The other
Open Decision (defer `writeBytesAtomic` until after a successful decode) was
also taken as recommended. No other deviation from the Fix Design.

Regression Test: `examples/wind/tests/cache_corruption_test.sh` (new; see Validation Plan)

`examples/wind` caches each fetched GRIB2 file under the host temp directory and,
on a later run, trusts a cached file as "the answer for this cycle/hour" the
moment its first four bytes read `GRIB` — no length check beyond "more than 16
bytes", no checksum, and no attempt to decode it before trusting it. The GRIB2
decode itself (`windsFrom`/`decodeFile`) runs **outside** the one place that
catches per-cycle failures (`attemptCycle`), both for the opening analysis
(`loadAnalysis`) and for the five-day forecast (`fetchForecast`). So a cached
file that starts with the magic bytes but is truncated or otherwise
undecodable — e.g. a disk-full write, a bug in a future format edition, media
corruption — is not treated as "this cycle failed, try the next one" the way an
unpublished or unreachable cycle is. It aborts `loadAnalysis` outright (shown to
the user as "No forecast to draw", process exits 1) with **no fallback to an
older cached cycle and no fallback to a fresh fetch**, even though
`loadAnalysis`'s whole reason for existing is exactly that fallback loop. The
same file is never invalidated, so every subsequent run hits the identical
corrupt file and fails identically until a human deletes the cache directory by
hand.

**Correct behavior:** a cache hit that turns out to be undecodable is exactly as
recoverable as a cycle that hasn't been published yet — it should count as this
cycle's attempt failing, let `loadAnalysis` walk back to the previous cycle (or
refetch), and the bad file should not be left in place to poison every later run.

References:

- `examples/wind/src/fetch.mfb` — the cache scheme, `attemptCycle`, `loadAnalysis`.
- `examples/wind/src/forecast.mfb` — `fetchForecast`, the forecast worker.
- `src/docs/spec/language/08_error-model.md` §8.3 — function-level `TRAP` semantics
  (confirms `attemptCycle`'s trailing `TRAP(e)` genuinely catches `cycleData`'s
  errors; this is not a dead-code/parse issue, the gap is that `windsFrom` is
  called from a different, untrapped function).

## Failing Reproduction

Built a standalone driver project (`/tmp/wind-repro`, scratch only — not part of
the repo) containing unmodified copies of `examples/wind/src/fetch.mfb` and
`src/grib.mfb`, plus a trivial stand-in `TYPE Winds` (to avoid pulling in
`flow.mfb`'s `canvas`/`color` GUI imports for a headless probe) and a `SUB
main()` that calls the real, unmodified `loadAnalysis()`:

```basic
SUB main()
  LET a AS Analysis = loadAnalysis() TRAP(e)
    io::print("loadAnalysis FAILED: code=" & toString(e.code) & " message=" & e.message)
    EXIT SUB
  END TRAP
  io::print("loadAnalysis OK: cycle=" & a.cycle.caption & " label=" & a.winds.label)
END SUB
```

Built with `mfb build /tmp/wind-repro` (succeeded). The host's real
`fs::tempDirectory()/mfb-wind-gfs` cache already held a valid, complete
2026-09-25 12Z analysis (`gfs-20260925-12-f000-10m-0p50.grb2`, sha256
`158c69e4…f52f3b6`) from an earlier real run of the app on this machine — a
plain `loadAnalysis()` run confirmed it reads that cache with no network access
under `sandbox-exec -f deny-net.sb` (a network-deny sandbox profile), so the
baseline is a clean cache hit, not a fetch.

To reproduce the bug without ever contacting NOMADS: the existing valid
`f000` cache file was backed up (`sha256` verified identical), then
overwritten in place with the literal bytes `GRIB` followed by 2000 bytes of
`/dev/urandom` (a plausible-but-corrupt size, well past the `len > 16` check):

```
$ printf 'GRIB'; head -c 2000 /dev/urandom > gfs-20260925-12-f000-10m-0p50.grb2
$ sandbox-exec -f deny-net.sb /tmp/wind-repro/build/wind-repro.out
```

- Observed (verbatim, captured today, 2026-09-25):
  ```
  loadAnalysis FAILED: code=77050007 message=grib: this is edition 151, and only edition 2 is read here
  ```
  Exit code 0 in this driver only because the driver's `TRAP` mimics
  `main.mfb:368`'s handler with `EXIT SUB` instead of `RETURN 1`; the real
  program's identical handler (`main.mfb:368-372`) shows the "No forecast to
  draw" notice with this same `e.message` and returns exit code 1 — a total
  failure of the run, with `loadAnalysis`'s eight-cycle retry loop (`fetch.mfb:173-178`)
  never reaching `back = 1`.
- Expected: the corrupt cache for the newest cycle should be treated the same
  as "this cycle isn't up yet" — `loadAnalysis` retries `back = 1` (the
  previous 06Z cycle), which is on disk and valid, and returns that analysis
  instead of failing the whole load.

The corrupted file was restored from the verified backup immediately after
capturing this output (`sha256` re-verified identical to the pre-corruption
file); no file under the shared cache directory was left modified or orphaned.

**Contrast cases** (same driver, a synthetic `2000-01-01T00Z` cycle chosen so it
can never collide with a real cached file, network denied by the same sandbox
profile so no external service is actually contacted):

- No cached file at all for that cycle/hour: `attemptCycle` returns
  `problem="TLS handshake, certificate validation, SNI validation, or protocol
  operation failed."` (the sandbox refusing the outbound connection) — a clean,
  retryable `Attempt.problem`, decode never attempted.
- A cached file present but **not** starting with `GRIB` (`NOTGRIBDATA_______________________`):
  identical outcome — `cycleData`'s magic check correctly rejects it and falls
  through to the same fetch attempt, same clean `problem` string.

Both contrast cases bound the bug precisely: the magic-byte check itself does
correctly reject a non-GRIB file and correctly routes that case through the
retryable `Attempt.problem` path. The hole only opens once the four magic bytes
happen to match — from that point on there is no further validation, and if
what follows doesn't decode, the failure is no longer retryable.

## Root Cause

`examples/wind/src/fetch.mfb:cycleData` (lines 136-149) is the cache-validity
check:

```
136 FUNC cycleData(c AS Cycle, hour AS Integer) AS List OF Byte
137   LET path AS String = cachePath(c, hour)
138   IF fs::fileExists(path) THEN
139     LET cached AS List OF Byte = fs::readBytes(path)
140     IF len(cached) > 16 AND beInt(cached, 0, 4) = GRIB_MAGIC THEN RETURN cached
141   END IF
```

Line 140 is the entirety of the cache-trust decision: length over 16 bytes and
the first 4 bytes equal `GRIB_MAGIC` (`1196575042`). There is no length upper
bound, no stored/expected size, no checksum, and — critically — no attempt to
decode the file before returning it as trusted data. `cycleData` is called only
from `attemptCycle` (line 151-157), which does carry a function-level `TRAP(e)`
that genuinely catches every error `cycleData` can raise (per
`08_error-model.md` §8.3, a function-level `TRAP` "traps every error from the
body" — this is the same idiom as `main.mfb:nextChar`, not dead code). That
trap is why an unpublished cycle (`cycleData` line 145's `FAIL`) or a network
failure inside `http::read` (line 143) becomes a recoverable `Attempt.problem`
rather than an aborted load.

The gap is that the actual GRIB2 decode is **not** done inside `cycleData` or
`attemptCycle`. It happens in `windsFrom` (`fetch.mfb:162-165`), which calls
`decodeFile` (`grib.mfb:418`) — the function that would actually notice a
corrupt body (edition byte, section lengths, template numbers, etc., each
guarded by its own `FAIL error(errorCode::ErrInvalidFormat, …)` in `grib.mfb`).
`windsFrom` is called from two places, both **outside** `attemptCycle`'s trap:

- `fetch.mfb:176`, inside `loadAnalysis`'s retry loop (`fetch.mfb:169-181`):
  `IF len(tried.data) > 0 THEN RETURN Analysis[cycle := c, winds :=
  windsFrom(tried.data, c)]`. `loadAnalysis` itself has no function-level
  `TRAP`, so a `windsFrom` failure propagates straight out of the `FOR back = 0
  TO CYCLES_TO_TRY - 1` loop (line 173) to `loadAnalysis`'s caller —
  `back = 1..7` (the older cycles the loop exists to try) are never reached.
- `forecast.mfb:75`, inside the worker `fetchForecast` (`forecast.mfb:60-80`):
  `LET frame AS ForecastNews = ForecastFrame[hour := hour, winds :=
  windsFrom(tried.data, c)]`. `fetchForecast` has a `DO WHILE len(tried.data) =
  0` retry (lines 68-73) for the "not published yet" case, but a corrupt cache
  makes `tried.data` non-empty, so that loop never engages either — the
  `windsFrom` failure propagates out of the `ISOLATED FUNC` to
  `main.mfb:416`'s `thread::waitFor(worker) TRAP(e)`, ending the forecast
  entirely rather than retrying that one hour.

A second, compounding defect: `cycleData` never rewrites or deletes a cached
file once it has been read, whether or not it goes on to decode successfully —
that happens one call frame away, in `windsFrom`, which has no path back to
`cachePath`. So a corrupt file is never invalidated; `pruneCache`
(`fetch.mfb:115-121`) only deletes files whose name doesn't match the *current*
cycle's prefix, not files that failed to decode. Once a bad file lands at a
`cachePath`, every future run of the program hits the identical corrupt file
and fails identically until a human deletes `fs::tempDirectory()/mfb-wind-gfs`
by hand.

## Goal

- A cached file that fails to decode is treated as this cycle's attempt
  failing (an `Attempt.problem`), not as an unrecoverable error: `loadAnalysis`
  proceeds to `back + 1` the same way it does for an unpublished cycle, and
  `fetchForecast` retries the hour (refetching, since the cache is now known
  bad) the same way it does for a not-yet-published one.
- A cached file that fails to decode is invalidated (deleted, or rewritten by
  the retry's own atomic write) so the next run does not hit the same corrupt
  bytes forever.

### Non-goals (must NOT change)

- The magic-byte pre-check itself (`len(cached) > 16 AND beInt(cached, 0, 4) =
  GRIB_MAGIC`) as the cheap first filter — it correctly and cheaply rejects the
  common "HTML error page" case (§ the file's own comment, `fetch.mfb:130-132`)
  before ever touching the network or the decoder. The fix adds a validation
  step *after* it, it does not replace it.
- `attemptCycle`'s existing trailing function-level `TRAP` idiom, or the
  general `TRAP` semantics — this is not a parser or control-flow bug, `TRAP`
  behaves exactly as documented (§8.3).
- The atomic-write behavior of a freshly fetched file (`fs::writeBytesAtomic`,
  `fetch.mfb:147`), which already prevents a half-written file from being
  trusted — that guarantee is correct and is not what this bug is about.
- The eight-cycle retry bound (`CYCLES_TO_TRY`) and the "first cycle that
  answers wins" selection order in `loadAnalysis`.

## Blast Radius

Every site touching the cache-validity check or the decode entry point
(`grep -rn "GRIB\|windsFrom\|loadAnalysis\|attemptCycle\|cachePrefix\|cacheDirectory\|decodeFile" examples/wind/src`):

- `fetch.mfb:140` (`cycleData`'s magic-byte trust check) — root cause, fixed by
  this bug (a decode attempt, not just a magic-byte check, decides whether a
  cached file counts as data).
- `fetch.mfb:176` (`loadAnalysis` calling `windsFrom` outside any trap) — fixed
  by this bug.
- `forecast.mfb:75` (`fetchForecast` calling `windsFrom` outside any trap) —
  fixed by this bug, same pattern.
- `fetch.mfb:151-157` (`attemptCycle`'s function-level `TRAP`) — unaffected;
  it already does the right thing for the errors that currently reach it
  (network failures, "not published yet"). The fix routes the decode failure
  through this same mechanism rather than replacing it.
- `fetch.mfb:96-121` (`cacheDirectory`, `cachePrefix`, `cachePath`,
  `pruneCache`) — unaffected in naming/layout; `pruneCache`'s cross-cycle
  deletion logic is untouched, but the fix needs a way to delete or overwrite
  one bad file, which is new logic near here, not a change to the existing
  functions' contracts.
- `grib.mfb:418` (`decodeFile`) and its `FAIL error(errorCode::ErrInvalidFormat,
  …)` sites — unaffected; these already correctly detect and report a corrupt
  message. They are the mechanism the fix leans on, not something broken.
- `main.mfb:368` (`loadAnalysis() TRAP(e)`) and `main.mfb:416`
  (`thread::waitFor(worker) TRAP(e)`) — unaffected; these are the correct
  last-resort handlers for "every cycle really did fail," which the fix does
  not change. They currently also catch the corrupt-cache case, which is why
  the failure mode observed today is a clean message and exit(1) rather than a
  crash — the bug is the *scope* of the failure (whole load) and the *lack of
  retry*, not an uncaught panic.

## Fix Design

Move the decode attempt inside the function that `attemptCycle` already traps,
so a decode failure becomes an ordinary `Attempt.problem` instead of an
uncaught propagation:

- Change `cycleData` (or add a wrapper `attemptCycle` calls in its place) to
  decode the candidate bytes — cached or freshly fetched — before returning
  them as "data". Concretely: after the magic-byte check passes (cached path,
  line 140) or after the fetch succeeds (line 148), call `windsFrom`/
  `decodeFile` right there, inside `cycleData`'s body, which is covered by
  `attemptCycle`'s function-level `TRAP`. `attemptCycle`'s `Attempt` record
  already carries `data AS List OF Byte`; either add a `winds AS Winds` field
  decoded eagerly, or keep decoding lazily but do it once inside the trapped
  call so a decode failure surfaces as `Attempt.problem`.
- On a decode failure of a **cached** file specifically, delete the file
  (`fs::deleteFile(path)`) before returning the failure, so the next attempt —
  this run's retry to `back + 1`, or a wholly separate future run — does not
  read the same corrupt bytes again. A decode failure of a **freshly fetched**
  file should not be cached at all (don't call `writeBytesAtomic` until the
  fetched bytes decode) or should be deleted the same way if the write already
  happened before decode is attempted — the ordering needs to guarantee no
  undecodable file is ever left at `cachePath`.
- `loadAnalysis` and `fetchForecast` then need no changes to their own control
  flow: `loadAnalysis`'s `IF len(tried.data) > 0` check and retry loop, and
  `fetchForecast`'s `DO WHILE len(tried.data) = 0` retry, both already do the
  right thing once a decode failure shows up as `tried.problem` /
  `len(tried.data) = 0` instead of an uncaught error. `windsFrom` itself would
  then no longer need to be called separately at `fetch.mfb:176` and
  `forecast.mfb:75` if the `Attempt` carries the decoded `Winds` directly — or,
  if kept as a separate call for clarity, it becomes a pure re-decode of bytes
  already proven to decode once, which cannot newly fail.

Rejected alternative: wrapping only the two outer call sites
(`fetch.mfb:176`, `forecast.mfb:75`) in their own inline `TRAP` and turning a
decode failure into "treat this cycle as absent." This would fix the *crash*
but not the *poisoned cache* — the same corrupt file would be re-read and
re-fail on every subsequent attempt within the same run (each `back` step
would re-read a *different* cycle's file, so this is only a problem for
`fetchForecast`'s within-cycle retry and for a second program run hitting the
same newest cycle) unless deletion is added separately at both call sites,
duplicating logic that belongs once, at the point that reads the cache.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `examples/wind/tests/cache_corruption_test.sh` (see Validation Plan)
      reproducing this document's repro against the built `wind` example,
      asserting today's failure (decode error, no fallback) to pin the
      pre-fix behavior.
- [x] Confirm the blast-radius list above against HEAD (`grep` re-run) before
      editing.

Acceptance: the new script fails (in the sense of demonstrating the bug) against
current `examples/wind/src`. Confirmed: against HEAD (pre-fix) the driver fails
to build (`Attempt` has no `winds` member); a scratch driver built from the
unmodified pre-fix `fetch.mfb` (not committed) reproduced the actual crash —
`attemptCycle` reports the corrupt cache as succeeding, and the later
`windsFrom` call outside any `TRAP` propagates the decode error uncaught,
exit 255.
Commit: d859650de

### Phase 2 — the fix

- [x] Move decode-on-read into `cycleData` (or its replacement) so it runs
      inside `attemptCycle`'s trapped scope, per Fix Design.
- [x] Delete (or never write) an undecodable cached/fetched file at `cachePath`.
- [x] Verify `loadAnalysis` and `fetchForecast` need no further changes (their
      existing retry conditions already key off `len(tried.data)`).

Acceptance: the Phase 1 script now shows fallback to the previous cycle (for
`loadAnalysis`) and to a fresh fetch attempt (for `fetchForecast`'s hour retry)
instead of an aborted load; the two contrast cases (missing cache, non-GRIB
cache) are unchanged. Confirmed: `attemptCycle` on the corrupt cycle now
reports a non-empty `problem` and deletes the cache file; `loadAnalysis`'s and
`fetchForecast`'s existing `IF tried.problem = ""` / `DO WHILE tried.problem <> ""`
conditions (adapted from `len(tried.data)`, since `Attempt` now carries the
decoded `Winds` directly per the Open Decision below) needed no other change.
Commit: d437854b9

### Phase 3 — regenerate expected outputs + full validation

- [x] Re-run the Phase 1 script end to end against the real cache directory
      scheme, confirming no stray files are left in the shared temp directory.
- [x] Full project build (`mfb build examples/wind`) and a manual run to
      confirm the GUI app still launches and draws normally against a clean
      cache.

Acceptance: script passes; `examples/wind` builds and runs unchanged for the
non-corrupt case. Confirmed: `examples/wind/tests/cache_corruption_test.sh`
passes; `mfb build examples/wind` succeeds. The interactive GUI launch
against a live NOAA fetch was not re-driven this session (no network access
available in this task) — deferred, same as the rest of this bug's own
Failing Reproduction, which was likewise unable to drive the live app.
Commit: d437854b9

## Validation Plan

- Regression test: `examples/wind/tests/cache_corruption_test.sh` — a scripted
  probe (this example has no test harness today) that: builds a headless
  driver equivalent to the one used in this bug's repro (or, once the fix
  lands, calls the real `loadAnalysis`/`attemptCycle` against a temp cache
  directory it controls rather than the shared one), plants a `GRIB`-prefixed
  but undecodable file at the computed cache path for a synthetic cycle with a
  known-good older cycle available, and asserts (a) the load succeeds by
  falling back, and (b) the corrupt file no longer exists afterward.
- Runtime proof: the manual repro in this document, re-run after the fix,
  against the real shared cache directory (with the same backup/restore
  discipline used here) to confirm the newest-cycle corruption case falls back
  to the previous real cycle rather than failing.
- Doc sync: none expected — `examples/wind`'s in-file commentary
  (`fetch.mfb:127-135`) already asserts the intended contract ("a cached file
  is only trusted if it starts with it too"); update that comment to describe
  the post-fix contract (decoded, not just magic-checked) as part of Phase 2.
- Full suite: this is an example program with no project-wide test hook found;
  validation is the regression script above plus a manual `mfb build
  examples/wind` and run.

## Open Decisions

- Whether `Attempt` should carry a decoded `Winds` directly (avoiding a second,
  redundant `windsFrom` call at each of the two call sites) or keep carrying
  raw bytes with decode-on-read moved earlier purely for trap coverage,
  accepting one guaranteed-safe re-decode per call site. Recommend the former
  (carry `Winds`) — simpler call sites, no wasted decode — but it changes
  `Attempt`'s shape and both its current field (`data`) and both readers
  (`loadAnalysis`, `fetchForecast`), which is why it's flagged rather than
  assumed.
- Whether a freshly-fetched-but-undecodable file should be written to cache at
  all before the decode is attempted. Recommend deferring the
  `writeBytesAtomic` call until after a successful decode, so a bad fetch never
  touches the cache in the first place — simpler than write-then-delete, and
  matches the existing atomic-write comment's spirit ("a program closed in the
  middle of the write leaves no half a file to be trusted next time").

## Summary

The engineering risk is concentrated in relocating the decode call so it runs
inside `attemptCycle`'s already-correct trap, and in getting the
delete-on-failure ordering right so a bad file is never left at a
deterministic, permanently-reused cache path. The magic-byte check, the
retry-loop shapes in `loadAnalysis` and `fetchForecast`, and the top-level
`TRAP` handlers in `main.mfb` are all already correct and are left untouched;
they are the seams the fix threads the decode result through, not code that
needs to change.
