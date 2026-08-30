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
| The existing Rust suite is green | `rustup run 1.96.0 cargo test` | MET — measured 2026-08-29 on `worktree-P-110` with `rustup run 1.96.0 cargo test --no-fail-fast`: 65 test binaries, every one `test result: ok`, `0 failed` throughout, process `EXIT=0`. (`--no-fail-fast` per the project's "cargo test fail-fast skips rt_ tests" rule.) |
| No unfinished plan already owns the networking surface | `rg -n 'net::ping|tcp::|udp::|tls::wrap' planning/plan-*.md` | MET — measured 2026-08-29: 10 hits, all inside `planning/plan-110-{A,B,C,D}-*.md`. No other live plan names any of these symbols. |

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

- [x] Record the ICMP facility, privilege behavior, reply-TTL source, and maximum payload for
      macOS AArch64, Linux x86_64 glibc/musl, Linux AArch64, Linux riscv64, and Windows x86_64 in
      this plan's Corrections section, citing SDK headers/man pages and a minimal runtime probe.
      → Corrections §C1. Probe checked in as `scripts/icmp-capability-probe.c`; run on macOS
      AArch64 (this host), 2228 Debian x86_64 glibc, 2227 Alpine x86_64 musl, 2229 Alpine riscv64
      musl, and 2223 Kali AArch64 (denied). Windows route verified by confirming the `Icmp*`
      exports in `C:\Windows\System32\IPHLPAPI.DLL` on 2230.
- [x] Resolve the two Open Decisions below before adding descriptors; update this plan with the
      chosen constants and exact errors. → Corrections §C3 freezes variants/ordinals, field order,
      the `rttMs` type change, defaults, validation ranges, and the Windows `IP_STATUS` map.
- [x] Added task: record the design consequences the probe forced (three backends not two, no raw
      fallback, the x86 8-argument staging hazard). → Corrections §C2.
- [x] Added task: verify the plan family's own citations resolve. → Corrections §C4 (the
      `src/syntaxcheck/builtins.rs` seam cited by A/B/C/D no longer exists).

Acceptance: a checked-in contract table names a real implementation route for every supported
target; permission-denied is reproducible or its OS error mapping is unit-tested from the native
constant.
**MET** — §C1 names a route for all six target rows. Permission denial is *reproducible on real
hardware* (box 2223, `EACCES`) and additionally reproducible anywhere via `unshare -Un`, so the
stronger half of the acceptance ("reproducible") holds and the unit-test fallback is not needed.
Commit: 4a30df2b0

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

Both are **RESOLVED** in Corrections §C3 below; the text here records what was recommended at
authoring time.

- Ping defaults — recommend `ttl=64`, `size=56`; record these as public contract before coding.
  **RESOLVED as recommended** (§C3).
- `Address.port` for ICMP — recommend ignore input and return responder port `0`, because ICMP has
  no transport port; rejecting nonzero would make lookup-produced addresses awkward.
  **RESOLVED as recommended** (§C3).

## Corrections

### C1 — Phase 1 platform capability matrix (measured, not assumed)

Every row below was produced by compiling and running a C probe on the named machine on
2026-08-29, not read off a man page. Probe sources are reproduced in
`scripts/icmp-capability-probe.c` (checked in by Phase 1 so the matrix is re-derivable).

| Target | Box | ICMP facility | Privilege | Reply buffer shape | Reply TTL source | Echo id | Max payload |
|---|---|---|---|---|---|---|---|
| macOS AArch64 | this host (15.7.7, 24G720) | `socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP)` | **unprivileged, always** | **includes the 20-byte IPv4 header** (`byte0 = 0x45`) | IP header byte 8 (`IP_RECVTTL` cmsg also works, and agrees — see below) | **preserved** (sent `0xABCD`, received `0xABCD`) | **8184** (`net.inet.raw.maxdgram` = 8192 minus the 8-byte ICMP header); 8185 → `EMSGSIZE` (40) |
| Linux x86_64 glibc | 2230-adjacent box 2228 (Debian 12) | same | unprivileged when gid ∈ `ping_group_range`; box reads `0 2147483647` | **bare ICMP message, no IP header** | `IP_RECVTTL` setsockopt + `recvmsg` cmsg | **rewritten by the kernel** (sent `0xABCD`, received `0x0001`…) | **65507**; 65508 → `EMSGSIZE` (90) |
| Linux x86_64 musl | 2227 (Alpine) | same | `ping_group_range` = `999 59999`, gid 1000 → permitted | bare ICMP | same cmsg | rewritten (`0x432f`…) | 65507 |
| Linux AArch64 glibc | 2223 (Kali) | same | **DENIED**: `ping_group_range` = `1 0` (empty), gid 1001 → `EACCES` (13); `SOCK_RAW` → `EPERM` (1) | — | — | — | — |
| Linux riscv64 musl | 2229 (Alpine) | same | `999 59999`, gid 1000 → permitted | bare ICMP | same cmsg | rewritten (`0x62c6`…) | 65507 |
| Windows x86_64 | 2230 (Win 11, 10.0.26100.9168) | `IcmpCreateFile` / `IcmpSendEcho` / `IcmpCloseHandle` in `C:\Windows\System32\IPHLPAPI.DLL` | unprivileged (Winsock `SOCK_RAW` ICMP would need Administrator; iphlpapi does not) | `ICMP_ECHO_REPLY` struct, not a wire packet | `ICMP_ECHO_REPLY.Options.Ttl` (offset 24) | handled by the OS | bounded by `RequestSize` (`WORD`) |

