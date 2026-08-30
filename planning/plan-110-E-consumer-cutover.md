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
- [x] Update HTTP runtime/syntax tests to prove plaintext and TLS client/server/async workflows,
      including buffered TLS readiness and connect/read timeout behavior.
      `tests/rt-behavior/http/http-tcp-transport-rt` covers the blocking client over plaintext AND
      TLS, the async `Stream` drive loop (`startRead`/`ready`/`pump`/`done`/`finish`) and the server
      bind — `plain status=200 / tls status=200 / async status=200 / bound=TRUE`, identical on
      macOS aarch64, Alpine x86_64 musl (2227) and Windows 11 (2230). `http_server_loopback` and
      the `byte-identity/http` corpus migrated with it. Buffered TLS readiness is already pinned by
      `tls-poll-rt`/`tls-poll-list-rt`, and the connect/read timeout convention by
      `tls-read-timeout-rt` and `tls-timeout-convention-rt` (plan-110-D).

Acceptance: HTTP acceptance and standalone runtime tests pass using only tcp/tls transport symbols;
`rg -n 'net::(Socket|Listener|connectTcp|listenTcp|accept|read|write|poll|close)' src/codegen/builtins/http`
returns no matches. **MET** — measured 2026-08-30: 0 matches for the full 15-symbol stream surface
(not just the 9 the criterion names), anchored so `net::read` does not mask `net::readText`.
Gates: acceptance 1298 tests / 0 mismatches; `artifact-gate.sh all` 1283 tests, 1440 builds,
1772 goldens, 0 diffs; `cargo test --bin mfb` 3392 passed.
Commit: 5629fc9bb, and the type-confusion fix below

### Phase 2 — Remaining first-party consumers

- [x] Migrate every compiler-owned source, acceptance source, runtime/syntax fixture, integration
      test, script, audit capability expectation, and example from the fresh census.
      24 fixture sources, 2 package sources (`tools/thread-package-sources/`), the
      `rt_macos_d4_union_state_tls` integration test, the audit-capability fixture, and the
      blackhole tooling's docs. `scripts/check-tcp-connect-timeout.sh` was already on `tcp::connect`
      from plan-110-B and still passes. Deliberately NOT migrated: the ~20 remaining `net::`
      mentions in compiler sources are prose citing net's behaviour as precedent, accurate until
      Phase 3 removes the surface.
- [x] Rename/move fixtures from net to tcp/udp/tls buckets where their behavior lives; preserve each
      behavioral assertion and use `git log -S`/blame before changing any disputed expected line.
      **36 fixtures moved** with `git mv` (so history follows them): 20 rt-behavior, 15 syntax,
      1 rt-error. `net` keeps exactly the 10 that test what it retains — lookup, ping, parseQuery,
      percentDecode, toUrl (×2), url_toString, decode, ping_range. No assertion was dropped; the
      two `readText` fixtures keep theirs by decoding explicitly
      (`encoding::utf8Decode(tcp::read(..))`), since `tcp::read` is bytes-only by design.
- [x] Update timeout blackhole tooling and byte-identity fixture sources.
      `scripts/net_blackhole_server.py`'s contract line now names `tcp::connect`;
      `scripts/check-tcp-connect-timeout.sh` was already migrated by plan-110-B and re-verified
      green here (`PASS: tcp::connect timed out with ErrTimeout`). `byte-identity/http` migrated;
      `byte-identity/net` deliberately keeps net's stream corpus until Phase 3 deletes the surface
      it covers.

Acceptance: `rg` finds old symbols only in historical plans/bugs and explicit negative migration
tests; all migrated runtime tests retain their prior observable behavior. **MET for the consumer
half** — every migrated fixture keeps its prior observable behaviour (acceptance 1299 tests,
0 mismatches, and no `.run` golden or exit code moved anywhere in the migration). The `rg` half
completes with Phase 3, which removes the surface the remaining hits describe.
Commit: 9b62dcf23, and the fixture move below

### Phase 3 — Remove legacy surface

