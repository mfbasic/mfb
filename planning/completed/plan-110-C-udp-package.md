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
| plan-110-B complete | `ls planning/plan-110-B-* 2>/dev/null` returns no matches | MET — measured 2026-08-29: no matches; archived at `planning/completed/plan-110-B-tcp-package.md` (commit abbd627da). |
| Full suite green | `rustup run 1.96.0 cargo test` | MET — measured 2026-08-29 at plan-110-B's tip with `--no-fail-fast`: 65 binaries, all `test result: ok`, exit 0. Acceptance (1295 tests) and `artifact-gate all` (1754 goldens, 0 diffs) green in the same state. |

## 1. Goal

- Every requested udp signature executes against real datagram sockets, uses `net::Address`, and
  returns `udp::Datagram` with sender address plus bytes.

### Non-goals

- Do not carry forward `receiveTextFrom` or `DatagramText`; String is accepted only by send and
  receive always preserves arbitrary bytes.
- Do not make UDP connected, reliable, ordered, or stream-like.
- ~~Do not delete legacy net APIs before plan 110-E.~~ — **impossible, corrected §C1**: two
  packages cannot both declare a record named `Datagram`, and `udp` pulls net's injected source in
  regardless of what the program imports. This letter therefore removes net's datagram surface and
  migrates its fixtures, which is plan-110-E's UDP slice pulled forward to where it is forced.
  Net's *stream* surface is untouched and still goes in plan-110-E.

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

- [x] Register the udp package, `udp.Socket`, `udp.Datagram`, close/drop routing, resource tag, and
      `net.Address` cross-package type references through registry, verifier, linker, and codegen.
      Registered in `builtins/mod.rs`, `registry/mod.rs`, `is_builtin_import`,
      `ARGUMENT_CHECKED_PACKAGES` (per plan-110-B §C6 — omission is silent), a new
      `RuntimeHelper::Udp` family, the three per-target call/import lists, the error-message pool,
      and `resolver`'s builtin type list. The runtime tag is reused
      (`RESOURCE_TAG_UDP_SOCKET`): the tag is a self-describing marker, not a dispatch key.
- [x] Add syntax/unit tests for record field order, qualified identities, close consumption,
      non-consuming send/receive/poll, and illegal resource copies.
      Three unit tests in `udp/mod.rs`: qualified identity + close routing (and that
      `net.UdpSocket` is *gone*, per §C1), `Datagram`'s field order and its use of net's shared
      `Address`, and that no text-receive shape survives while `send`'s String overload does.
      Cross-package identity is pinned by `func_udp_send_invalid`.

Acceptance: a minimal udp program binds, obtains its local net Address, and drops cleanly.
**MET** — `func_udp_bind_valid` binds port 0, reads back a real assigned port, sets both timeouts,
closes explicitly, and exits 0.
Commit: 2dd8f30fd

### Phase 2 — Datagram I/O

