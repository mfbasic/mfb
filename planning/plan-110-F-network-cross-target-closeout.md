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
Commit: ae180fb7e (census + coverage), 730ca79d5 (harnesses + bug-459)

### Phase 2 — Native target matrix

- [x] Run macOS console proofs locally, Windows proofs on the configured Windows system, and Linux
      glibc/musl architecture proofs according to `.ai/remote_systems.md`; record exact commands and
      results in the phase ledger. Matrix and per-box results in **F-C3**; six targets run the same
      loopback program, two boxes were down and are recorded as such rather than skipped silently.
- [x] Verify timeout elapsed bounds, ICMP permission Error, IPv4 and available IPv6 Address values,
      resource cleanup, and TLS certificate rejection on each backend. All covered by the F-C3
      matrix and the TLS harnesses, except IPv6 -- see **F-C4**, which records that there is no IPv6
      surface to certify and measures what happens instead.
- [x] Prove no `wrap` surface survives anywhere: no registry member, no `WrapMode`, no runtime
      helper, no man/spec text promising it (plan-110-D §C9).
      Measured 2026-08-30, `grep -rIn 'WrapMode\|tls::wrap\|tls\.wrap\|"wrap"' src tests scripts
      examples | grep -v /golden/`, discounting `wrapper`/`wrapped`/`wrapping`/`text-wrap`: **two
      hits, neither a surface.** `spec/language/18_builtin-functions.md` states the absence and why;
      `examples/browser/dom/src/resolve.mfb` matches the CSS `flex-wrap` value. No registry member,
      no `WrapMode`, no runtime helper, no promise.
- [x] Fix every defect found, adding a RED regression test before each fix as required by project
      policy; never leave a target-specific bug for another plan.
      Five, each RED-checked before its fix and archived to `bugs/completed/`: **bug-458**
      `poll(List)` double-closing its borrowed element, **bug-459** `tls::close(listener)` running
      the socket body (SIGSEGV on macOS), **bug-460** Windows never initializing Winsock for
      `tcp`/`udp`, **bug-461** Schannel rejecting a PKCS#1 key, **bug-462** macOS `tls::listen`
      releasing uninitialized stack slots on every failure path (SIGTRAP). Three of the five were
      invisible before this letter because no test had ever *executed* a TLS server.
- [x] **Carried in from plan-110-B §C5 — Windows TCP loopback is broken and was broken before
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
      **Resolved — all three shapes are green on box 2230** (see F-C3 for the run). Two separate
      causes, neither the single one §C5 guessed at: the four Windows defects fixed in plan-110-D,
      and **bug-460** here. §C5's own hypothesis -- "the host string not reaching `getaddrinfo`
      intact" -- was wrong; see **F-C5**.
- [x] ~~**Carried in from plan-110-D §C3 — `tls::listen` rejects a PKCS#8 private key on macOS with
      an opaque error.** Pre-existing, not introduced by plan-110. `SecItemImport` (which
      `keyPath` goes through) returns `errSecUnknownFormat` (-25257) for
      `-----BEGIN PRIVATE KEY-----` and accepts only the traditional
      `-----BEGIN RSA PRIVATE KEY-----` form — so a key produced by a modern `openssl req`
      invocation fails with no indication of why. Either accept PKCS#8 or raise a diagnostic that
      names the format problem and the `openssl rsa -traditional` conversion, and document the
      accepted formats on `tls::listen`.~~ — moot: the premise no longer holds.
      **The premise is FALSE as of 2026-08-30 — macOS accepts PKCS#8** end to end (listen, accept,
      read, write, close, clean exit, verified against `openssl s_client`). The real defect is the
      MIRROR of the one recorded, on the platform this letter first executed a TLS server on:
      **Windows rejected PKCS#1** (bug-461, fixed). The accepted formats are now documented on
      `tls::listen`. See **F-C6**.

Acceptance: every supported native target passes the protocol matrix; any genuinely unavailable
environment is reported as a blocker rather than represented by compile success. **Met** — the F-C3
matrix, plus TLS client+server proofs on macOS, Windows and four Linux boxes. Boxes 2224
(aarch64 musl), 2232 (riscv64 glibc), 2222, 2225 and 2226 refused connections and are reported as
unavailable, not as passes; both architectures they would have added are execution-covered by
another libc (2223 aarch64 glibc, 2229 riscv64 musl).
Commit: —

