# plan-47-G: the Winsock networking surface

Last updated: 2026-07-20
Effort: small (<1h) for G1; medium (1h–2h) for G2
Depends on: **G1 depends on nothing** (inert chokepoint refactor, lands before 47-A).
**G2 depends on 47-S** (the lifted socket seam) and G1.
Feature-wide precondition: master §Prerequisites.
Produces: a `net_symbol` chokepoint (G1); the Winsock implementations and `net.*` in
`runtime_calls` (G2). Consumed by 47-H.

Implements `net::*` over Winsock2. **This is the sub-plan the master got most wrong.**
It is described in the 2026-07-14 feature map as one of the surfaces that "add *new*
methods to the Windows `CodegenPlatform`" — but there is **no `emit_socket`, no
`emit_connect`, no `emit_close_socket` on the trait at all.** The net surface is
*constants only*. Every socket call is a hardcoded POSIX symbol literal in shared
lowering.

The single behavioral outcome: a program that resolves a hostname, connects, sends and
receives over TCP, and a program that binds/listens/accepts, both produce byte-identical
stdout on Windows and linux-x86_64 — including the non-blocking and timeout paths.

References (read first):

- `src/target/shared/code/net/{mod,io,poll}.rs` — the shared lowering holding all 37
  symbol literals (§2.1).
- `planning/plan-47-F-threads.md` §Phase 1 — **the technique this sub-plan clones**, at
  38% the scale.
- `planning/plan-47-S-raise-the-posix-seam.md` §3.1 — the socket seam split (keep 6
  portable constants, lift 4 non-portable ones). G2 is its consumer.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| *(G1 only)* Byte-identity goldens for all four targets | `find tests -path '*/golden/*' -name '*.ncode*' \| while read f; do b="${f##*/}"; b="${b%.*}"; echo "${b##*.}"; done \| sort -u` | **NOT MET — `linux-riscv64` has 0** |
| *(G2)* plan-47-S has landed | `rg -n 'fn emit_classify_socket_error' src/` | **NOT MET** |
| *(G2)* plan-47-C has landed | `ls src/target/win_x86_64/code.rs` | **NOT MET** |
| *(G2)* The Win11 box answers, with outbound network | `ssh -p 2230 test@127.0.0.1 true` | **UNVERIFIED — run it** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before continuing and again before deciding to stop. If you stop, report all
> four statuses.

## 1. Goal

- **G1:** all 37 socket symbol literals in shared lowering route through one
  `net_symbol` chokepoint. Zero behavior change; four targets byte-identical.
- **G2:** the Winsock arms — `closesocket`, `ioctlsocket(FIONBIO)`, `WSAPoll`,
  `WSAGetLastError` — plus `WSAStartup`/`WSACleanup`, which have **no POSIX analog at
  all** (§3.2).
- `net.*` advertised in `runtime_calls` only after G2.
- Runtime proof: a TCP client and a TCP server round-trip, plus the non-blocking connect
  and the receive-timeout paths.

### Non-goals (explicit constraints)

- **No TLS.** That is 47-H, which depends on this.
- **No IPv6-only or dual-stack changes.** Whatever `getaddrinfo` returns today is what
  Windows returns; behavior parity, not new capability.
- **G1 adds no Windows behavior.** A Windows arm in G1 destroys its byte-identity proof.
- **Do not lift the 6 portable socket constants.** 47-S §3.1 keeps `SOL_SOCKET`,
  `SO_REUSEADDR`, `SO_RCVTIMEO`, `SO_SNDTIMEO`, `SO_ERROR` and `ADDRINFO_ADDR_OFFSET` as
  constants because Winsock defines them compatibly. Lifting them is churn.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| POSIX socket symbol literals in shared lowering | **37** | `rg -n '"(socket\|connect\|bind\|listen\|accept\|recv\|send\|close\|fcntl\|poll\|getaddrinfo\|setsockopt\|getsockopt\|freeaddrinfo\|recvfrom\|sendto)"' src/target/shared/code/net/ \| wc -l` |