- [x] Remove net Socket/Listener/~~UdpSocket/Datagram/DatagramText~~ records/resources, transport
      descriptors, compatibility shims, aliases, cleanup/type recognition, and now-dead code.
      **The datagram half is already done** — `UdpSocket`, `Datagram`, `DatagramText`, `bindUdp`,
      `sendTo`, `sendTextTo`, `receiveFrom`, `receiveTextFrom` and the `UdpSocket` overloads were
      removed by plan-110-C (commit 2dd8f30fd), which was forced to absorb this slice: two packages
      cannot both declare a record named `Datagram`, so `udp` could not be added while net's
      remained. See plan-110-C §C1. What is left here is the **stream** half: net's `Socket`,
      `Listener`, `connectTcp`, `listenTcp`, `accept`, `read`, `readText`, `write`, `writeText`,
      `poll`, `close`, `localAddress`, `remoteAddress`, `setReadTimeout`, `setWriteTimeout`.
- [x] Physically move the shared native emitters into `tcp/` and `udp/` (deferred here from
      plan-110-B §C2 and plan-110-C): `tcp` and `udp` currently lower through the
      `lower_net_*_helper` emitters in `net::{gen_shared, gen_io, gen_poll}`. Once net's stream
      descriptors are deleted, split them so each package owns its own, leaving `net` only the
      resolver/address/URL emitters `lookup` and `ping` still need. Doing it here rather than in
      B/C means editing those ~2,700 lines once instead of twice.
      Done: `net::gen_shared`/`gen_poll` became `codegen::os::socket::{shared, poll}` (the
      platform-neutral sockaddr/handle/pollfd primitives all three transports share, which belong
      to no one package); the stream I/O emitters moved to `tcp/gen_io.rs` (967 lines:
      accept/read/write) and the datagram ones to `udp/gen_io.rs` (883: bind/receive/send), leaving
      `net/gen_io.rs` at ~295 lines holding only the resolver. `artifact-gate all` proved the
      ~3,400-line move byte-identical: 1284 tests, 1441 builds, 1772 goldens, 0 diffs. The sweep it
      forced then found two live defects and the dead call tables -- Corrections C4-C6.
- [x] Remove `TlsSocket`/`TlsListener`, readText/writeText, and old aliases after confirming zero
      live consumers; keep only the exact requested package surfaces.
      Done by plan-110-D: `TlsSocket`/`TlsListener` became `tls::Socket`/`tls::Listener`,
      `tls::readText` and `tls::writeText` were removed (write took a String overload), and
      3016059f5 swept the residue — the dead `text` parameter through all three read emitters, the
      `tls.readText` data object and validate-utf8 trigger, and the package prose still promising
      "paired byte/text forms". `net::readText`/`writeText` go with net's stream surface above.
      Verified: `mfb man net` lists exactly 5 members, `mfb man tls` lists no `wrap`.
- [x] Add negative syntax tests proving legacy calls/types are no longer exported.
      `tests/syntax/net/net_stream_surface_removed_invalid` pins all 15 removed stream names plus
      the two resource types, and the three datagram members plan-110-C removed, so one fixture
      covers the whole transport removal: 20 diagnostics, every one
      "Built-in package `net` does not export ...". A surface that still resolves is a surface that
      still exists, and this is what proves it does not.

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

**C1 — the post-D census's own first run was wrong (Phase 1, commit 32d2ee215).**
An unanchored `net::read` also matches `net::readText`, and `net::write` matches
`net::writeText`, which inflated `read` from 14 source files to 18 and its tests from 3 to 10.
Every symbol is now anchored with a trailing non-identifier boundary
(`rg -l 'net::read[^A-Za-z]'`), and the wrong numbers are recorded beside the right ones in the
census table rather than quietly replaced. Post-D totals moved as the plan predicted: **270**
non-golden files mention `net::` or `tls::` (§2 said 231) and **20** under `builtins/http` (§2 said
18) — `rg -l 'net::|tls::' src tests scripts --glob '!**/golden/**' | wc -l`.

**C2 — a prerequisite the plan did not list: two built-in resources with the same bare name were
the same type (commit f670ec6f8).** plan-110 gave `net`, `tcp`, `udp` and `tls` resources with
IDENTICAL bare names (`Socket`, `Listener`), and `ir::verify::compat::compatible()`'s bare-name
fallback then equated them, so `RES s AS udp::Socket = tcp::accept(server, 0)` compiled clean. The
fallback was correct when written — bare names were globally unique — so this is a hole plan-110
opened and plan-110 closes. Narrowed to *built-in resources* after a first, too-broad attempt
("two differing qualified names are different types") was caught by
`syntax/packages/package-comparable-import-invalid`, where `comparable::Box` and
`package_comparable_types.Box` are the same type under an import alias. The tightening then found
two latent wrong annotations that had been compiling silently — `byte-identity/http` and
`http_server_loopback` both bound `net::Listener` from `http::server`, which returns a
`tcp::Listener`. Pinned by `tests/syntax/tcp/resource_bare_name_confusion_invalid` (six shapes,
both directions).