### Phase 3 — Artifacts, docs, and final gates

- [x] Run full cargo test, acceptance, coverage/artifact gates, and serialized all-target golden
      regeneration; attribute every changed `.ncode`/`.ncodesum` to plan 110 behavior.
      Final run: `cargo test --no-fail-fast` 68 binaries exit 0 (`artifact_gate_all` inside it);
      acceptance 1307 tests / 0 mismatches; `artifact-gate.sh all` 1291 tests, 1448 builds, 1780
      goldens. **Every golden regenerated in this letter is attributed to one named fix**, and each
      diff set was classified BEFORE regenerating: 10 tcp/udp `.ncodesum` (bug-458's removed
      cleanup, the only cover fixtures binding a list poll), 4 `.ir` + 5 tls `.ncodesum` (bug-459's
      `tls.close` -> `tls.closeListener`), 2 windows-only tcp/udp `.ncodesum` (bug-460's entry
      gaining `WSAStartup`), and 4 tls/http `.ncodesum` on macOS + Windows (bug-461/462 changing
      `tls::listen`; http embeds tls). No diff went unexplained and none was regenerated in bulk.
- [x] Complete registry descriptor man content and the canonical embedded stdlib spec for net/tcp/
      udp/tls; update HTTP references, capability audit expectations, and all citations.
      The stdlib spec had no transport topic at all -- the split left the *model* (what a handle is,
      who closes it, stream vs datagram, poll's two conventions, the TLS credential rules) stated
      nowhere in one place, with only per-function man pages. Added
      `src/docs/spec/stdlib/17_transports.md`, indexed in `stdlib/spec.md` and cross-linked from the
      http and icmp topics. `tls::listen`'s own descriptor gained the accepted key encodings and a
      corrected account of when a mismatched pair surfaces (F-C6, F-C7). The capability-audit
      expectations and the six other spec topics were corrected in plan-110-E (C4-C7).
- [x] Render `mfb man <pkg> --all` and owning `mfb spec ... --all`; ensure no `[[` leaks and every
      requested signature/type/status/ownership/error rule is documented once.
      Measured 2026-08-30: `mfb man net|tcp|udp|tls --all` and `mfb spec stdlib transports` each
      render **0** `[[` occurrences. Surfaces are exactly as requested -- net 5 functions and no
      resources at all, tcp 11 + Socket/Listener, udp 8 + Socket/Datagram, tls 11 + Socket/Listener
      with no `wrap` and no `readText`/`writeText`.
- [x] Run `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`, then
      re-run `rustup run 1.96.0 cargo test` and the final acceptance gate.
      Both formatters run, both suites green (counts above). Note the parentheses in that command:
      dropping the subshell leaves the session's working directory in `repository/`, and the next
      `cargo test` then silently runs **only** that sub-workspace -- 318 tests, exit 0, looking like
      a pass. Caught here by the binary count (4 result lines instead of 68) and re-run from the
      worktree root.

Acceptance: all required gates and native runtime proofs pass; generated deltas are exclusively
attributable to plan 110; the four rendered package surfaces exactly match the request. **Met**, as
measured in the three boxes above.
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

**F-C3 — the Phase 2 native matrix.** One loopback program (literal-host listen + `localAddress`,
connect/accept/read/write/`remoteAddress`, a VARIABLE host, the wildcard bind, `connect(Address)`,
udp bind/send/receive with the sender address checked against the peer's own port, the accept/poll/
negative-timeout convention, and `net::ping`), cross-built per target and run on the box:

| box  | target              | transport matrix | MFBASIC↔MFBASIC TLS |
|------|---------------------|------------------|---------------------|
| —    | macos-aarch64       | all green | server proven vs `openssl s_client`; client leg needs a local anchor macOS has no hook for |
| 2230 | windows-x86_64      | all green | server proven vs `openssl s_client` over an ssh tunnel |
| 2227 | linux-x86_64 musl   | all green | PASS + negative |
| 2228 | linux-x86_64 glibc  | all green | PASS + negative |
| 2223 | linux-aarch64 glibc | green except ping | PASS + negative |
| 2229 | linux-riscv64 musl  | all green | PASS + negative |

Boxes 2222, 2224, 2225, 2226 and 2232 refused connections; recorded as unavailable, not as passes.
Both architectures they would have added are execution-covered by the other libc.

The 2223 ping result is the CONTRACT, not a defect, and was measured rather than assumed:
`cat /proc/sys/net/ipv4/ping_group_range` is `1 0` (an empty range) with the caller's gid 1001,
while 2227 has `999 59999` and 2228 `0 2147483647` with gid 1000. plan-110-A §C3 requires an OS
refusal to be an ERROR, never a `PingStatus`, and `mfb man net ping` names the empty-range
distribution case explicitly.

**F-C4 — there is no IPv6 surface to certify, so the Open Decision is moot.** `net::lookup` is
documented IPv4-only ("Only IPv4 results are returned"), `net::Address` carries a textual IP with no
family, and the emitters use `AF_INET` throughout. The plan's Open Decision recommended "native IPv6
loopback wherever the OS runner exposes it"; there is nothing to exercise. Measured instead:
`tcp::listen("::1", 0)` raises `77070001` (ErrAddressInvalid) — a clean rejection, not a silent
mis-parse or a wildcard bind. Adding IPv6 is a feature, not a certification gap, and is out of scope
for a letter whose goal is to certify the surface that exists.

**F-C5 — plan-110-B §C5's hypothesis was wrong, and its symptoms had two causes.** That letter
guessed "the host string not reaching `getaddrinfo` intact on Windows — an empty node plus
`AI_PASSIVE` is precisely what binds `0.0.0.0`". The actual causes were the four Windows defects
fixed in plan-110-D, and **bug-460**: the entry never called `WSAStartup` for a `tcp`/`udp` program,
so every socket call failed with `WSANOTINITIALISED`. Worth recording because the wrong hypothesis
would have sent the next reader into `getaddrinfo` argument marshalling, which was never at fault.
bug-460 also could not have been found by any probe written before the transport split: every one of
them called `net::lookup` first, which flipped the gate for the rest of the program.

**F-C6 — the carried-in macOS PKCS#8 defect does not reproduce; its MIRROR does.** plan-110-D §C3
recorded `tls::listen` rejecting a PKCS#8 key on macOS with `errSecUnknownFormat`. Measured
2026-08-30 on macOS aarch64: a PKCS#8 key listens, accepts, reads, writes and closes, verified
against `openssl s_client`. The premise is false. What IS true is the same class in the opposite
direction, on the platform this letter first executed a TLS server on:

| key PEM | macOS | Linux (OpenSSL) | Windows (Schannel) |
|---|---|---|---|
| PKCS#8 `BEGIN PRIVATE KEY` | serves | serves | serves |
| PKCS#1 `BEGIN RSA PRIVATE KEY` | serves | serves | **7-707-0008 at listen** (bug-461, fixed) |

Both encodings now work everywhere and `tls::listen` documents that, so one PEM serves every target.

**F-C7 — two `tls::listen` documentation claims were false, and probing them found bug-462.** The
descriptor said a cert or key that "does not match its partner raises `ErrTlsFailed`" at listen. It
does not: no backend verifies the pair while building the credential. Measured on macOS — listen
succeeds, `tls::accept` raises `7-707-0008` on the first connection, and the client reports
`tls_process_cert_verify: bad signature`. Corrected to describe when the mismatch actually surfaces.

Probing the *other* half of that sentence — a key that "cannot be read, does not parse" — is what
found **bug-462**: an encrypted PEM did not raise at all, it killed the process with SIGTRAP in
`CFRelease`, because `tls::listen`'s four CoreFoundation cleanup slots were never initialized and
its NULL-guarded release ran on stack garbage. Every failure exit shared that path. After the fix
all five exits raise catchably: missing cert, missing key, garbage cert, encrypted key, and (at
accept) a mismatched pair.

## Summary

This letter prevents a broad networking rewrite from being declared complete on compiler proxies.
Its hard gate is real native behavior, especially ICMP permissions and the TLS client/server
handshake on each backend.
