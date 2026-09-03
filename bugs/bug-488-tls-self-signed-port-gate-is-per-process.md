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