**C3 — two mistakes in the Phase 2 fixture move, corrected rather than shipped (commit
9a194a716).** (a) The mover dropped `IMPORT net` from any source that stopped naming a `net::`
symbol — but `Address` is net's RECORD, so a fixture reading `bound.port` still needs net in scope.
13 fixtures failed with "native plan has no storage class for type 'Unknown'", an error naming
neither the import nor the type; 16 sources got the import back. (b) An earlier over-broad pass also
rewrote net's OWN fixtures and an `_invalid` fixture whose whole point is that `readText` is
rejected; reverted, because those belong to Phase 3, which deletes the surface they cover.

**C4 — Phase 3's first box under-counted "now-dead code": the runtime-call tables.** Deleting net's
stream descriptors left the *call names* behind in six places that no longer had anything to name —
`SUPPORTED_RUNTIME_CALLS` on all three backends (`linux_common`, `macos_aarch64`, `win_x86_64`:
18/18/19 rows), the error-message pool trigger in `codegen/memory/data/data_objects.rs` (18 rows),
the libc-symbol table `target::shared::plan::net_libc_symbols` (15 arms), and four
`builder_values.rs` padding/code-form dispatch arms. plan-110-B had anticipated exactly this: it
spelled tcp's `net_libc_symbols` rows out rather than aliasing them to net's "so that deleting net's
transport surface in plan-110-E cannot silently empty them." Measured dead, not assumed:
`target::shared::runtime::spec_for_call("net.write")` returns `None`, which is what reds
`target::linux_x86_64::plan::tests::write_is_never_imported`.

**C5 — the sweep found a live resource-lifetime bug: `poll(List)` double-closed its borrowed
element (bugs/completed/bug-458).** `CodeBuilder::value_aliases_live_resource` — bug-375's
classifier — matched only `NirValue::Call`/`CallResult`, but a built-in package member lowers to
`NirValue::RuntimeCall`. The `net.poll` and `tls.poll` names it listed were therefore never reached:
dead conditions that read as a fix. Every `RES ready = tcp::poll(socks)` was classified as an OWNER,
closing the borrowed element at the bind's scope exit and again when the list drained. Proven by
A/B, macOS aarch64, `tests/rt-behavior/tcp/tcp-poll-list-rt` copied to `/tmp/pollprobe`: adding
`"tcp.poll"` to the name list alone changed nothing (`diff` of `mfb build -ncode` output = 0 lines);
adding the `RuntimeCall` variant removed **315** lines of `.ncode` — the four binds' close cleanups.
`udp.poll`, which never had a name arm at all, is added for the same contract. The 1200-iteration
leak loop in `tcp-poll-list-rt` did not catch it because the two closes are adjacent, so the second
`close(2)` just returns `EBADF`; the guard is therefore the codegen-inspection unit test
`borrowed_resource_tests::poll_list_forms_alias_a_live_resource`.

**C6 — the same sweep found the audit blind to `tcp` and `udp`.** `audit::collect::source` mapped
only `"net" | "tls" | "http"` to the `network` capability and had zero `tcp.*`/`udp.*` rows in
`resource_producer`, so since plan-110-B/C a tcp-only program disclosed **no** network capability
and every `tcp::connect`/`tcp::listen`/`tcp::accept`/`udp::bind` handle was missing from the audit's
Resources section and its close-may-fail findings — the exact class of bug-96 and bug-278.
`http.server` was still mapped to `("Listener", "net.close")` and now names the `tcp::Listener` it
actually returns.

**C7 — `mfb man tcp poll` renders `List OF RES tcp::Socket`, not `List OF tcp::Socket`.**
`cli::man::tests::function_types_use_public_package_qualification` expected the latter because net's
descriptor omitted the `RES`, which no source spelling of a resource list may do (§15.6). tcp's
descriptor is the more correct one; the assertion was corrected to the real render, and the
qualification the test guards is unaffected.

## Summary

This is the largest breadth/churn letter but should add no new transport semantics. HTTP is migrated
first because it is the densest production consumer and proves the new package composition.
