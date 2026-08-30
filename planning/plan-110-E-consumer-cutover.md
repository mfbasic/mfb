# plan-110-E: Consumer cutover and legacy net removal

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-110-D

Atomically migrate compiler-owned consumers and fixtures to `tcp`, `udp`, and the new TLS type
names, then remove the legacy transport surface from `net` so the installed packages exactly match
the requested API.

References: plan-110-D; `.ai/resources-packages.md`; `.ai/man-content.md`;
`src/codegen/builtins/http/mod.rs:register`; `src/codegen/builtins/net/mod.rs:register`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-110-D complete | `ls planning/plan-110-D-* 2>/dev/null` returns no matches | MET — measured 2026-08-30: no matches; archived at `planning/completed/plan-110-D-tls-wrap-and-surface.md` (commit 9a08b48f3). |
| New packages pass their standalone runtime gates | `rustup run 1.96.0 cargo test` plus B/C/D runtime commands | MET — measured 2026-08-30: 65 test binaries, exit 0. Acceptance 1297 tests / 0 mismatches; `artifact-gate.sh all` 1281 tests, 1438 builds, 1770 goldens, 0 diffs. B/C/D runtime commands re-run cross-target on macOS aarch64, Alpine x86_64 musl (2227) and Windows 11 (2230) with identical output. |

## 1. Goal

- `net` exports only lookup/query/URL/ping values and functions; all TCP, UDP, TLS, HTTP, tests,
  scripts, docs, and package metadata use the new contract with no public compatibility aliases.

### Non-goals

- Do not change HTTP parsing, framing, timeout values, TLS verification, or server lifecycle while
  changing transport names.
- Do not preserve undocumented legacy aliases; this is the requested breaking package rework.
- Do not wholesale rebaseline behavioral goldens. Correct only expectations proven changed by the
  public contract; regenerate byte-identity sentinels mechanically.

## 2. Current State

There are 231 non-golden files mentioning `net::` or `tls::` (`rg -l 'net::|tls::' src tests scripts
--glob '!**/golden/**' | wc -l`). HTTP itself uses both transports in 18 files (`rg -l 'net::|tls::'
src/codegen/builtins/http --glob '*.rs' | wc -l`): its `Stream` union currently names
`net::Socket`/`tls::TlsSocket`, server functions return the old listeners, and helpers call old
read/write/poll APIs. The exact census must be re-run after A–D because those letters add matches.

Verified by reading HTTP helpers: it depends on byte reads, String writes, scalar poll, connect,
listen/accept, and close/drop identities. URL parsing remains in net and should not move.

## 3. Design Overview

First regenerate a symbol-by-symbol census partitioned into semantic consumers, examples/man
descriptors, test sources, scripts, specs, and generated goldens. Migrate production consumers and
their tests together, then remove net TCP/UDP descriptors/resources/helpers and legacy TLS names.
Keep internal runtime symbols stable only when harmless; public names and qualified type identities
must be exact. Sweep both man and spec citations after every move.

Expected diffs: all fixtures importing old transport APIs, HTTP injected source/IR/native output,
resource metadata, and package audit listings. Pure URL/query fixtures should remain behaviorally
unchanged. An unexpected unrelated golden diff triggers root-cause inspection, never a rollback of
the correct API.

## Phases

### Phase 1 — Fresh census and HTTP migration

