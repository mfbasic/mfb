# plan-110-F: Network cross-target certification and closeout

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-110-E

Certify the completed net/tcp/udp/tls rework on every supported runtime family, reconcile only
expected generated artifacts, and finish embedded specifications/man output. The outcome is not
merely green compilation: real DNS, ICMP, TCP, UDP, TLS client/server/wrap, and HTTP behavior is
observed on the supported targets.

References: plan-110-E; `.ai/testing-gates.md`; `.ai/remote_systems.md`;
`.ai/specifications.md`; `.ai/man-content.md`; `.ai/build-tooling.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-110-E complete | `ls planning/plan-110-E-* 2>/dev/null` returns no matches | NOT MET |
| No legacy live API references remain | exact rg commands from plan-110-E Phase 3 | NOT MET |
| Full local suite green | `rustup run 1.96.0 cargo test` | UNVERIFIED |

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
generated test CA/server identities, TLS direct and wrap flows, and an isolated permission-denied
ICMP environment. Run it natively per target. Keep live external connectivity only as an additional
signal. Serialize acceptance/golden commands that share output directories.

Expected codegen drift includes new packages/types/helpers and every deliberately migrated
fixture. Root-cause any other diff by objdumping one fixture, then fix the implementation or correct
the prediction before regeneration.

## Phases

### Phase 1 — Deterministic certification harness

- [ ] Recount all networking fixtures/artifacts with commands and record the post-migration matrix
      in Corrections; ensure every requested overload has valid and invalid coverage.
- [ ] Add/finish reusable local peer scripts for TCP blackhole, UDP echo, TLS CA/client/server/wrap,
      and ICMP permission denial; no mocks or public-network dependency for required proof.
- [ ] Prove the harness itself fails when one expected response/status/certificate is deliberately
      wrong, then restore it.

Acceptance: one documented command per protocol yields stable observable results and detects an
injected wrong result.
Commit: —

### Phase 2 — Native target matrix

- [ ] Run macOS console proofs locally, Windows proofs on the configured Windows system, and Linux
      glibc/musl architecture proofs according to `.ai/remote_systems.md`; record exact commands and
      results in the phase ledger.
- [ ] Verify timeout elapsed bounds, ICMP permission Error, IPv4 and available IPv6 Address values,
      resource cleanup, TLS certificate rejection, and wrap on each backend.
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

To be filled during execution with the post-E census, target commands, and every corrected premise.

## Summary

This letter prevents a broad networking rewrite from being declared complete on compiler proxies.
Its hard gate is real native behavior, especially ICMP permissions and TLS wrap on each backend.
