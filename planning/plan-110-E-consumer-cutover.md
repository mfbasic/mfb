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
| plan-110-D complete | `ls planning/plan-110-D-* 2>/dev/null` returns no matches | NOT MET |
| New packages pass their standalone runtime gates | `rustup run 1.96.0 cargo test` plus B/C/D runtime commands | NOT MET |

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

- [ ] Re-run `rg -l`/`rg -n` census after plan 110-D and record exact per-symbol counts here before
      scheduling edits; include source, tests, scripts, specs, planning citations, and man examples.
- [ ] Migrate `src/codegen/builtins/http/` Stream variants, server return types, helpers, docs, and
      imports to tcp/tls while leaving net Url/query calls intact.
- [ ] Update HTTP runtime/syntax tests to prove plaintext and TLS client/server/async workflows,
      including buffered TLS readiness and connect/read timeout behavior.

Acceptance: HTTP acceptance and standalone runtime tests pass using only tcp/tls transport symbols;
`rg -n 'net::(Socket|Listener|connectTcp|listenTcp|accept|read|write|poll|close)' src/codegen/builtins/http`
returns no matches.
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

- [ ] Remove net Socket/Listener/UdpSocket/Datagram/DatagramText records/resources, transport
      descriptors, compatibility shims, aliases, cleanup/type recognition, and now-dead code.
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
