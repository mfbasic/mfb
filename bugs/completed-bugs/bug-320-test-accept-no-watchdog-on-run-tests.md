# bug-320: `scripts/test-accept.sh` has no watchdog on a `.run` test, so one hung binary wedges the whole suite with no diagnostic

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Test infrastructure (availability)

Status: Fixed 2026-07-18
Regression Test: `scripts/test-accept-selftest.sh`

## Resolution

`run_with_watchdog` wraps the `.run` execution in the same perl/`alarm` pattern
`test-macapp.sh` and `test-appimage.sh` already use, with a 300s default bound
(`MFB_ACCEPT_RUN_TIMEOUT` overrides). It differs from `run_headless` in passing
stdout/stderr straight through rather than summarizing them, because the program's
output is diffed as `build.log`. A hang prints `timeout` into that log — so it
diffs loudly against the golden — and yields 99. Exit status otherwise mirrors what
the shell reported before (128+N on a signal, else the program's code), so no
existing `[exit N]` golden churns.

The child opens `/dev/null` on fd 0 itself, so a fixture's result no longer depends
on how the harness was launched.

Two notes for whoever reads this next:

- **The `<pkg>.run` file is a trigger, not a golden.** This report called the
  checked-in `func_fs_pathNormalize_valid.run` stale because it holds 4 lines for 8
  `io::print` calls. It is never compared; `scripts/test-accept.sh:305` only tests
  its existence to decide whether to execute the fixture, and the program's output
  is diffed as `build.log` (which was complete and correct). Nothing was stale and
  nothing was masked.
- **perl is now a hard requirement of the harness**, checked once at startup with
  an actionable error rather than silently losing the watchdog on all 462 executing
  fixtures. macOS ships perl and has no `timeout(1)`; the Alpine boxes are the
  reverse (BusyBox `timeout`, no perl). The harness runs on macOS, so perl is the
  right primitive, but the guard makes the dependency explicit.

**This report's suggested 30–60s bound is wrong — do not restore it.** It assumed
fixtures are fast. Some are not, for reasons unrelated to the code under test: the
`tests/rt-behavior/native/*` LINK fixtures `dlopen` the system `libsqlite3.dylib`,
and macOS stalls 40–60s on that — 0s CPU, pure wall clock, duration varying with
the network (the signature of a Gatekeeper/notarization check). Measured directly:
`native-link-const-64bit-rt` 44s, `native-link-alias-collision-rt` **61s**. At a
60s bound the first is flaky and the second fails every run.

The bound is therefore 300s. It exists to turn an *infinite* hang into one named
failure, not to police performance, so headroom costs nothing — it only elapses on
a fixture that is already broken.

That 40–60s `dlopen` stall is a genuine (pre-existing, environment-specific) drag
on the suite, unrelated to this bug and not fixed here. Worth filing separately.

Validated by `scripts/test-accept-selftest.sh` (passthrough, exit code, 128+N on
signal, bounded hang printing `timeout`, and `/dev/null` stdin under a live pipe —
the last reproducing the original plan-51 trigger), plus a green acceptance run
across all 994 tests (rt-error 132, rt-behavior 339, syntax 522, acceptance 1).

`scripts/test-accept.sh` executes a fixture's program directly whenever the fixture
ships a `<pkg>.run` golden (`:263-270`). There is **no timeout anywhere in the
script** — `grep -n 'timeout\|perl -e\|alarm' scripts/test-accept.sh` returns
nothing. So a program that never exits stops the suite forever: no output, no
failing fixture, no exit code. The run simply sits there, and because the harness
buffers its per-fixture log until the fixture completes, `tail`ing the log shows
nothing either. The only way to see what happened is to `ps` for the child and
`lsof` its fds.

`scripts/test-macapp.sh` already establishes the house pattern for exactly this —
a 15s `perl`/`alarm` watchdog around each launch, with the comment "a GUI app that
fails to start does not exit — it hangs". `scripts/test-appimage.sh` (plan-51-D)
copies it. `test-accept.sh`, which is the suite everyone actually runs, has none.

This is not hypothetical: it cost roughly an hour during plan-51. The trigger was
launching the harness without redirecting stdin —

```sh
nohup ./scripts/test-accept.sh target/debug/mfb target/accept-actual > log 2>&1 &
```

— which leaves fd 0 as an inherited **pipe** rather than `/dev/null`. plan-15's
stdin broadcast reader subscribes to fd 0; on a pipe that never delivers and never
closes, that thread blocks forever, so the program finishes its work and then wedges
at teardown. `tests/rt-behavior/crypto/crypto-ec-valid` was where it landed, purely
because it happened to be first; the same launch hangs on any fixture with a `.run`
golden. Re-running with `< /dev/null` completes that fixture normally (16 lines of
expected output).

So there are two separable defects, and the first is the important one:

1. **No watchdog** (this bug). Any hang — from a real regression, a wrong stdin, a
   locked keychain, an unavailable device — presents identically as "the suite
   stopped", which is the least diagnosable failure mode available. A hang caused
   by a genuine codegen regression would look exactly the same, and would be
   *indistinguishable from a slow machine* to whoever is watching.
2. **Inherited stdin is load-bearing** and undocumented. The harness should
   `< /dev/null` each executed program itself rather than depending on how it was
   invoked, so the result does not change based on the caller's fd 0.

The single correct behavior a fix produces: a fixture whose program does not exit
within a bounded time is recorded as a failure for *that fixture*, naming it, and
the suite proceeds to the next one — and an executed program's stdin is
deterministically `/dev/null` regardless of how the harness was launched.

Suggested shape (mirroring `test-macapp.sh`, which already has a working
implementation to copy): wrap the program execution in the same `perl`/`alarm`
watchdog, print `timeout` into the fixture's actual output so it diffs loudly
against the `.run` golden, and add `< /dev/null` to the invocation. A generous
bound (30–60s) keeps slow fixtures like `tests/acceptance` — which legitimately
takes ~4 minutes as a `mfb test` run, a different code path — unaffected.

References:

- `scripts/test-accept.sh:263-270` — the unguarded `.run` execution.
- `scripts/test-macapp.sh:38-49` — `run_headless`, the 15s watchdog to copy.
- `scripts/test-appimage.sh` — `timeout_run`, the same pattern, added by plan-51-D.
- [[plan-15 stdin broadcast]] — the fd-0 subscriber that blocks on a live pipe.
- Found while running the plan-51 acceptance gate.