- [x] Re-run `rg -l`/`rg -n` census after plan 110-D and record exact per-symbol counts here before
      scheduling edits; include source, tests, scripts, specs, planning citations, and man examples.

      **Census, measured 2026-08-30** (file counts, `grep -rlE 'net::<sym>([^A-Za-z0-9_]|$)'`).
      The boundary is load-bearing: an unanchored `net::read` also matches `net::readText` and
      `net::write` matches `net::writeText`, which inflated the first run's `read` from 14 to 18
      source files and its tests from 3 to 10. Goldens are listed separately because they are
      regenerated, never edited.

      | symbol | src | tests | scripts | examples | plan/bugs | goldens |
      |---|---|---|---|---|---|---|
      | `net::Socket` | 18 | 27 | 0 | 0 | 24 | 1 |
      | `net::Listener` | 7 | 5 | 0 | 0 | 13 | 0 |
      | `net::connectTcp` | 24 | 26 | 3 | 0 | 34 | 3 |
      | `net::listenTcp` | 20 | 25 | 0 | 0 | 28 | 1 |
      | `net::accept` | 14 | 20 | 0 | 0 | 24 | 1 |
      | `net::read` | 14 | 3 | 0 | 0 | 20 | 1 |
      | `net::readText` | 11 | 8 | 0 | 0 | 16 | 1 |
      | `net::write` | 8 | 5 | 0 | 0 | 10 | 1 |
      | `net::writeText` | 14 | 11 | 0 | 0 | 15 | 1 |
      | `net::poll` | 15 | 7 | 0 | 0 | 20 | 2 |
      | `net::close` | 11 | 6 | 0 | 0 | 18 | 1 |
      | `net::localAddress` | 14 | 24 | 0 | 0 | 24 | 1 |
      | `net::remoteAddress` | 4 | 3 | 0 | 0 | 7 | 1 |
      | `net::setReadTimeout` | 7 | 5 | 0 | 0 | 19 | 1 |
      | `net::setWriteTimeout` | 5 | 4 | 0 | 0 | 14 | 1 |

      Totals: **270 non-golden files** mention `net::` or `tls::`
      (`grep -rln 'net::\|tls::' src tests scripts examples | grep -v /golden/ | wc -l`), and
      **20 files** under `src/codegen/builtins/http` do
      (`grep -rln 'net::\|tls::' src/codegen/builtins/http --include='*.rs' | wc -l`) — up from
      the 231/18 the plan recorded before A–D, as it predicted.

      HTTP's own dependence on the stream surface is **40 references across 11 files**
      (`grep -rnE "net::(Socket|Listener|connectTcp|listenTcp|accept|read|readText|write|writeText|poll|close|localAddress|remoteAddress|setReadTimeout|setWriteTimeout)([^A-Za-z0-9_]|$)" src/codegen/builtins/http`):
      `func_read.rs`, `helper_read_net.rs`, `func_respond_path.rs`, `func_start_read.rs`,
      `func_ready.rs`, `mod.rs`, `func_handle_request.rs`, `helper_wait_readable.rs`,
      `func_server.rs`, `helper_start_exchange.rs`, `func_pump.rs`. That list is Phase 1's
      work-list and the acceptance grep's target.
- [x] Migrate `src/codegen/builtins/http/` Stream variants, server return types, helpers, docs, and
      imports to tcp/tls while leaving net Url/query calls intact.
      11 files, 40 references. The `Stream` union's variants are now `tcp::Socket | tls::Socket`;
      `http::server` returns a `tcp::Listener`; every helper calls `tcp::` for transport.
      `net` is retained in `add_imports` and in the examples that use it, because the URL/query
      surface (`net::Url`, `net::toUrl`, `net::percentDecode`, `net::parseQuery`) did not move.
      `net::writeText` folded into `tcp::write`'s String overload. Three doc examples gained
      `IMPORT tcp`, since an example naming `tcp::Listener` without it would not compile for a
      reader who copied it.
- [~] Update HTTP runtime/syntax tests to prove plaintext and TLS client/server/async workflows,
      including buffered TLS readiness and connect/read timeout behavior.
      Proven live on all three platforms with a probe covering the blocking client over plaintext
      AND TLS, the async `Stream` drive loop (`startRead`/`ready`/`pump`/`done`/`finish`), and the
      server bind — `plain status=200 / tls status=200 / async status=200 / bound=TRUE`, identical
      on macOS aarch64, Alpine x86_64 musl (2227) and Windows 11 (2230). Remaining: land that probe
      as a committed fixture, and the existing http fixtures' golden refresh.

