# bug-488 — `rt_tls_connect_allow_self_signed` is flaky when two `cargo test` runs share a machine

STATUS: OPEN (test-isolation flake; no product defect)
FOUND: plan-121-E prerequisite run (2026-09-03)
SEVERITY: low — the test fails CLOSED (it reports the anomaly); it does not pass silently

## Symptom

In a full `cargo test --no-fail-fast`, one case failed:

```
defaults_to_rejecting_a_self_signed_peer ... FAILED
  left:  "result=connected"
  right: "result=raised"
```

The client **connected** to a peer it should have refused. Re-run in isolation
**3/3 green**, all four cases each time:

```
cargo test --test rt_tls_connect_allow_self_signed -- --test-threads=1
```

At the time of the failure a second worktree session (`P-116`) was running its own
full `cargo test` on the same machine.

## Second occurrence — it fails in BOTH directions (2026-09-03, bug-500 landing)

A second sighting during the audit-3 fix pass, with the **opposite** signature:

```
accepts_a_self_signed_peer ... FAILED
  left:  "result=raised"
  right: "result=connected"
```

so the client **refused** a peer it should have accepted, where the first sighting
had it **accept** one it should have refused. Same trigger: two other
`cargo test` runs (audit-3 fix agents) were live on this machine at the time.
Re-run in isolation immediately afterwards, 4/4 green:

```
cargo test --no-fail-fast --test rt_tls_connect_allow_self_signed -- --test-threads=1
→ test result: ok. 4 passed; 0 failed
```

The rest of that suite was clean (4589 passed, 1 failed — this case only).

This matters for the diagnosis: a gate that can flip a verdict in *either*
direction is not "a stale allow that leaks in", it is two runs sharing one
process-global piece of state and each seeing the other's value. Whichever fix is
chosen must therefore make the state per-connection (or per-test), not merely
reset it before each case — resetting still races when the reader is another
process's test.

## Third occurrence — first direction again (2026-09-03, bug-496 landing)

`defaults_to_rejecting_a_self_signed_peer` failed with `left: "result=connected"`
(the first signature) during the bug-496 merged full run, while two other audit-3
fix agents had `cargo test` live on this machine. Sole failure in the run (113
result lines otherwise green); re-run in isolation immediately afterwards 3/3 green
(`cargo test --test rt_tls_connect_allow_self_signed`, 4 passed each time).

## Fourth occurrence — a DIFFERENT symptom, so the shared state is wider than the port (2026-09-03, plan-122 landing)

Two sightings in one plan-122 session, both with a peer worktree session
(`P-116`) running its own `cargo test` on the same machine.

The first matched §Symptom exactly (`defaults_to_rejecting_a_self_signed_peer`,
`left: "result=connected"`, `right: "result=raised"`). The second, in the
post-merge run, is **new**:

```
still_rejects_an_expired_certificate ... FAILED
  the certificate meant to be expired is still valid — this case would assert the
  opposite of what it reads as:
  Certificate will not expire

still_rejects_a_name_mismatch ... FAILED
```

That first one is not a verdict flip at all — it is the file's own **setup
guard** firing, refusing to run a case that would have asserted vacuously. So the
test is failing closed here too, which is good, but it means the contention
damages more than the port: the cases also share the **generated certificate
identity** on disk, and a concurrent run regenerating it can leave this run
looking at an identity that is not the one its case needs.

Isolation re-runs: **4/4 green, twice**, immediately after each failure
(`cargo test --release --test rt_tls_connect_allow_self_signed`).

Consequence for the fix: making `port_gate()` cross-process is **necessary but not
sufficient**. Whatever lock is chosen has to cover the identity generation as
well, or this case will keep failing under a shared machine after the port race is
closed. A per-run temporary identity directory would remove the second half
outright and is probably cheaper than locking it.

Not attributable to plan-122, which touches no TLS code: `artifact-gate.sh all`
reported 1890 goldens 0 diffs on the same tree, and the whole
`rt_tls_connect_allow_self_signed` file is green when nothing else is running.

**Four sightings in one day, by three unrelated sessions.** The third and fourth
were reported independently and collided as a merge conflict in this file, which is
itself a measure of how often this fires: it is no longer an occasional flake but a
near-certainty whenever two `cargo test` runs overlap, which is now the normal
working pattern. Both were kept rather than one being resolved away — the third
adds frequency evidence, the fourth adds a symptom the port race does not explain.

## Fifth occurrence — a fixed-port sibling, `rt_macos_tls_write_capacity` (2026-09-04, bug-502 landing)

`macos_tls_write_sends_capacity_over_count_byte_list_exactly` (fixed
`PORT = 18453`, 1 s bind sleep, `openssl s_client` peer) failed once in the
bug-502 landing suite with `peer did not receive the exact byte payload
[65, 66, 67, 68, 69]; got []` while a peer session's `cargo test` was running
the same suite on this box; the branch touched only `src/fmt.rs`/`src/cli/fmt.rs`.
Re-run alone (`--test-threads=1`) it passed in 2.1 s, and `lsof -i :18453` was
empty afterwards. Same class: a fixed port shared across processes.

## Cause

`tests/rt_tls_connect_allow_self_signed.rs` picks a port by binding an ephemeral
one and **releasing it** so `openssl s_server` can take it (`free_port`, line
~361). The file knows this window is dangerous and guards it with `port_gate()`
(line ~349) — but that is a `static OnceLock<Mutex<()>>`, i.e. **per-process**.

It therefore serializes the four cases *inside this test binary* and cannot
serialize against:

- other test binaries in the same `cargo test` run, or
- **another `cargo test` on the same machine** — now routine, since work happens
  in several `.claude/worktrees/*` sessions concurrently.

`start_peer`'s loser detection only catches the case where our `s_server` **fails
to bind** (it exits, so `lost = true` and it retries). It cannot catch the
converse: our port being handed to someone else's listener in the release window,
after which the client under test reaches a stranger.

## Why it matters even though the test failed correctly

The assertion did its job. But the file's own header says the negatives are the
important cases, and a negative that flakes gets re-run and waved through — which
is how a real regression in default TLS trust would eventually be dismissed as
"that flaky TLS test". The value at risk is the credibility of a security
assertion, not the assertion itself.

## Fix directions (not yet chosen)

- Have `s_server` bind and report its own port, removing the release window
  entirely (the file notes banner-parsing is "markedly less stable across OpenSSL
  versions" — so measure before adopting).
- Or make the gate cross-process: an advisory lock on a file under a well-known
  path, which is what `scripts/artifact-gate.sh` already does for a different
  shared resource.
- Or bind the listener in-process and hand `s_server` the inherited descriptor.

## Reproduce

Run two full `cargo test --no-fail-fast` invocations concurrently from two
worktrees of this repo. Expect an intermittent `result=connected` in
`defaults_to_rejecting_a_self_signed_peer`, or a name-mismatch/expired case
reporting the wrong outcome for the same reason.
