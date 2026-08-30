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
- ~~Resource ownership is not descriptor-generic for ordinary consuming calls; close consumption is
  selected in `src/syntaxcheck/builtins.rs:net_consumes_argument`.~~ — **false, corrected §C4**:
  that file and symbol no longer exist. Close consumption *is* descriptor-generic today, keyed off
  `RegistryResource::close_function` and resolved in `src/ir/verify/link.rs:929`
  (`consumed_resource` → `close_op_for` → `builtin_resource_close_function`). The conclusion
  survives the correction and gets stronger: because consumption is keyed to the resource's *own*
  registered close op, `tls::wrap` consuming a `tcp::Socket` is not expressible by any existing
  seam, so plan 110-D needs a genuinely new ownership mechanism, not a table row.
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
Commit: cadd99a25

### Phase 2 — Registry contract and frontend

- [x] Add `PingStatus`, `PingResult`, and both `ping` implementations under
      `src/codegen/builtins/net/`; preserve the existing Address/Url layouts.
      → `func_ping.rs` (descriptor, two overloads) and the enum/record in `mod.rs`. Address and
      Url are untouched: the only `.ir` change to existing fixtures is the two added declarations
      plus a uniform 24-line `ErrorLoc` shift (§C11).
- [x] Add argument normalization, return typing, enum/record source injection, and errors; audit
      AST, HIR, IR, link verifier, resource, and binary-representation seams for the new names.
      → padding + `net.pingAddr` routing in `builder_values.rs`; per-target
      `SUPPORTED_RUNTIME_CALLS`; libc/DLL imports in the three `plan.rs`; the error-message pool in
      `data_objects.rs`; the alias force-emit in `builder/mod.rs` (§C8). No resource or
      binary-representation seam needed: `PingStatus`/`PingResult` are *value* types, so unlike
      `net.Socket` they never reach the resource registry or the `.mfp` type table.
- [x] Tests: add valid and invalid fixtures under `tests/rt-behavior/net/`,
      `tests/rt-error/net/`, and `tests/syntax/net/`, covering both overloads and all defaults.
      → `func_net_ping_valid`, `func_net_ping_range_invalid`, `func_net_ping_invalid`, plus
      `ping` coverage added to `tests/byte-identity/net` so the backend's codegen is gated on all
      five targets, plus four unit tests in `gen_ping.rs` pinning the enum ordinals, the record
      field offsets, the documented maximum, and the overload shapes.
- [x] Added task: fix the two pre-existing compiler bugs that blocked the contract's own spelling
      (§C7) — enum variants named `Ok`/`Error`/`Err` were unmatchable, and no enum-typed value
      could be bound through an inline `TRAP`.

Acceptance: `mfb man net ping` and a fixture compile to the exact requested signatures; invalid
arity/types/ranges fail with the specified diagnostics.
**MET** — `mfb man net ping` renders both requested signatures verbatim:
`net::ping(host AS String, [timeoutMs AS Integer], [ttl AS Integer], [size AS Integer]) AS PingResult`
and the `Address` form. `func_net_ping_invalid`'s golden pins the diagnostics: arity is reported as
"expected 1 to 4" and a wrong argument type names both overloads. Range violations are runtime
(the values are not constant-folded), and `func_net_ping_range_invalid` proves all seven raise
while every boundary value is accepted.
Commit: 46e3b7203

### Phase 3 — Native ICMP execution

- [x] Implement packet construction, monotonic deadline, reply/error parsing, and OS error mapping
      in per-platform emitters, preserving caller-saved register and stack-alignment invariants.
      → `gen_ping.rs`: `lower_ping_posix` (macOS and Linux arms) and `lower_ping_windows`. Every
      value that must survive an external call lives in a stack slot, never a vreg, matching the
      rest of `net`; the Windows 8-argument call stages arguments 4–7 to the outgoing-args area
      rather than through `c_arg(7)`, which is `rbp` (§C2).
- [x] Add deterministic parser/checksum/unit tests plus loopback runtime tests for host and Address.
      → four `gen_ping.rs` unit tests (each RED-checked, §C12) and the `func_net_ping_valid`
      fixture covering both overloads, the defaults, size 0 and size 8184. The **checksum is
      genuinely covered by the macOS loopback run**, which is where the golden harness executes:
      measured with `/tmp/p110-probe/cksum-matters.c`, a deliberately corrupted checksum gets no
      reply on macOS (`correct -> 1, corrupt -> 0`). On Linux the kernel recomputes it
      (`corrupt -> 1`), so a Linux run would *not* validate it — which is exactly why the claim is
      recorded with the platform it holds on rather than stated flatly.