| Distinct symbols | **15** | same, `--no-filename -o \| sort -u` |
| Socket constants on `CodegenPlatform` | **10** | master §2.1 |
| — portable to Winsock (47-S keeps) | 6 | 47-S §3.1 |
| — not portable (47-S lifts) | 4 | `o_nonblock`, `eagain`, `einprogress`, `emsgsize` |
| `emit_socket` / `emit_connect` methods on the trait | **0** | `rg -n 'fn emit_(socket\|connect\|accept\|listen\|bind)' src/target/shared/code/types.rs` → no matches |

Literal counts by symbol: `freeaddrinfo` 8, `fcntl` 6, `getaddrinfo` 4, `poll` 3,
`close` 3, `socket` 2, `setsockopt` 2, `bind` 2, and one each of `sendto`, `recvfrom`,
`recv`, `listen`, `getsockopt`, `connect`, `accept`.

**Scale comparison:** F rewrites ~85 pthread literals, G rewrites 37, E rewrites 6. Same
technique; the master classed G with D (a 17-method surface) rather than with F.

### 2.2 Where POSIX and Winsock actually diverge

| POSIX | Winsock | Kind of difference |
|---|---|---|
| `close(fd)` | `closesocket(s)` | **different function** — `close` on a socket handle is wrong |
| `fcntl(fd, F_SETFL, O_NONBLOCK)` | `ioctlsocket(s, FIONBIO, &1)` | **different call**, not a different flag — 6 sites |
| `poll(fds, n, ms)` | `WSAPoll(fds, n, ms)` | same shape, different name (Vista+) |
| `errno == EAGAIN` | `WSAGetLastError() == WSAEWOULDBLOCK` | **different error channel entirely** |
| *(nothing)* | `WSAStartup` / `WSACleanup` | **no POSIX analog** — §3.2 |
| `socket`, `bind`, `listen`, `accept`, `connect`, `send`, `recv`, `sendto`, `recvfrom`, `getaddrinfo`, `freeaddrinfo`, `setsockopt`, `getsockopt` | same names | portable — these 13 need only the import, not a rewrite |

So of 15 distinct symbols, **13 port by name** and 2 do not (`close`→`closesocket`,
`fcntl`→`ioctlsocket`), plus `poll`→`WSAPoll`. The literal *count* overstates the
semantic work; the chokepoint (G1) is what makes that visible.

### 2.3 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| There is no `emit_socket`/`emit_connect` on the trait | **CONFIRMED** | no matches in `types.rs`; the net surface is constants only |
| 37 socket literals live in shared `net/` | **CONFIRMED** | §2.1 command |
| Winsock defines `SOL_SOCKET`/`SO_*` compatibly | **CONFIRMED** | Winsock2 header contract; hence 47-S keeps them |
| `O_NONBLOCK` has no Winsock constant | **CONFIRMED** | non-blocking is `ioctlsocket(FIONBIO)`, a call |
| Winsock errors do not use `errno` | **CONFIRMED** | `WSAGetLastError()`; hence 47-S's `emit_classify_socket_error` |
| A socket handle is not a file descriptor on Windows | **CONFIRMED** | `SOCKET` is a `UINT_PTR` handle; `close()` on it is undefined |
| `WSAStartup` must precede any socket call | **CONFIRMED** | Winsock contract — §3.2 |
| Round-trip parity on Windows | **UNVERIFIED — this is the acceptance criterion** | proven on the Win11 box |

## 3. Design Overview

**G1 — the chokepoint (inert, blocks on nothing).** One `net_symbol(intent)` function
maps an intent to a symbol name; all 37 literals route through it. Nothing else changes.
Proof: 0-diff goldens on all four targets. Lands before 47-A alongside F1 and E1.

**G2 — the Winsock arms.** Fill `net_symbol`'s Windows arm (3 renames), consume 47-S's
`emit_set_nonblocking` and `emit_classify_socket_error`, and solve §3.2.

### 3.2 `WSAStartup` has no POSIX analog — the one genuinely new problem

Winsock requires `WSAStartup(MAKEWORD(2,2), &wsadata)` **before any socket call**, once
per process. POSIX has nothing like it, so there is no existing seam and no shared code
path that would call it.

