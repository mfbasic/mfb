# plan-110-F: Network cross-target certification and closeout

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-110-E

Certify the completed net/tcp/udp/tls rework on every supported runtime family, reconcile only
expected generated artifacts, and finish embedded specifications/man output. The outcome is not
merely green compilation: real DNS, ICMP, TCP, UDP, TLS client/server, and HTTP behavior is
observed on the supported targets. (`wrap` is not certified because it does not exist — it was cut
in plan-110-D §C9.)

References: plan-110-E; `.ai/testing-gates.md`; `.ai/remote_systems.md`;
`.ai/specifications.md`; `.ai/man-content.md`; `.ai/build-tooling.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-110-E complete | `ls planning/plan-110-E-* 2>/dev/null` returns no matches | MET — measured 2026-08-30: no matches; archived at `planning/completed/plan-110-E-consumer-cutover.md` (commit e56e969ae). |
| No legacy live API references remain | anchored per-symbol sweep, below | MET — measured 2026-08-30: **zero live references**; see the classification below. |
| Full local suite green | `rustup run 1.96.0 cargo test` | MET — measured 2026-08-30 after merging main (plan-111 A–G): 67 test binaries, 0 failures, exit 0. |

**Legacy-reference sweep, measured 2026-08-30.** Same anchored form plan-110-E's census used, over
the 23 removed symbols:

```
for sym in Socket Listener connectTcp listenTcp accept read readText write writeText poll close \
           localAddress remoteAddress setReadTimeout setWriteTimeout bindUdp sendTo sendTextTo \
           receiveFrom receiveTextFrom UdpSocket Datagram DatagramText; do
  grep -rlE "net::${sym}([^A-Za-z0-9_]|$)" src tests scripts examples | grep -v '/golden/'
done
```

`planning/` and `bugs/` are excluded deliberately: those are the historical record of the
migration, and rewriting them would falsify it. Of the hits that remain, **none is a live API
reference** — every one is either a comment/doc naming the old spelling, or the negative fixture
that exists to prove the spelling is gone:

| class | count | example |
|---|---|---|
| the removal fixture itself | 1 file, 20 spellings | `tests/syntax/net/net_stream_surface_removed_invalid` |
| "migrated from" provenance comments in moved fixtures | 6 | `tests/rt-behavior/udp/func_udp_send_valid` (`' Migrated from net::sendTo by plan-110-C.`) |
| emitter/plan comments citing the behaviour they inherited | 12 | `tcp/gen_io.rs:405` (`matching net::connectTcp's bounded-wait error (bug-185)`) |
| `scripts/` provenance | 2 | `check-tcp-connect-timeout.sh` header, `scripts/README.md` |

The sweep did find **four user-facing** ones, fixed here rather than left for Phase 3 because a
prerequisite that reads "no legacy references" must not be signed off with stale `mfb man` text:
`tls::listen`'s description promised the endpoint "is resolved and bound exactly as `net::listenTcp`
does" (now `tcp::listen`), and `tls::close` plus the `tls` package overview both contrasted
themselves against `net::close` (now `tcp::close` — verified still accurate: `tcp::close` does treat
an already-closed handle as an error, `src/codegen/builtins/tcp/func_close.rs` DESC). A fourth,
`tcp/func_close.rs`'s internal note, named `net::close` as a fellow user of the shared emitter; it
now names `udp::close`, verified by both routing through
`fs::gen_handle::lower_fs_close_helper`.

## 1. Goal

- Prove the requested networking contract end-to-end across macOS AArch64, Windows x86_64, and
  supported Linux architecture/libc targets, with docs/specs and drift sentinels synchronized.

### Non-goals

- Do not weaken or skip tests to accommodate target failures.
- Do not call a compile-only artifact runtime proof.
- Do not reshape correct implementation merely to preserve byte identity; expected package/runtime
  changes require regeneration and delta attribution.

## 2. Current State

The host acceptance command does not exercise all cross-target native code, and `.ncodesum` files
are byte-identity drift sentinels rather than behavioral tests (`.ai/testing-gates.md`). The network
suite also includes a live external TLS fixture that may be environmentally red, so local loopback
peers are required for deterministic certification. This final letter remeasures the fixture and
artifact population after E rather than relying on the pre-plan count of 74 network/TLS fixture
sources from plan 110-A.

## 3. Design Overview

