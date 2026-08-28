# plan-110-C: UDP package extraction

Last updated: 2026-08-27
Effort: large (3h–1d)
Depends on: plan-110-B

Create the requested `udp` package with `udp::Socket` and one binary `udp::Datagram` shape, moving
datagram I/O out of net while retaining legacy shims until the final cutover.

References: plan-110-B; `.ai/compiler.md`; `.ai/resources-packages.md`;
`src/codegen/builtins/net/{func_bind_udp.rs,func_receive_from.rs,func_send_to.rs,gen_io.rs,gen_poll.rs}`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-110-B complete | `ls planning/plan-110-B-* 2>/dev/null` returns no matches | NOT MET |
| Full suite green | `rustup run 1.96.0 cargo test` | UNVERIFIED |

## 1. Goal

- Every requested udp signature executes against real datagram sockets, uses `net::Address`, and
  returns `udp::Datagram` with sender address plus bytes.

### Non-goals

- Do not carry forward `receiveTextFrom` or `DatagramText`; String is accepted only by send and
  receive always preserves arbitrary bytes.
- Do not make UDP connected, reliable, ordered, or stream-like.
- Do not delete legacy net APIs before plan 110-E.

## 2. Current State

`net` currently owns `UdpSocket`, `Datagram`, `DatagramText`, `bindUdp`, two send functions, two
receive functions, close/address/timeout/poll overloads (`src/codegen/builtins/net/mod.rs:register`).
The current backend already preserves datagram boundaries and sender Address; bug fixtures under
`tests/rt-behavior/net/bug160_sendto_capacity_gt_count` protect the byte-count/capacity distinction.

Verified by reading the descriptors and `gen_io.rs`: byte and text send share OS machinery, while
text receive performs UTF-8 validation. The requested surface intentionally drops that decode path.

## 3. Design Overview

Add `src/codegen/builtins/udp/`, qualified `udp.Socket`, and record
`Datagram { from AS net::Address, bytes AS List OF Byte }`. Share or move only UDP native lowerers;
String send marshals UTF-8 bytes directly. Register `udp.close` as consuming and all other socket
arguments as borrowed. Poll list returns a borrowed member, preserving resource ownership.

Expected generated changes are confined to udp/new metadata and migrated fixtures; unchanged TCP,
TLS, HTTP, and pure net fixtures are semantic controls.

## Phases

### Phase 1 — Types and ownership

- [ ] Register the udp package, `udp.Socket`, `udp.Datagram`, close/drop routing, resource tag, and
      `net.Address` cross-package type references through registry, verifier, linker, and codegen.
- [ ] Add syntax/unit tests for record field order, qualified identities, close consumption,
      non-consuming send/receive/poll, and illegal resource copies.

Acceptance: a minimal udp program binds, obtains its local net Address, and drops cleanly.
Commit: —

### Phase 2 — Datagram I/O

- [ ] Implement `bind`, `localAddress`, `close`, both `send` overloads, `receive`, and read/write
      timeout setters using udp-owned descriptors and native symbols.
- [ ] Preserve payload count independently of list capacity and preserve source endpoint/IPv4/IPv6
      formatting where supported by the existing contract.
- [ ] Tests: valid/invalid fixtures for every overload plus binary bytes (including invalid UTF-8),
      String UTF-8, zero/maximum payload, truncation at maxBytes, sender address, timeouts, and the
      bug160 regression.

Acceptance: two loopback sockets exchange exact binary and String datagrams and report the sender;
timeouts and closed handles error as documented.
Commit: —

### Phase 3 — Readiness

- [ ] Implement scalar and list `udp::poll`, including omitted/immediate/positive/negative timeout,
      EINTR/deadline recomputation, empty-list behavior, and borrowed-resource return.
- [ ] Tests: exercise two sockets so the returned list element—not merely Boolean readiness—is
      observable.

Acceptance: poll selects the socket with a queued datagram without consuming it, and the next
receive returns that datagram intact.
Commit: —

## Validation Plan

Run full `cargo test`, new UDP runtime/error/syntax fixtures, acceptance and cross-target artifact
and runtime proofs. Update descriptor man content and embedded stdlib specs. Regenerate only
expected drift and use one-fixture disassembly for surprises. Run both required rustfmt commands.

## Open Decisions

- Zero-length datagrams — recommend supporting them as successful `bytes=[]`; EOF has no UDP
  meaning and must not be borrowed from TCP semantics.

## Corrections

To be filled during execution.

## Summary

The key contract is preservation of datagram boundaries and raw bytes. Removing the text receive
shape simplifies the public API but must not weaken binary behavior or sender reporting.