- [x] Implement `bind`, `localAddress`, `close`, both `send` overloads, `receive`, and read/write
      timeout setters using udp-owned descriptors and native symbols. The String overload lowers
      through the `udp.sendText` code form, selected in `builder_values` off argument **2** (the
      payload sits after the socket and address, unlike `tcp::write`'s argument 1).
- [x] Preserve payload count independently of list capacity and preserve source endpoint/IPv4/IPv6
      formatting where supported by the existing contract. The bug-160 regression fixture moved
      across as `bug160_send_capacity_gt_count` and still builds its payload with `append` so
      capacity > count; sender-address reporting is asserted in `func_udp_send_valid` and
      `func_udp_receive_valid`.
- [x] Tests: valid/invalid fixtures for every overload plus binary bytes (including invalid UTF-8),
      String UTF-8, zero/maximum payload, truncation at maxBytes, sender address, timeouts, and the
      bug160 regression. Byte `255` in the payload proves no sign extension; a zero-length datagram
      is asserted to arrive as length 0 rather than as an end-of-stream; an oversized datagram is
      asserted to RAISE rather than truncate.

Acceptance: two loopback sockets exchange exact binary and String datagrams and report the sender;
timeouts and closed handles error as documented.
**MET** on macOS AArch64 and Alpine x86_64 musl, byte-for-byte identical output on both: a String
payload round-trips (`B text=ping`), a byte payload preserves `255` unchanged (`C bytes=4
last=255`), the sender's port is reported non-zero, and an oversized datagram raises.
Commit: 2dd8f30fd

### Phase 3 — Readiness

- [x] Implement scalar and list `udp::poll`, including omitted/immediate/positive/negative timeout,
      EINTR/deadline recomputation, empty-list behavior, and borrowed-resource return. Both lower
      through the same emitters `net::poll`/`tcp::poll` use, so the timeout convention, EINTR
      retry, and empty-list rejection are the already-proven ones rather than a reimplementation.
- [x] Tests: exercise two sockets so the returned list element—not merely Boolean readiness—is
      observable. `/tmp/udptest`'s case I sends to exactly one of two pooled sockets and then reads
      *through the returned socket*, so a wrong element would surface as wrong bytes rather than
      passing on a Boolean.
- [x] Added task: absorb plan-110-E's UDP-removal slice, forced by the `Datagram` name collision
      (§C1) — net's datagram surface removed and its fixtures migrated to `udp` rather than
      deleted, plus new `tests/byte-identity/{tcp,udp}` fixtures so both packages' codegen is gated
      on all five targets.

Acceptance: poll selects the socket with a queued datagram without consuming it, and the next
receive returns that datagram intact.
**MET** — `I listpoll=pick`: the datagram was sent to exactly one of two pooled sockets, `poll`
returned that socket, and reading *through the returned socket* produced the sent bytes. Poll does
not consume: the same datagram is still there for the receive (`G ready=TRUE` then
`H afterPoll=up`).
Commit: 2dd8f30fd

## Validation Plan

Run full `cargo test`, new UDP runtime/error/syntax fixtures, acceptance and cross-target artifact
and runtime proofs. Update descriptor man content and embedded stdlib specs. Regenerate only
expected drift and use one-fixture disassembly for surprises. Run both required rustfmt commands.

## Open Decisions

- Zero-length datagrams — recommend supporting them as successful `bytes=[]`; EOF has no UDP
  meaning and must not be borrowed from TCP semantics. **RESOLVED as recommended**, and asserted:
  `E emptyLen=0` sends and receives a real empty datagram. Documented on both `send` and `receive`
  so the TCP reading ("empty means the peer closed") cannot be carried over by habit.

## Corrections

### C1 — The "do not delete legacy net APIs before plan 110-E" non-goal is impossible here

**Two built-in packages cannot both declare a record named `Datagram`.** The injected builtin
sources share one top-level namespace, so `udp`'s `Datagram` and `net`'s existing `Datagram`
collide at declaration:

```
/tmp/udptest/builtins/net.mfb:32 error[2-201-0010 SYMBOL_DUPLICATE_TOP_LEVEL]:
    Top-level symbol `Datagram` was already declared in builtins/udp.mfb:9.
```

This is not an artifact of a test importing both. `udp` declares `IMPORT net` for the shared
`Address` record, so net's source is injected into *any* program that imports `udp` — the error
above is from a program whose only import is `udp`. There is no ordering, and no
import-hygiene fix, that lets the two coexist.

So exactly one of these had to give:

1. Ship `udp` with a differently-named record and rename it in plan-110-E. Rejected: it fails this
   letter's own acceptance ("returns `udp::Datagram`") and puts a knowingly-wrong public API into
   the tree, which is precisely what an interim state should not do.
2. Have `udp::receive` return *net's* `Datagram` and move the record's ownership in plan-110-E.
   Rejected for the same reason — `udp::Datagram` would not exist.
3. **Delete net's datagram surface in this letter** — the removal plan-110-E was going to perform
   anyway, pulled forward to the point where it is forced.

Option 3 is taken. This letter therefore absorbs plan-110-E's UDP slice: net loses `UdpSocket`,
`Datagram`, `DatagramText`, and the members `bindUdp` / `sendTo` / `sendTextTo` / `receiveFrom` /
`receiveTextFrom`, together with the `UdpSocket` overloads on `close` / `localAddress` / `poll` /
`setReadTimeout` / `setWriteTimeout`. Its fixtures migrate to `udp` in the same change rather than
being deleted, so no behavioural assertion is lost.

The corresponding tasks are struck from plan-110-E Phase 3 with a pointer here, so the work is
neither done twice nor silently dropped. The letter's Non-goal is corrected in place above.

Note this does **not** generalise to `tcp`: `tcp` introduces no value record (its endpoints are
net's `Address`), which is why plan-110-B could leave net's stream surface standing and this
letter cannot.

## Summary

The key contract is preservation of datagram boundaries and raw bytes. Removing the text receive
shape simplifies the public API but must not weaken binary behavior or sender reporting.