Three placements, and the choice matters:

- **(a) In the program entry (47-C's floor), unconditionally.** Simple and always
  correct, but every Windows program — including `hello.exe` — pays a ws2_32 import and
  an init call it does not need.
- **(b) In the entry, conditionally on `net.*` being advertised.** Requires the entry
  lowering to know whether any net call is reachable. The information exists
  (`runtime_symbols`, the same list `skip_entry_arena_destroy` inspects at
  `mod.rs:712`), so this is achievable.
- **(c) Lazily, on first socket call, behind a guard.** Cheapest for non-net programs but
  adds a branch and a global to every socket operation, and gets subtle under threads.

Recommended **(b)**. `WSACleanup` on the shutdown path symmetrically (`lower_shutdown`,
`entry_and_arena.rs:1868`).

**Where design uncertainty concentrates:** §3.2's placement, and nowhere else. Everything
else is a rename or a seam 47-S already built. **G2 Phase 1 is a spike on `WSAStartup`
placement** — a program that opens one socket and exits — before the 37 sites are
touched.

**Where correctness risk concentrates:** the error channel. Every non-blocking path in
shared `net/` compares against `EAGAIN`/`EINPROGRESS`. On Windows those comparisons are
against a value that is never set, so a would-block condition reads as a hard failure —
or worse, as success. 47-S's `emit_classify_socket_error` is the fix; G2's job is to make
sure **every** comparison site goes through it, with none left reading `errno`.

**Rejected alternative:** *use the Winsock POSIX-compatibility names via `#define`.*
There is no such layer for a code generator — those are C preprocessor conveniences, not
exported symbols. `closesocket` is the real export.

**Rejected alternative:** *keep `close` for sockets and let Windows sort it out.*
Rejected: `close()` on a `SOCKET` handle is undefined behavior, not a leak.

## 4. Detailed Design

`net_symbol` Windows arm — only three entries differ:

| Intent | POSIX | Windows |
|---|---|---|
| close a socket | `close` | `closesocket` |
| poll readiness | `poll` | `WSAPoll` |
| set non-blocking | `fcntl` | *(handled by 47-S's `emit_set_nonblocking` → `ioctlsocket`)* |

The other 13 symbols keep their names and need only ws2_32 imports.

## Compatibility / Format Impact

- **New:** `net.*` in the Windows `runtime_calls`; ws2_32 imports; `WSAStartup`/
  `WSACleanup` in the entry/shutdown path (§3.2).
- **Changed (shared, G1):** 37 literals route through one chokepoint. Byte-identical for
  the four existing targets.
- **Unchanged:** the `net::` language surface; every other backend's networking.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### G1 Phase 1 — the chokepoint (inert; blocks on nothing; land early)

- [ ] Add `net_symbol(intent)` and route all 37 literals in `net/mod.rs` (14),
      `net/io.rs` (16 across recv/send/accept/close/addrinfo) and `net/poll.rs` (2)
      through it.
- [ ] No Windows arm. No behavior change.

Acceptance: `scripts/artifact-gate.sh` 0 diffs on all four existing targets. A diff means
the refactor changed emission — fix it, do not rebaseline.
Commit: —

### G2 Phase 1 — spike: `WSAStartup` placement (settles the only uncertainty)

- [ ] Implement §3.2 option (b): conditional init in the entry, keyed on `net.*` being
      in `runtime_symbols`; `WSACleanup` on the shutdown path.
- [ ] Runtime: a program that creates one socket and exits, and `hello.exe` — confirm
      `hello.exe` gains **no** ws2_32 import.

Acceptance: the socket program works; `hello.exe`'s import table is unchanged from 47-C.
If conditional init proves impractical, record why and fall back to (a) explicitly rather
than drifting into (c).
Commit: —

### G2 Phase 2 — the renames and the error channel

- [ ] Fill `net_symbol`'s Windows arm: `closesocket`, `WSAPoll`.
- [ ] Route every non-blocking/would-block comparison through 47-S's
      `emit_classify_socket_error`. **Audit that none is left reading `errno`** —
      `rg -n 'eagain\(\)|einprogress\(\)|emsgsize\(\)' src/target/shared/code/net/`
      should return nothing after this phase.
- [ ] `emit_set_nonblocking` → `ioctlsocket(FIONBIO)` for all 6 former `fcntl` sites.

Acceptance: the audit grep is empty; a blocking TCP client round-trips on Windows.
Commit: —

### G2 Phase 3 — the async paths (largest blast radius last)

The non-blocking and timeout paths are where a missed error-channel site produces a hang
or a spurious failure rather than an obvious break.

- [ ] Runtime: non-blocking connect (`EINPROGRESS`-equivalent), receive with
      `SO_RCVTIMEO`, and a server accept loop.
- [ ] Advertise `net.*` in `runtime_calls`.

Acceptance: non-blocking connect, receive-timeout and accept-loop programs all produce
byte-identical stdout to linux-x86_64. A hang here is the expected failure mode for a
missed `WSAGetLastError` site — treat a timeout as a failure, not a flake.
Commit: —

## Validation Plan

- Tests: G1's proof is byte-identity. G2's is runtime round-trips on the Win11 box.
- Coverage check: G1 edits shared lowering compiled by every backend, and
  `linux-riscv64` has zero goldens (master §Prerequisites row 3), so its 0-diff is
  vacuous. Seed them before G1.
- Runtime proof: TCP client, TCP server, non-blocking connect, receive timeout — all
  byte-compared against linux-x86_64.
- Doc sync: none expected; `net::` semantics are unchanged. A Windows-specific limitation
  would be a spec change.
- Acceptance: full suite plus `scripts/artifact-gate.sh` 0 diffs.

## Open Decisions

1. **`WSAStartup` placement** (§3.2) — recommended (b), conditional on `net.*` being
   advertised, so `hello.exe` pays nothing. Settle it in G2 Phase 1 with a spike;
   the failure mode is drifting into (c) lazy-init, which gets subtle under threads.
2. **`WSAPoll`'s known behavior gap.** `WSAPoll` historically did not report failed
   connections via `POLLERR` the way POSIX `poll` does. Recommended: verify against the
   non-blocking-connect test in G2 Phase 3 specifically, and if it bites, use
   `select()` for the connect-completion case only, documenting why.
3. **Whether `SOCKET` handles need a distinct representation** from file descriptors in
   the runtime. Recommended: no — both are integer-sized handles and shared lowering
   already treats them opaquely. Revisit only if `emit_close_file` and the socket close
   path prove to collide.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The master's classification of G was wrong.** It listed G with D as a
  surface that "adds new methods to the Windows `CodegenPlatform`". There is **no
  `emit_socket`/`emit_connect` on the trait at all** — the net surface is constants only,
  and all 37 socket calls are hardcoded literals in shared lowering. G is F's shape at
  38% the scale, and is split into an inert G1 that blocks on nothing.
- 2026-07-20 — **The literal count overstates the semantic work.** Of 15 distinct
  symbols, 13 port to Winsock by name; only `close`→`closesocket`, `poll`→`WSAPoll` and
  `fcntl`→`ioctlsocket` differ. The chokepoint is what makes that visible — which is an
  argument for G1 landing early regardless of when G2 does.
- 2026-07-20 — **`WSAStartup` has no POSIX analog and therefore no existing seam.** It is
  the one genuinely new mechanism in this sub-plan and it is not mentioned anywhere in
  the 2026-07-14 master.

## Summary

The engineering risk is the error channel, not the calls. Thirteen of fifteen socket
symbols port by name; what does not port is `errno` itself. Every would-block comparison
in shared `net/` reads a value Windows never sets, and the failure mode is a hang or a
spurious success rather than an obvious break — which is why G2 Phase 2 ends with an
audit grep that must come back empty, and why the async paths are scheduled last.

The one genuinely new mechanism is `WSAStartup`, which has no POSIX analog and so no
seam to slot into. Its placement is settled by a spike before the 37 sites are touched.

What is left untouched: the `net::` language surface, TLS (47-H), the 6 portable socket
constants 47-S deliberately keeps, and every other backend's networking.