- [x] Add a permission-denial runtime test using an isolated test environment that actually denies
      ICMP socket creation; do not accept a mocked errno as end-to-end proof.
      → proven two independent ways, neither mocked: box 2223 denies ICMP for an ordinary user by
      shipping `ping_group_range = 1 0`, and `unshare -Un` on any Linux box maps the caller to gid
      65534 with the same effect. Both run the real program and both raise `ErrNetworkFailed`
      (7-707-0003), exit 255 (§C10).

Acceptance: loopback returns `Ok` with the responder, positive elapsed/TTL/size values; a silent
address returns `Timeout`; denied permission raises Error; malformed/unrelated replies are ignored.
**MET** on all five target families (§C10). Loopback: `Ok`, responder `127.0.0.1`, `rttMs > 0.0`,
`ttl > 0`, `size = 56`. Silent address (192.0.2.1): `Timeout`, address still the destination, all
three measurements zeroed. Denied permission: Error, not a status. Unrelated replies are ignored by
construction — the receive loop re-polls against the deadline rather than accepting the first
packet, which is what makes the call correct on macOS, where **every** ICMP socket receives
**every** reply on the host (§C1).
Commit: 46e3b7203

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

### C5 — Measured socket constants and struct layouts (Phase 2)

`net::ping` bakes numeric socket options, clock ids, and struct offsets into emitted machine code,
where a wrong value fails silently on a platform the build host cannot execute. Every number below
was printed from the platform's own headers by `scripts/icmp-constants-probe.c`, run on macOS
AArch64 (this host) and on 2227 (Alpine x86_64 musl), 2228 (Debian x86_64 glibc), 2229 (Alpine
riscv64 musl) and 2223 (Kali AArch64 glibc). **Every value that the ping backend uses differs
between macOS and Linux**, so none of them could have been shared:

| Constant | macOS | Linux (all four arch/libc combinations) |
|---|---:|---:|
| `AF_INET` | 2 | 2 |
| `SOCK_DGRAM` | 2 | 2 |
| `IPPROTO_IP` | 0 | 0 |
| `IPPROTO_ICMP` | 1 | 1 |
| **`IP_TTL`** | **4** | **2** |
| **`IP_RECVTTL`** | **24** | **12** |
| **`CLOCK_MONOTONIC`** | **6** | **1** |

Linux `recvmsg` structure offsets (identical on x86_64/aarch64/riscv64 and on glibc/musl — only the
*declared width* of `msg_iovlen`, `msg_controllen` and `cmsg_len` varies, which is invisible when
the emitter stores and reads 8-byte values at these 8-aligned little-endian offsets):

| Field | Offset | Note |
|---|---:|---|
| `msghdr.msg_name` | 0 | |
| `msghdr.msg_namelen` | 8 | u32 |
| `msghdr.msg_iov` | 16 | |
| `msghdr.msg_iovlen` | 24 | glibc `size_t`, musl `int`+pad; a u64 store of `1` is correct for both |
| `msghdr.msg_control` | 32 | |
| `msghdr.msg_controllen` | 40 | a u64 store is safe **on Linux only** — `msg_flags` sits at 48 |
| `msghdr.msg_flags` | 48 | |
| `sizeof(struct msghdr)` | 56 | macOS is 48 |
| `cmsghdr.cmsg_len` | 0 | glibc 8 bytes, musl 4+pad |
| `cmsghdr.cmsg_level` | 8 | u32 |
| `cmsghdr.cmsg_type` | 12 | u32 |
| `CMSG_DATA` offset | 16 | |
| `CMSG_LEN(sizeof(int))` | 20 | `CMSG_SPACE` = 24 |

Two traps this measurement caught that a from-memory transcription would have got wrong:

1. **`msg_controllen` cannot be written with a u64 store on macOS** — `msg_flags` is at offset 44
   there (48 on Linux), so an 8-byte store at 40 clobbers it. This is harmless in the shipped
   design only because macOS uses `recvfrom` and never builds a `msghdr` at all; had the "one
   uniform POSIX `recvmsg` path" of §3 been implemented, it would have been a live bug.
2. **The control message arrives as `IP_TTL`, not `IP_RECVTTL`.** `IP_RECVTTL` (Linux 12) is the
   value passed to `setsockopt` to *enable* the option; the resulting `cmsg_type` is `IP_TTL`
   (Linux 2). The first version of the Phase 1 probe compared `cmsg_type == IP_RECVTTL`, found no
   match, and reported no TTL at all despite the kernel having supplied one — the reply-TTL source
   looked unavailable on Linux until the comparison was fixed.

These are carried into codegen as `CodegenPlatform` methods (`ipproto_ip`, `ip_ttl`,
`ip_recvttl`, `cmsg_ip_ttl_type`, `clock_monotonic`, `so_rcvbuf`), matching the existing
`sol_socket`/`so_rcvtimeo` idiom rather than as literals at the emission sites.

