# bug-483: `tls::localAddress(listener)` returns a record whose `host` String segfaults

Last updated: 2026-08-31
Effort: small-to-medium — the repro is four lines and deterministic; the regression
window is six commits wide and already bounded
Severity: HIGH
Class: Correctness (regression — a working call now crashes the process)

Status: Open
Regression Test: two exist and are RED at HEAD —
`tests/rt_tls_listener_local_address.rs::tls_local_address_reports_the_port_a_listener_bound_to`
and
`tests/rt_tls_listener_thread_transfer.rs::a_transferred_tls_listener_accepts_on_the_receiving_thread`.
Both were added by bug-465 and both pass on the baseline below, so no new fixture is
needed — the fix is verified by turning these green.

## What happens

A program that asks a TLS **listener** for its bound address, and then reads a field of
the returned `net::Address`, dies with `SIGSEGV` (exit 139).

```basic
IMPORT io
IMPORT net
IMPORT tls

FUNC main AS Integer
  RES server = tls::listen("127.0.0.1", 0, "/tmp/p98-tls/cert.pem", "/tmp/p98-tls/key.pem")
  LET b = tls::localAddress(server)
  io::print("host=" & b.host & " port=" & toString(b.port))
  RETURN 0
END FUNC
```

```
$ ./build/tlsprobe.out
[exit 139]
```

Nothing is printed — the crash is in the `&` concatenation that reads `b.host`, before
`io::print` is reached.

**It is the field read, not the call.** With the print removed the program runs past
`localAddress` and prints a following statement, then still faults on the way out:

```basic
  RES server = tls::listen(...)
  io::print("listening")        -> printed
  LET b = tls::localAddress(server)
  io::print("got address")      -> printed
  tls::close(server)
  RETURN 0                      -> [exit 139]
```

So the record comes back, and its `host` String payload is bad. `tls::listen` +
`tls::close` with no `localAddress` between them exits 0.

## It is a regression, and the window is bounded

Measured with the `git archive` + `cargo build --release` attribution technique
(`.ai/` / bug-478's notes), running the SAME four-line program against each binary:

| commit | date | result |
|---|---|---|
| `f0e4d3ff2` plan-108-C: compile every network-family example | 2026-08-30 | **`host=127.0.0.1 port=60594`, exit 0** |
| `8b27c8a11` bug-480 Phase 4b: regenerate the goldens the declared-name change shifts | 2026-08-31 | exit 139 |
| `4cce58103` bug-480 Phase 4b: qualify the remaining registry type identities | 2026-08-31 | exit 139 |
| `HEAD` | 2026-08-31 | exit 139 |

So the break is inside `f0e4d3ff2..8b27c8a11` — the bug-480 Phase 4 series and the
plan-108-C/D commits interleaved with it. `4cce58103` is *not* the culprit: its parent
already crashes.

## Ruled out

* **The declared type spelling.** `tls::localAddress`, `tls::remoteAddress` and
  `tls::connect` still name the bare `net::ADDRESS_TYPE` where `udp::address()` was
  moved to `ADDRESS_TYPE_ID` by `4cce58103`. Qualifying all three tls sites and
  rebuilding does **not** fix the crash (measured).
* **`tcp::address()`.** Reverting it to the bare name and rebuilding does not fix the
  crash either (measured). That change is correct on its own grounds — it restores the
  tcp/udp consistency `tcp_endpoints_use_the_shared_net_address_record` asserts — and is
  unrelated to this.
* **Load / contention.** bug-465's deadlines were widened for exactly that reason, but
  this reproduces on a quiet machine, in isolation, every run.
* **The cert.** A freshly generated `openssl req -x509 -newkey rsa:2048 -nodes` pair, the
  same shape the test uses.

## Where to look first

`tls::localAddress` has **two** overloads — Socket and Listener — and bug-465 finding 3
was precisely that the Listener one was missing. The Socket path is what
`tls::remoteAddress` and the tcp equivalents exercise, and those are not obviously
broken; the failing case is the Listener overload specifically. So the suspicion is that
the overload's *return* record is built through a path that the value-type namespace
change re-pointed, leaving the `host` String's payload pointer wrong — the same class of
symptom as a stale record layout, not a null handle.

`.ai/net-tls.md` and plan-110-D record that the endpoint queries reuse the `net` address
emitter and that the TLS record keeps the fd in the canonical handle slot; that shared
emitter is the first thing to read.

## Found while

Landing plan-98-F Phase 3. The two RED tests turned up in the full-suite run at the end
of that work and were **not** caused by it — plan-98's changes are Windows-only codegen
plus test expectations, and both were ruled out by measurement above. An earlier
full-suite run in the same session appeared not to show them, but that output had been
truncated by a `head -12` on exactly twelve matching lines, so it is no evidence that
they were ever green after the bug-480 series landed.
