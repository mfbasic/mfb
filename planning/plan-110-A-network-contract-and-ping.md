# plan-110-A: Network contract foundation and ping

Last updated: 2026-08-27
Overall Effort: huge (>3d)
Effort: large (3h–1d)
Depends on: nothing

Establish the shared `net` value contract and implement real ICMP echo so the later
transport-package moves build on tested `net::Address`, `net::Url`, `net::PingStatus`, and
`net::PingResult` identities. The checkable outcome is that both `net::ping` overloads execute
on supported targets, report the specified status/result fields, and raise a normal runtime
error when the OS denies ICMP permission.

References:

- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/arch-abi.md`
- `.ai/resources-packages.md`, `.ai/net-tls.md`, `.ai/testing-gates.md`
- `.ai/man-content.md`, `.ai/specifications.md`, `.ai/build-tooling.md`
- `src/codegen/builtins/net/mod.rs:register`
- `src/codegen/builtins/process/mod.rs:register` (registry-enum precedent)

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| The existing Rust suite is green | `rustup run 1.96.0 cargo test` | UNVERIFIED — re-run before implementation |
| No unfinished plan already owns the networking surface | `rg -n 'net::ping|tcp::|udp::|tls::wrap' planning/plan-*.md` | MET — only this plan family after authoring |

Everything below assumes those checks are green. The status is a snapshot; re-run the commands.

## 1. Goal

- Preserve `net::lookup`, `parseQuery`, `percentDecode`, and `toUrl`; add production ICMP ping
  with host and `net::Address` overloads and the exact result/status contract in the request.

### Non-goals

- Do not implement ping with TCP connect, a subprocess, a mock, or a platform-unsupported stub.
- Do not bound DNS lookup with `timeoutMs`; the established connect convention resolves first.
- Do not retain TCP/UDP resources in `net` after plan 110-E.
- Do not treat generated-byte identity as a constraint: new types and runtime helpers are expected
  to change net fixtures on every supported target.

## 2. Current State

`net` currently registers 3 records and 3 resources together with URL, DNS, TCP, and UDP members
in `src/codegen/builtins/net/mod.rs:register`. `Address` is `{host String, port Integer}` and `Url`
has eight fields in registry order. No ICMP/ping implementation exists: `rg -n
'ICMP|SOCK_RAW|net::ping' src tests` returns no networking implementation matches. Registry enums
already render from descriptors (`src/codegen/builtins/process/mod.rs:register`).

### Measured populations

| What | Count | Command |
|---|---:|---|
| Files in current net implementation | 35 | `find src/codegen/builtins/net -type f \| wc -l` |
| Current net/tls fixture source files | 74 | `find tests/rt-behavior/net tests/rt-error/net tests/syntax/net tests/rt-behavior/tls tests/rt-error/tls tests/syntax/tls -type f -name main.mfb 2>/dev/null \| wc -l` |
| Non-golden source/test/script files mentioning net or tls APIs | 231 | `rg -l 'net::|tls::' src tests scripts --glob '!**/golden/**' \| wc -l` |

### Verified properties

- `net::Address` already crosses native helpers as a record whose field order is ABI-relevant;
  verified by reading `net::register` and `src/codegen/builtins/net/gen_shared.rs`.
- Resource ownership is not descriptor-generic for ordinary consuming calls; close consumption is
  selected in `src/syntaxcheck/builtins.rs:net_consumes_argument`. `tls::wrap` therefore needs an
  explicit ownership task in plan 110-D.
- ICMP permission differs by OS and deployment configuration. The contract must translate the
  actual permission failure into an Error, not `Unreachable` and not a fake PingResult.

## 3. Design Overview

Keep `Address`, `Url`, URL helpers, and DNS in `net`. Add registry `PingStatus` variants in the
declared order `Ok`, `Timeout`, `Unreachable`, `TtlExceeded`, plus `PingResult` in the exact field
order `status,address,rttMs,ttl,size`. Implement one ping ABI-function family with host/address
aliases and POSIX/Windows backends. Use OS ICMP echo facilities appropriate to each platform;
parse matching echo replies and ICMP errors, reject invalid timeout/TTL/size before system calls,
and use monotonic elapsed time. `address.port` is ignored for ICMP and the responder record uses
port 0; document that explicitly.

Correctness risk concentrates in packet parsing, identifiers/checksums, timeout accounting, and
Windows/macOS/Linux API differences. Design uncertainty is whether unprivileged ICMP is available
on every supported runner; Phase 1 records a capability matrix without weakening behavior.

This is behavior-changing work. `.ncode`/`.ncodesum` changes are expected for ping fixtures and
package metadata; unexpected diffs in unrelated fixtures trigger one-fixture objdump diagnosis.

Rejected: shelling out to `ping` (not portable, injectable, and not a runtime primitive); raw-only
sockets (needlessly require privilege where datagram ICMP APIs exist); returning a permission
status (the requested contract explicitly says Error).

## 4. Public value and error contract

Defaults follow the shared timeout convention: omitted `timeoutMs` is unbounded, `0` is one
immediate attempt, positive is a deadline, negative errors. Recommend defaults `ttl=64` and
`size=56`; validate TTL `1..=255` and a documented payload maximum before allocation. `Ok` carries
measured values; every non-Ok status zeroes `rttMs`, `ttl`, and `size`. Name resolution/system
errors remain Errors. An echo timeout is `PingStatus::Timeout`, while failure to create/use the
ICMP facility due to access control is an Error.

## Phases

### Phase 1 — Prove platform facilities and freeze semantics

- [ ] Record the ICMP facility, privilege behavior, reply-TTL source, and maximum payload for
      macOS AArch64, Linux x86_64 glibc/musl, Linux AArch64, Linux riscv64, and Windows x86_64 in
      this plan's Corrections section, citing SDK headers/man pages and a minimal runtime probe.
- [ ] Resolve the two Open Decisions below before adding descriptors; update this plan with the
      chosen constants and exact errors.

Acceptance: a checked-in contract table names a real implementation route for every supported
target; permission-denied is reproducible or its OS error mapping is unit-tested from the native
constant.
Commit: —

### Phase 2 — Registry contract and frontend

- [ ] Add `PingStatus`, `PingResult`, and both `ping` implementations under
      `src/codegen/builtins/net/`; preserve the existing Address/Url layouts.
- [ ] Add argument normalization, return typing, enum/record source injection, and errors; audit
      AST, HIR, IR, link verifier, resource, and binary-representation seams for the new names.
- [ ] Tests: add valid and invalid fixtures under `tests/rt-behavior/net/`,
      `tests/rt-error/net/`, and `tests/syntax/net/`, covering both overloads and all defaults.

Acceptance: `mfb man net ping` and a fixture compile to the exact requested signatures; invalid
arity/types/ranges fail with the specified diagnostics.
Commit: —

### Phase 3 — Native ICMP execution

- [ ] Implement packet construction, monotonic deadline, reply/error parsing, and OS error mapping
      in per-platform emitters, preserving caller-saved register and stack-alignment invariants.
- [ ] Add deterministic parser/checksum/unit tests plus loopback runtime tests for host and Address.
- [ ] Add a permission-denial runtime test using an isolated test environment that actually denies
      ICMP socket creation; do not accept a mocked errno as end-to-end proof.

Acceptance: loopback returns `Ok` with the responder, positive elapsed/TTL/size values; a silent
address returns `Timeout`; denied permission raises Error; malformed/unrelated replies are ignored.
Commit: —

## Validation Plan

- Run `rustup run 1.96.0 cargo test` (the required full suite), targeted runtime fixture executables,
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`, and the relevant cross-target
  artifact/runtime gates in `.ai/testing-gates.md`.
- Regenerate only expected `.ncode`/`.ncodesum` drift with the repository scripts and inspect one
  fixture per changed backend; never rebaseline behavioral expectations.
- Update registry descriptor docs and `src/docs/spec/stdlib/` networking contract in the same
  phase; verify `mfb man` and `mfb spec` citations/rendering.
- After Rust changes run `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- Ping defaults — recommend `ttl=64`, `size=56`; record these as public contract before coding.
- `Address.port` for ICMP — recommend ignore input and return responder port `0`, because ICMP has
  no transport port; rejecting nonzero would make lookup-produced addresses awkward.

## Corrections

To be filled during execution.

## Summary

This letter isolates the only wholly new protocol and freezes shared value semantics before TCP,
UDP, TLS, and consumers move. Packet parsing and real permission behavior are the principal risks.