### C6 — The documented maximum payload did not round-trip, and why

Phase 1 froze `size`'s maximum at **8184** as the smaller of the two platforms' `sendto` limits
(§C3). Executing the finished backend showed that number was measured on the wrong side of the
exchange: on macOS a 8184-byte ping **sent successfully and then reported `Timeout`**, as though
the host were down.

Binary-searching the real limit through the shipped implementation gave **8132** payload bytes
(IP total 8160). The cause is the *receive* path, not the send path: macOS's default raw receive
space is `net.inet.raw.recvspace` = 8192, and BSD socket-buffer accounting charges per-datagram
overhead on top of the bytes, so a reply at the documented maximum is dropped by the socket layer
before `recvfrom` ever sees it. The Phase 1 probe missed this because it measured the largest
payload `sendto` would *accept* and separately round-tripped only small payloads — it never
round-tripped a large one.

The response was **not** to trim the published maximum to 8132. That number is an artifact of a
tunable default, and lowering the contract to match a default would have made the documentation
true by weakening the feature. `/tmp/p110-probe/rcvbuf.c` measured the alternative:

```
default SO_RCVBUF reported = 8192
largest round-tripping payload, default buffer = 8132
request SO_RCVBUF=32768   -> reported 32768   largest payload = 8184
request SO_RCVBUF=65536   -> reported 65536   largest payload = 8184
```

So the backend now sets `SO_RCVBUF` to 65536 on the ICMP socket before sending, and the frozen
8184 maximum is honest on both platforms — verified end to end: `8184` returns `Ok size=8184` and
`8185` raises `ErrInvalidArgument`. §C3 is unchanged; the implementation was corrected to meet it.

The general lesson, worth more than the constant: **a send-side limit is not a round-trip limit.**
Any future probe of a request/response protocol must measure the largest message that comes back,
not the largest the kernel accepts.

### C7 — Two pre-existing compiler bugs this letter had to fix first

Both were found by writing the first real `net::ping` program, both reproduce with **no ping and
no plan-110 surface at all**, and both are fixed here rather than deferred (a deferred bug is one
the next session inherits).

1. **A user enum with an `Ok`, `Error`, or `Err` variant could not be matched.**
   `CASE Outcome.Ok` was rejected with `TYPE_RESULT_NOT_MATCHABLE` — the guard that stops
   `CASE Ok` from being read as the internal `Result` member consulted only the UNION variant
   table, which is `None` for an enum, so every enum variant sharing one of those three names was
   caught by it. Repro (pre-existing, no ping):

   ```
   ENUM Outcome
     Ok
     Failed
   END ENUM
   ' MATCH o / CASE Outcome.Ok -> error[2-203-0071 TYPE_RESULT_NOT_MATCHABLE]
   ```

   Fixed in `src/ir/verify/matching.rs:check_match_patterns` by exempting the scrutinee's own
   enum variants alongside its union variants. `net::PingStatus.Ok` is the contract-mandated
   spelling, so this was not optional.

2. **No enum-typed value could be bound through an inline `TRAP`.**
   `native code cannot materialize default value for type 'Outcome'` — the default-value
   materializer in `src/codegen/memory/value/builder_value_semantics.rs` had arms for every scalar,
   collection, record, union and resource shape but none for an enum, so the error-path temp could
   not be built. This also blocked any *record* carrying an enum field, because the record arm
   defaults each field in turn — which is exactly how `PingResult` hit it. Fixed by adding the
   enum arm: an enum value is its ordinal at run time, so its default is ordinal 0, the first
   declared variant, precisely as `Integer`'s default is 0.

   Repro (pre-existing, no ping): any `LET o = <fallible returning an ENUM> TRAP(e) … END TRAP`.

### C8 — A code-form alias needs an explicit force-emit entry