Export presence on 2230 verified by scanning the DLL image for the export names:
`IcmpCreateFile`, `IcmpCloseHandle`, `IcmpSendEcho`, `IcmpSendEcho2`, `IcmpParseReplies`,
`Icmp6CreateFile` — all **FOUND**.

**Behavioural facts the matrix does not fit, all measured:**

- **macOS ICMP sockets are promiscuous.** Two `SOCK_DGRAM`/`IPPROTO_ICMP` sockets were opened and
  an echo was sent from socket *a*; **socket *b* also received the reply** ("LEAK: socket b got 84
  bytes type=0 id=5a5a seq=0063"). Linux demultiplexes per socket by the kernel-assigned id and
  does not leak. Consequence for the design: **on macOS the reply match must check the echo id and
  the sequence number and keep reading until the deadline**, because an unrelated ping in another
  process will otherwise be mistaken for ours. On Linux the id cannot be checked (the kernel
  rewrote it), so the match is type + sequence, which is sound there precisely because the kernel
  already demultiplexed.
- **ICMP Time Exceeded is delivered to the datagram socket** (macOS, `ttl=1` toward `8.8.8.8`):
  `icmp_type=11 code=0` from the first-hop router, and the embedded original datagram carries
  `icmp_type=8 id=abcd seq=0007` — so an error reply *can* be matched back to our echo through the
  quoted original header. Both Linux boxes are behind a NAT that re-originates the echo, so
  `ttl=1` still returned a normal echo reply there; TTL-exceeded matching is therefore verified on
  macOS and unit-tested on the parser for the other targets.
- **Destination Unreachable was not reproducible** from any of the four probed boxes (10.255.255.1,
  192.168.254.254, 240.0.0.1 all timed out silently rather than eliciting type 3). The type-3 →
  `Unreachable` mapping is therefore covered by a deterministic parser unit test, not by a live
  network round trip.
- **A silent address times out cleanly** (192.0.2.1, TEST-NET-1) on every permitted box:
  `poll` expires at the deadline with no packet, measured 1501 ms / 1503 ms / 1505 ms for a
  1500 ms request.
- **Permission denial is reproducible on real hardware**, no namespace tricks required: box 2223
  (Kali AArch64) has `ping_group_range = 1 0` for an ordinary user and returns `EACCES`. A second,
  fully portable denial environment exists on any Linux box: `unshare -Un` (user namespace **only**
  — *not* `-Urn`) maps the caller to gid 65534, `ping_group_range` reads `65534 65534`, and both
  `SOCK_DGRAM` (`EACCES` 13) and `SOCK_RAW` (`EPERM` 1) ICMP fail. Phase 3's denial test uses this,
  so it does not depend on box 2223 staying misconfigured.

### C2 — Design decisions forced by C1

- **`SOCK_DGRAM`/`IPPROTO_ICMP` only on POSIX; no `SOCK_RAW` fallback.** Measured: raw ICMP failed
  with `EPERM` for the ordinary user on *every* Linux box and is unnecessary on macOS, where the
  datagram socket already works unprivileged. Adding a raw fallback would also weaken matching —
  a raw socket receives every ICMP packet on the host, so it would need cross-process
  disambiguation that the datagram socket gets from the kernel for free. Where the datagram socket
  is denied, the requested contract wants an **Error**, which is exactly what C1's denial
  environments produce.
- **Three ping backends, not two.** `PlatformFamily` already distinguishes `Linux` / `MacOS` /
  `Windows` (`src/codegen/engine/types/types.rs:231`), and C1 shows macOS and Linux differ in all
  three of buffer shape, TTL source, and id handling. The existing `net` emitters branch only on
  `family() == Windows`; the ping emitter must branch three ways. This is a genuine correction to
  §3's "POSIX/Windows backends" phrasing — a single POSIX arm cannot be written.
- **macOS supports `IP_RECVTTL` too, but the backend still reads the IP header there.** The
  consolidated probe shows `setsockopt(IP_RECVTTL)` returning 0 on macOS and the cmsg TTL agreeing
  with the in-buffer IP header (64 on loopback, 114 from `8.8.8.8`). A single uniform cmsg path is
  nevertheless *not* simpler, because `struct msghdr` and `struct cmsghdr` have different layouts
  on the two systems — macOS `cmsghdr` is 12 bytes (`socklen_t cmsg_len`) against Linux's 16
  (`size_t cmsg_len`), and `msghdr` is 48 bytes against 56 — so a hand-emitted `recvmsg` needs
  per-family offsets either way. macOS therefore uses the strictly simpler `recvfrom` + IP-header
  parse (no `msghdr` at all) and Linux uses `recvmsg` + cmsg because it has no other source.
- **Do not trust the IPv4 `total_length` field on macOS.** The probe read bytes `45 00 40 00 …`
  for an 84-byte packet: BSD hands `ip_len` up in **host byte order with the header length already
  subtracted** (`0x0040` little-endian = 64 = 84 − 20). The parser uses the `recvfrom` return
  length, never `ip_len`.
- **An 8-argument external call cannot go through `emit_external_int_call` on x86.** Windows
  `IcmpSendEcho` takes 8 integer arguments. `emit_external_int_call` stages argument *n* in
  `abi::c_arg(n)` before spilling, and the x86 call bank is
  `["rcx","rdx","r8","r9","rdi","rsi","rax","rbp"]` (`src/arch/x86_64/select.rs:92`) — so `c_arg(7)`
  is **`rbp`, the frame pointer**, and staging through it corrupts the frame before the call
  (`c_arg(6)` = `rax` is likewise the C return register). The Windows ping emitter therefore stages
  arguments 0–3 in `c_arg(0..=3)` and writes arguments 4–7 to the outgoing-args area itself with
  `abi::outgoing_stack_arg_store(<vreg>, k)`, then calls `platform.emit_external_call` directly.
  This constraint is not documented anywhere else in the tree and applies to any future >6-argument
  external call.

### C3 — Frozen public contract (resolves both Open Decisions)

- `PingStatus` variants in declaration order — `Ok` = 0, `Timeout` = 1, `Unreachable` = 2,
  `TtlExceeded` = 3. An enum value is its ordinal `Integer` at runtime
  (`process::didSignal` precedent, `src/codegen/builtins/process/func_did_signal.rs`).
- `PingResult` field order — `status`, `address`, `rttMs`, `ttl`, `size`, exactly as requested.
- **`rttMs` is `Float`, not `Integer`.** Phase 3's acceptance requires a loopback ping to report a
  **positive** elapsed value, and a loopback round trip is 52–93 µs (measured) — which truncates to
  `0` in whole milliseconds on every platform, making the criterion unsatisfiable as an `Integer`.
  Typing it `Float` (milliseconds, fractional) satisfies the criterion **as written** rather than
  weakening it to "non-negative". `Float` record props are already idiomatic
  (`audio`'s `frequencyHz`/`gainOverall`).
- Defaults `ttl = 64`, `size = 56` — as recommended.
- Validation before any system call: `ttl` must be `1..=255`, `size` must be `0..=8184`
  (the **minimum** of the measured per-platform maxima, so one documented limit holds everywhere),
  `timeoutMs` negative → `ErrInvalidArgument`. `size = 0` is valid and produces a bare 8-byte ICMP
  message (measured working on macOS and all three Linux boxes).
- `address.port` is ignored on input and the returned responder `Address` carries port `0` — as
  recommended; ICMP has no transport port.
- `Ok` carries measured `rttMs`/`ttl`/`size`; `Timeout`, `Unreachable`, and `TtlExceeded` zero
  `rttMs`, `ttl`, and `size`. Name-resolution and system failures raise Errors; an echo deadline is
  `PingStatus::Timeout`; ICMP-facility creation denied by access control is an Error.
- **Windows unbounded timeout.** `IcmpSendEcho` takes a `DWORD` millisecond timeout and has no
  infinite value, so an omitted `timeoutMs` is implemented as a loop that re-issues the echo with a
  bounded per-attempt timeout until a non-`IP_REQ_TIMED_OUT` outcome arrives. Re-sending is what a
  real ping does and is the honest reading of "unbounded"; documented on the member.
- Windows `IP_STATUS` mapping: `0` (`IP_SUCCESS`) → `Ok`; `11010` (`IP_REQ_TIMED_OUT`) → `Timeout`;
  `11002`/`11003`/`11004`/`11005`/`11012` → `Unreachable`; `11013`/`11014` → `TtlExceeded`; every
  other value → Error.

### C4 — Dangling citations found in the plan family

- Plans A, B, C and D cite `src/syntaxcheck/builtins.rs:net_consumes_argument` and
  `src/syntaxcheck/builtins.rs:BUILTIN_ARG_MODES`. **Neither the file nor the symbols exist**
  (`rg -l 'net_consumes_argument|BUILTIN_ARG_MODES' src` matches only a *doc comment* in
  `src/codegen/builtins/net/func_close.rs`, which itself calls it "the former source checker's").
  Close-consumption is now entirely registry-driven: `RegistryResource::close_function` →
  `crate::codegen::resource::builtin_resource_close_function` → `close_op_for` →
  `consumed_resource` (`src/ir/verify/link.rs:929`). Corrected in B/C/D's own Corrections when each
  letter runs; recorded here because A discovered it.
- Consequence for plan 110-D: because consumption is keyed off *the resource's own registered close
  op*, `tls::wrap` consuming a `tcp::Socket` is **not** expressible by any existing seam — D's
  "extend builtin argument ownership metadata" is a genuinely new mechanism, not a table row.

## Summary

This letter isolates the only wholly new protocol and freezes shared value semantics before TCP,
UDP, TLS, and consumers move. Packet parsing and real permission behavior are the principal risks.
