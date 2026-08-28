# plan-110-B: TCP package extraction

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-110-A

Create the `tcp` package and move the plaintext stream/listener contract out of `net` while
temporarily retaining legacy `net` entry points as migration shims until plan 110-E. The outcome is
that every requested `tcp::*` overload runs with `tcp::Socket`/`tcp::Listener` ownership and the
same proven OS behavior as today's net implementation.

References: plan-110-A; `.ai/compiler.md`; `.ai/resources-packages.md`; `.ai/net-tls.md`;
`src/codegen/builtins/net/{mod.rs,gen_shared.rs,gen_io.rs,gen_poll.rs}`;
`src/syntaxcheck/builtins.rs:BUILTIN_ARG_MODES`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-110-A archived/completed | `ls planning/plan-110-A-* 2>/dev/null` returns no matches | NOT MET |
| Full Rust suite green | `rustup run 1.96.0 cargo test` | UNVERIFIED |

## 1. Goal

- Add the exact `tcp` signatures from the request, including scalar/list poll and String/List Byte
  write overloads, backed by real native execution on every supported target.

### Non-goals

- Do not change timeout conventions, short-read behavior, full-write behavior, backlog semantics,
  or record layout merely because the package/type names change.
- Do not delete legacy `net` members in this letter; plan 110-E owns the atomic consumer cutover.
- Do not keep `readText`/`writeText`: String is an overload of `write`; `read` remains bytes only.

## 2. Current State

The current surface is `net::{connectTcp,listenTcp,accept,read,readText,write,writeText,close,
poll,localAddress,remoteAddress,setReadTimeout,setWriteTimeout}` with resources `net.Socket` and
`net.Listener` (`src/codegen/builtins/net/mod.rs:register`). Native code is split across 18
consumer files outside net/tls that mention these APIs (`rg -l 'net::|tls::' src/codegen/builtins/http
--glob '*.rs' | wc -l` gives 18 for the HTTP package as a whole); plan 110-E migrates them.

Verified by reading `src/syntaxcheck/builtins.rs`: `net.close` is special-cased as consuming, so
`tcp.close` must be registered there. Verified by reading `net/gen_poll.rs`: list poll returns a
borrowed resource identity and therefore must be retyped, not just renamed at documentation level.

## 3. Design Overview

Add `src/codegen/builtins/tcp/` with descriptors and native emitters moved from the TCP portions of
net. Give resources qualified identities `tcp.Socket` and `tcp.Listener`, close functions
`tcp.close` and an internal listener-shaped alias where required. Retain backend record layout and
resource tags unless the registry proves a tag must change. Legacy net descriptors may delegate to
the same lowerers during the transition, but must return legacy resource identities; no unsafe
cross-identity substitution.

This is behavior-changing at the public API and expected to change TCP/package fixtures on all
targets. Core syscall instruction sequences should remain equivalent; any unexpected semantic
golden diff is localized with one objdump before proceeding.

## Phases

### Phase 1 — Package/resource seam

- [ ] Register `tcp` in `src/codegen/builtins/mod.rs` and `src/codegen/registry/mod.rs`; add package
      argument inference and consuming `tcp.close` handling in `src/syntaxcheck/builtins.rs`.
- [ ] Define `tcp.Socket` and `tcp.Listener` resources, cleanup functions, sendability, runtime tags,
      and verifier/link/binary-representation recognition without changing net identities.
- [ ] Add registry/unit tests proving qualified type lookup and lexical-drop close routing.

Acceptance: a minimal package can declare/drop each tcp resource, and move-after-close is rejected.
Commit: —

### Phase 2 — Constructors, endpoints, close, timeouts

- [ ] Move/share native lowerers for `listen`, `accept`, both `connect` overload families,
      `localAddress`, `remoteAddress`, `close`, and timeout setters into tcp-owned files.
- [ ] Ensure Address parameters/returns use `net.Address`; `connect(address, timeoutMs)` uses the
      shared value without introducing a tcp Address duplicate.
- [ ] Tests: valid/invalid fixtures for every overload and real loopback timeout/address behavior.

Acceptance: a tcp loopback server connects, reports both endpoints, times out according to the
language convention, and closes exactly once under explicit close and lexical drop.
Commit: —

### Phase 3 — I/O and readiness

- [ ] Implement byte `read`, byte/String `write`, scalar listener/socket poll, list socket poll, and
      full-write/EOF/error semantics using tcp-owned descriptor names and OS aliases.
- [ ] Retarget `scripts/check-net-connect-timeout.sh` to tcp and rename it consistently; preserve its
      real blackhole deadline proof and update `scripts/README.md`.
- [ ] Tests: cover all overloads, partial writes, maxBytes boundaries, scalar/list readiness,
      timeout 0/positive/negative, closed handles, and empty-list rejection.

Acceptance: loopback transfers binary and UTF-8 String payloads losslessly, poll returns the exact
ready borrowed socket, and the blackhole script proves the requested deadline.
Commit: —

## Validation Plan

Run full `cargo test`, tcp runtime fixtures, acceptance, artifact gates and native target proofs.
Regenerate expected codegen drift only. Update descriptor man content and the embedded stdlib spec.
Run both required rustfmt commands after Rust edits.

## Open Decisions

- Migration shims — recommend internal/shared lowerers plus public legacy net descriptors until
  plan 110-E, because an intermediate commit must keep HTTP and all fixtures buildable.

## Corrections

To be filled during execution.

## Summary

The risk is qualified resource identity and cleanup, not the already-proven socket syscalls. This
letter lands tcp without prematurely breaking current HTTP/net consumers.