`net.pingAddr` is an `os_alias` synthesized by `builder_values`, so the NIR only ever names
`net.ping`. The plan's runtime-symbol scan reads the NIR, so the alias body was never emitted and
the call site relocated against an undefined `_mfb_rt_net_net_pingAddr`. Registering the alias in
the registry and in the per-target supported-call lists is **not** sufficient — there is a separate
hand-maintained list in `src/codegen/engine/builder/mod.rs` that force-emits each synthesized
alias whenever its base symbol is present (`connectTcpAddr` off `connectTcp`, `pollList` off
`poll`, process's `spawnEnv` off `spawn`, …). Added the `pingAddr`-off-`ping` entry there. Any
future `os_alias` needs the same, and the symptom is a link-time undefined symbol, not a
compile error.

### C9 — Windows execution findings (box 2230)

Running the contract program on Windows 11 x86_64 confirmed the `iphlpapi` backend and turned up
two divergences worth recording:

- **`IcmpSendEcho` rejects a `0` timeout.** With `timeoutMs = 0` the call failed outright and the
  status mapped to an Error, instead of the convention's "one immediate attempt, report `Timeout`
  unless a reply is already waiting". The per-attempt timeout is now clamped up to 1 ms — the
  smallest expressible wait — which is precisely the accommodation
  `lower_net_set_timeout_helper` already makes for Winsock's `SO_RCVTIMEO`, where `0` means
  *infinite* rather than *don't wait* (plan-73-C). After the clamp, `timeoutMs = 0` reports
  `Timeout` on Windows exactly as it does on POSIX.
- **The API's own `RoundTripTime` is unusable for `rttMs`.** It is a whole-millisecond `ULONG`, so
  a loopback ping would report `0.0` and break the contract's measured-rtt guarantee. The backend
  times the exchange with `QueryPerformanceCounter`/`QueryPerformanceFrequency` instead, mirroring
  the POSIX `clock_gettime(CLOCK_MONOTONIC)` path; `rttMs > 0.0` holds on Windows loopback.
- Windows loopback reports `ttl = 128` against Unix's `64`. That is the platform's default hop
  limit, not a defect, and is why the fixtures assert `ttl > 0` rather than a literal.

`IcmpSendEcho`'s 8 arguments are staged as §C2 requires — 0..3 in the register bank, 4..7 written
directly to the outgoing-args area — rather than through `emit_external_int_call`, whose `c_arg(7)`
would be `rbp`.

### C10 — Cross-target execution ledger for Phase 3

Every row is the same contract program, cross-built on the macOS host and executed natively.

| Target | Box | Result |
|---|---|---|
| macos-aarch64 | this host | all 13 cases pass; `TtlExceeded` observed naming the real first hop (192.168.1.1) |
| linux-x86_64 musl | 2227 | all pass; reply TTL via cmsg (64 loopback, 255 off-link) |
| linux-x86_64 glibc | 2228 | all pass |
| linux-riscv64 musl | 2229 | all pass |
| linux-aarch64 glibc | 2223 | **ICMP denied by the OS** → `ErrNetworkFailed` (7-707-0003) raised, exit 255 — the contract's required behavior |
| windows-x86_64 | 2230 | all pass after the `0`-timeout clamp; `ttl = 128` (platform default) |

The portable denial environment was verified independently on 2227: under `unshare -Un`,
`ping_group_range` reads `65534 65534` and the same program raises `ErrNetworkFailed`. So the
permission-denied path is proven on real hardware *and* reproducibly anywhere, not mocked.

`TtlExceeded` is reproducible only from the macOS host: both Linux boxes and the Windows box sit
behind a NAT that re-originates the echo, so `ttl = 1` returns a normal reply there (the Phase 1
probe found the same, §C1). `Unreachable` was not reproducible from any available network.

### C11 — Golden drift: measured, classified, and fully explained

Adding two declarations to `net`'s injected source drifted **54 `.ir` goldens and one `.ast`** —
every fixture that imports `net`, directly or through `http`/`tls`/the resource-union tests. This
is the documented consequence of editing the injected package source, not a surprise.

The drift was classified rather than assumed. Normalizing every `"line": N` field away and diffing
each golden against `HEAD` shows exactly two kinds of change and nothing else:

1. The `PingResult` type and `PingStatus` enum declarations appearing.
2. `ErrorLoc` line numbers inside `builtins/net.mfb` shifting by a uniform **+24** — the number of
   lines the two declarations add (130→154, 133→157, 136→160, 139→163, 190→214, 204→228, 222→246,
   226→250).

53 of the 54 files carry a byte-for-byte identical added/removed signature; the 54th is
`tests/byte-identity/net`, which additionally gained the six `ping` calls I added to it. No
semantic change appears in any of them, so regenerating is correct.

### C12 — The unit tests were RED-checked

A test that cannot fail is not a test. `ping_status_literals_match_the_declared_variant_order` was
verified to fail by changing `STATUS_TIMEOUT` from `"1"` to `"9"`
(`assertion left == right failed, left: "9", right: "1"`), then reverted and re-confirmed green.
The pin matters because the emitters write status ordinals as literals while the *declaration
order* in `net::register` is what actually assigns them — reordering the enum while editing its
documentation would silently change what every ping reports, with nothing failing.

## Summary

This letter isolates the only wholly new protocol and freezes shared value semantics before TCP,
UDP, TLS, and consumers move. Packet parsing and real permission behavior are the principal risks.