Acceptance: HTTP acceptance and standalone runtime tests pass using only tcp/tls transport symbols;
`rg -n 'net::(Socket|Listener|connectTcp|listenTcp|accept|read|write|poll|close)' src/codegen/builtins/http`
returns no matches. **The grep half is MET** — measured 2026-08-30, 0 matches for the full
15-symbol stream surface (not just the 9 the criterion names), anchored so `net::read` does not
mask `net::readText`. The test half lands with the fixture in the next commit.
Commit: —

### Phase 2 — Remaining first-party consumers

- [ ] Migrate every compiler-owned source, acceptance source, runtime/syntax fixture, integration
      test, script, audit capability expectation, and example from the fresh census.
- [ ] Rename/move fixtures from net to tcp/udp/tls buckets where their behavior lives; preserve each
      behavioral assertion and use `git log -S`/blame before changing any disputed expected line.
- [ ] Update timeout blackhole tooling and byte-identity fixture sources.

Acceptance: `rg` finds old symbols only in historical plans/bugs and explicit negative migration
tests; all migrated runtime tests retain their prior observable behavior.
Commit: —

### Phase 3 — Remove legacy surface

- [ ] Remove net Socket/Listener/~~UdpSocket/Datagram/DatagramText~~ records/resources, transport
      descriptors, compatibility shims, aliases, cleanup/type recognition, and now-dead code.
      **The datagram half is already done** — `UdpSocket`, `Datagram`, `DatagramText`, `bindUdp`,
      `sendTo`, `sendTextTo`, `receiveFrom`, `receiveTextFrom` and the `UdpSocket` overloads were
      removed by plan-110-C (commit 2dd8f30fd), which was forced to absorb this slice: two packages
      cannot both declare a record named `Datagram`, so `udp` could not be added while net's
      remained. See plan-110-C §C1. What is left here is the **stream** half: net's `Socket`,
      `Listener`, `connectTcp`, `listenTcp`, `accept`, `read`, `readText`, `write`, `writeText`,
      `poll`, `close`, `localAddress`, `remoteAddress`, `setReadTimeout`, `setWriteTimeout`.
- [ ] Physically move the shared native emitters into `tcp/` and `udp/` (deferred here from
      plan-110-B §C2 and plan-110-C): `tcp` and `udp` currently lower through the
      `lower_net_*_helper` emitters in `net::{gen_shared, gen_io, gen_poll}`. Once net's stream
      descriptors are deleted, split them so each package owns its own, leaving `net` only the
      resolver/address/URL emitters `lookup` and `ping` still need. Doing it here rather than in
      B/C means editing those ~2,700 lines once instead of twice.
- [ ] Remove `TlsSocket`/`TlsListener`, readText/writeText, and old aliases after confirming zero
      live consumers; keep only the exact requested package surfaces.
- [ ] Add negative syntax tests proving legacy calls/types are no longer exported.

Acceptance: `mfb man net --all`, `tcp --all`, `udp --all`, and `tls --all` enumerate exactly the
requested public members/types (plus the established `toString(net::Url)` general override), and
legacy source fails with a precise unknown-member/type diagnostic.
Commit: —

## Validation Plan

Run `rustup run 1.96.0 cargo test`, every migrated real runtime proof, full acceptance, audit tests,
and artifact gates. Use `git diff --word-diff` on behavioral goldens and the approved regeneration
scripts for `.ncode`/`.ncodesum`. Update all descriptor docs and embedded stdlib topics; run citation
tests and render all four man packages. Run both required rustfmt commands.

## Open Decisions

- Compatibility window — recommend none after this letter: A–D provide internal staging, while E
  deliberately removes public legacy names to meet the exact requested surface.

## Corrections

To be filled during execution, including the mandatory post-D census.

## Summary

This is the largest breadth/churn letter but should add no new transport semantics. HTTP is migrated
first because it is the densest production consumer and proves the new package composition.
