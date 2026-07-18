# bug-320: `scripts/test-accept.sh` has no watchdog on a `.run` test, so one hung binary wedges the whole suite with no diagnostic

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Test infrastructure (availability)

Status: Open
Regression Test: a fixture whose program blocks forever must fail that fixture and let the suite continue, not hang

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