Build a deterministic loopback matrix with local DNS-independent addresses, local TCP/UDP peers,
generated test CA/server identities, TLS direct client/server flows, and an isolated permission-denied
ICMP environment. Run it natively per target. Keep live external connectivity only as an additional
signal. Serialize acceptance/golden commands that share output directories.

Expected codegen drift includes new packages/types/helpers and every deliberately migrated
fixture. Root-cause any other diff by objdumping one fixture, then fix the implementation or correct
the prediction before regeneration.

## Phases

### Phase 1 — Deterministic certification harness

- [x] Recount all networking fixtures/artifacts with commands and record the post-migration matrix
      in Corrections; ensure every requested overload has valid and invalid coverage.
      Matrix and commands in **F-C1**: 103 fixture directories across the five buckets (plan-110-A's
      pre-plan figure was 74), 25 byte-identity `.ncodesum` goldens, and all 40 declared overloads
      exercised by at least one fixture source. Valid/invalid symmetry was NOT holding -- `udp` had
      3 invalid fixtures for 8 members and `net::toUrl` had none -- so six new fixtures close it.
- [x] Add/finish reusable local peer scripts for TCP blackhole, UDP echo, TLS CA/client/server,
      and ICMP permission denial; no mocks or public-network dependency for required proof.
      TCP blackhole already existed (`scripts/check-tcp-connect-timeout.sh` +
      `net_blackhole_server.py`, retargeted to `tcp::connect` by plan-110-B). Three added:
      `scripts/check-udp-echo.sh` + `net_udp_echo_server.py` (a POSIX-sockets echo peer, so a round
      trip has to be right on the WIRE, not merely self-consistent -- payload, `Datagram.from`
      being the peer's own port, datagram boundaries, zero-length);
      `scripts/gen-test-tls-identity.sh` + `check-tls-loopback.sh` (a throwaway CA and a
      `SAN IP:127.0.0.1` leaf; an MFBASIC TLS server dialled by `openssl s_client`, plus the
      negative where an unrelated CA must be rejected); and `scripts/check-icmp-permission.sh`
      (`unshare -Urn` on Linux, `sandbox-exec` on macOS, to manufacture the ICMP denial the
      contract says must RAISE rather than return a `PingStatus`). Every one is loopback-only.
- [x] Prove the harness itself fails when one expected response/status/certificate is deliberately
      wrong, then restore it.
      `scripts/check-net-harness-selftest.sh` runs all four twice -- as shipped (must PASS) and
      against an injected wrong expectation (must FAIL, *and for the injected reason*, not by
      crashing on the edit). The sabotaged copy is written to `scripts/.selftest-<name>` and removed
      on every exit path including SIGINT, because each harness resolves its peer scripts relative
      to its own location and a copy elsewhere silently fails for the wrong reason. It also asserts
      the injection changed something: a `sed` that matched nothing would otherwise "prove"
      detection it never exercised. Measured: 8/8 verdicts correct.

Acceptance: one documented command per protocol yields stable observable results and detects an
injected wrong result. **Met**, measured 2026-08-30 on macOS aarch64:
`scripts/check-tcp-connect-timeout.sh`, `check-udp-echo.sh`, `check-tls-loopback.sh` and
`check-icmp-permission.sh` each PASS, and `check-net-harness-selftest.sh` reports
"every networking harness passes as shipped and fails on an injected wrong result".
Commit: —

### Phase 2 — Native target matrix

- [ ] Run macOS console proofs locally, Windows proofs on the configured Windows system, and Linux
      glibc/musl architecture proofs according to `.ai/remote_systems.md`; record exact commands and
      results in the phase ledger.
- [ ] Verify timeout elapsed bounds, ICMP permission Error, IPv4 and available IPv6 Address values,
      resource cleanup, and TLS certificate rejection on each backend.
- [ ] Prove no `wrap` surface survives anywhere: no registry member, no `WrapMode`, no runtime
      helper, no man/spec text promising it (plan-110-D §C9).
- [ ] Fix every defect found, adding a RED regression test before each fix as required by project
      policy; never leave a target-specific bug for another plan.
- [ ] **Carried in from plan-110-B §C5 — Windows TCP loopback is broken and was broken before
      plan-110.** Proven pre-existing: a `net` program built by a main-tip compiler (`f79f6212a`)
      behaves identically to one built on this branch. Measured on box 2230 (Windows 11,
      10.0.26100.9168):
      (a) `net::listenTcp("127.0.0.1", 0)` binds but `localAddress` reports **`0.0.0.0`**, not
      `127.0.0.1`; (b) connecting to that port then raises `ErrNetworkFailed` (7-707-0003);
      (c) with a **variable** host rather than a literal, `listenTcp` itself raises.
      Both symptoms point at the host string not reaching `getaddrinfo` intact on Windows — an
      empty node plus `AI_PASSIVE` is precisely what binds `0.0.0.0`. macOS and Linux report
      `127.0.0.1` and connect for all three shapes. Fix this before certifying `tcp`/`udp`/`tls`
      on Windows, with a RED runtime fixture first; `tcp` inherits the defect and cannot be
      execution-certified there until it is fixed.
- [ ] **Carried in from plan-110-D §C3 — `tls::listen` rejects a PKCS#8 private key on macOS with
      an opaque error.** Pre-existing, not introduced by plan-110. `SecItemImport` (which
      `keyPath` goes through) returns `errSecUnknownFormat` (-25257) for
      `-----BEGIN PRIVATE KEY-----` and accepts only the traditional
      `-----BEGIN RSA PRIVATE KEY-----` form — so a key produced by a modern `openssl req`
      invocation fails with no indication of why. Either accept PKCS#8 or raise a diagnostic that
      names the format problem and the `openssl rsa -traditional` conversion, and document the
      accepted formats on `tls::listen`.

Acceptance: every supported native target passes the protocol matrix; any genuinely unavailable
environment is reported as a blocker rather than represented by compile success.
Commit: —

### Phase 3 — Artifacts, docs, and final gates

- [ ] Run full cargo test, acceptance, coverage/artifact gates, and serialized all-target golden
      regeneration; attribute every changed `.ncode`/`.ncodesum` to plan 110 behavior.
- [ ] Complete registry descriptor man content and the canonical embedded stdlib spec for net/tcp/
      udp/tls; update HTTP references, capability audit expectations, and all citations.
- [ ] Render `mfb man <pkg> --all` and owning `mfb spec ... --all`; ensure no `[[` leaks and every
      requested signature/type/status/ownership/error rule is documented once.
- [ ] Run `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`, then
      re-run `rustup run 1.96.0 cargo test` and the final acceptance gate.

Acceptance: all required gates and native runtime proofs pass; generated deltas are exclusively
attributable to plan 110; the four rendered package surfaces exactly match the request.
Commit: —

## Validation Plan

This plan is itself the validation plan. A green gate means only that covered code is green, so the
native matrix explicitly executes each backend and protocol. Commands/results must be recorded in
the phase ledger when run; claims without commands are guesses.

## Open Decisions

- Required IPv6 certification — recommend native IPv6 loopback wherever the OS runner exposes it;
  record an environmental blocker rather than silently reducing Address coverage.

## Corrections

**F-C1 — the post-E fixture/artifact matrix (Phase 1 box 1).** Measured 2026-08-30. A fixture is a
directory holding a `project.json` (`find <tree>/<bucket> -name project.json | wc -l`); the
byte-identity column counts per-target `.ncodesum` goldens (`ls tests/byte-identity/<b>/golden/*.ncodesum`).

| bucket | rt-behavior | syntax | rt-error | byte-identity targets | members |
|---|---|---|---|---|---|
| `net` | 7 | 5 | 1 | 5 | 5 |
| `tcp` | 22 | 17 | 1 | 5 | 11 |
| `udp` | 6 | 6 | 0 | 5 | 8 |
| `tls` | 5 | 18 | 0 | 5 | 11 |
| `http` | 7 | 3 | 0 | 5 | — |

plan-110-A's pre-plan count was **74 network/TLS fixture sources**; the post-migration total across
these five buckets is **103** fixture directories. The plan was right to say the number had to be
remeasured rather than carried forward.

**Every overload is exercised.** Per-shape grep over fixture sources only
(`grep -rlE '<pattern>' tests --include='*.mfb' | grep -v /golden/`), run for all 40 declared
overloads across the four packages; the lowest count was 1 (`udp::poll` scalar form) and no shape
came back 0. The full command is `/tmp/f_overload_cover.sh` in the session log; the distinguishing
patterns are recorded per shape, e.g. `tcp::connect\("[^"]*", *[^,)]+, *[^,)]+\)` for the
host/port/timeout form vs `tcp::connect\([a-zA-Z_][A-Za-z0-9_.]*, *[^,)]+\)` for the
Address/timeout one.

**But valid/invalid coverage was NOT symmetric, and the box required it to be.** `tcp` and `tls`
each had a per-member `_invalid` fixture; `udp` had three (`bind`, `send`, `receive`) for eight
members, and `net` had none for `toUrl`. Five members were reached only incidentally, from fixtures
written for something else. Closed here, with six new fixtures:

| fixture | closes |
|---|---|
| `syntax/udp/func_udp_poll_invalid` | both `poll` overloads; also that a `List OF RES tcp::Socket` is rejected by `udp::poll` — the bare-name confusion plan-110-E's C2 closed, now pinned from the udp side too |
| `syntax/udp/func_udp_endpoints_invalid` | `localAddress`, `close`, both timeout setters, and `close`'s consume (`TYPE_USE_AFTER_MOVE`) |
| `syntax/udp/func_udp_absent_members_invalid` | `remoteAddress`, `receiveText`, and net's `bindUdp`/`sendTo`/`receiveFrom` spellings |
| `syntax/net/func_net_toUrl_invalid` | `toUrl` arity/argument type (the runtime half was already `rt-behavior/net/func_net_toUrl_invalid_runtime`) |
| `rt-behavior/udp/func_udp_endpoints_valid` | `localAddress` on an ephemeral bind, both setters at `0`/positive/negative, `close` |
| `rt-behavior/udp/func_udp_poll_valid` | scalar readiness (FALSE at 0, TRUE after a send), the list multiplex's borrowed return, `ErrTimeout` on an idle list, `ErrInvalidArgument` on an empty one |

Two facts had to be corrected while writing them, both measured rather than assumed:

* **Unknown-member and type diagnostics cannot share a fixture.** The resolver reports an unknown
  member before type checking runs, so a single `udp::remoteAddress(1)` in the endpoints file
  collapsed 15 diagnostics to 1. That is why `func_udp_absent_members_invalid` is separate — the
  same split `tls`'s `readText_invalid` / `writeText_invalid` pair already documents.
* **Use-after-close is a COMPILE error, not a runtime raise.** `func_udp_endpoints_valid` first
  tried to `TRAP` a call on a closed socket; it does not build (`TYPE_USE_AFTER_MOVE`), because
  `close` consumes its argument. The case moved to the invalid fixture, which is the stronger
  statement.

**F-C2 — the TLS harness crashed on its first run: bug-459.** An explicit `tls::close(listener)`
segfaults on macOS. `nw_connection_cancel` is called on the `nw_listener`, whose dispatch-queue slot
is not a connection's, and `dispatch_async` faults on a null queue
(`KERN_INVALID_ADDRESS at 0x54`). The `Listener` overload's rewrite onto the internal
`tls.closeListener` body selected on the BARE type name while a built-in resource has been
package-qualified end to end since bug-441, so the filter matched nothing and the SOCKET body ran.
Scope drop was unaffected (the resource descriptor names `tls.closeListener` directly), which is why
only an explicit close crashed and why no fixture noticed.

Dated precisely, not guessed: the `.ir` golden of `tests/syntax/tls/close_valid` carried
`tls.closeListener` at `b61003c20^` and four plain `tls.close` at `b61003c20` — bug-441 itself,
which re-baselined the golden with the loss already in it. A byte-identity golden recorded the
regression rather than catching it, because nothing asserted what the target should BE.

Present on `main` too; plan-110-D renamed the constant but did not introduce the mismatch. Fixed by
comparing `TLS_LISTENER_TYPE_ID`; RED-checked first
(`ir::tests::lower_coverage_tests::explicit_tls_listener_close_rewrites_to_the_listener_body` failed
before, passes after), with a mirror test pinning that `tcp::Listener` — which shares the bare name
— does not reach tls's body. Nine goldens regenerated, every diff line a `tls.close` →
`tls.closeListener` on a listener argument: 4 `.ir` and the 5 per-target tls `.ncodesum`, the latter
confirming the misrouting was on every backend, not just macOS.

This is exactly what the plan's own §Design Overview predicted a deterministic local harness would
be for: the surface had 18 compile-only fixtures and no server had ever been executed.

## Summary

This letter prevents a broad networking rewrite from being declared complete on compiler proxies.
Its hard gate is real native behavior, especially ICMP permissions and the TLS client/server
handshake on each backend.
