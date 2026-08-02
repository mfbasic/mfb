# plan-73-D: tls family timeout migration

Last updated: 2026-08-01
Effort: large (3h–1d)
Depends on: plan-73-A (convention, constants, spec section). Lands after plan-73-C by letter order.

Migrate `tls::connect` and `tls::accept` to the plan-73 timeout convention
(plan-73-A §1) across all three TLS backends — OpenSSL (Linux), Network.framework
(macOS), and Schannel (Windows). This is the highest-blast-radius sub-plan: the
same value-flip must be reproduced in three independently-written codegen paths and
proven on real hardware for each.

References:

- `.ai/compiler.md` (READ FIRST), `.ai/specifications.md`, `.ai/remote_systems.md`.
- plan-73-A — the convention + canonical spec section.
- Codegen: `src/target/shared/code/tls/mod.rs`, `openssl.rs`, `macos/{client,server}.rs`,
  `schannel_server.rs`, `schannel_impl.rs`; specs `src/target/shared/runtime/tls_specs.rs`;
  descriptor `src/builtins/tls.rs`.
- Man: `src/docs/man/builtins/tls/{connect,accept}.md`.

## Prerequisites

See plan-73-A's Prerequisites table. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-73-A complete | `mfb spec language builtin-functions` shows "Timeout convention" | MET — A landed a234b2e87 |
| TLS runtime boxes reachable | per `.ai/remote_systems.md` (Linux OpenSSL box; Windows Schannel box 2230; macOS local) | PARTIAL (2026-08-01): macOS local ✅; Windows 2230 ✅ UP; x86_64/aarch64 Linux boxes (2222/2228) DOWN (conn refused), BUT box **2229 (Alpine riscv64) UP with OpenSSL 3.5.7 + libssl.so.3** — the OpenSSL backend serves all Linux arches, so it is runtime-provable there. All three backends have a reachable runtime target. |
| Cross-compile toolchain available (only box 2229 has cargo) | `ssh -p 2229 … which cargo` | MET — 2229 has `/usr/bin/cargo`; the mfb cross-compiler also runs on the macOS host (`mfb build -target linux-*`). |

If plan-73-A is not complete, this sub-plan cannot start, full stop. If a TLS box
is unreachable, that backend's runtime proof is blocked — codegen changes may land
but the phase is not *done* until its runtime proof passes (see Validation).

## 1. Goal

- `tls::connect(host, port[, timeoutMs][, serverName]) AS TlsSocket` (producing):
  omit = block until connected + handshaken or the OS/TLS layer fails; `0` = one
  immediate attempt, `ErrTimeout` if not immediately complete; `> 0` = bounded,
  `ErrTimeout` on expiry; `< 0` = `ErrInvalidArgument`. (Today `0`/omit = no bound
  = block; negatives unstated. This flips explicit `0` and rejects negatives.)
  Host resolution remains outside the deadline (unchanged).
- `tls::accept(listener[, timeoutMs]) AS TlsSocket` (producing): omit = block;
  `0` = one immediate attempt, `ErrTimeout` if none pending / handshake not
  immediately complete; `> 0` = bounded, `ErrTimeout`; `< 0` = `ErrInvalidArgument`.
  (Today omit/`0` = block indefinitely.)
- All three backends behave identically at the source level; the two man pages
  cite the canonical section; every tls fixture/example matches; `cargo test` +
  `artifact-gate` green and the runtime proof passes on each platform.

### Non-goals

- No new tls functions (`tls::poll` is deferred, not part of plan-73).
- No change to certificate verification, SNI, handshake, or `ErrTlsFailed` mapping
  beyond the timeout value/expiry rule.
- macOS's collapse of connect/bind failures into `ErrTlsFailed` (documented
  platform difference) is unchanged — only the *timeout* path is unified.

## 2. Current State

From the audit (man pages + codegen read):

- `tls::connect`: positive bounds and raises `ErrTimeout`; `0` (default when
  omitted) means **no bound** (block); negatives unstated
  (`src/docs/man/builtins/tls/connect.md:43-50`; `src/target/shared/code/tls/openssl.rs`
  connect path; `macos/client.rs` "lnMs > 0 => deadline; else FOREVER";
  `schannel_impl.rs`).
- `tls::accept`: positive bounds and raises `ErrTimeout`; `0` (default when
  omitted) blocks indefinitely (`src/docs/man/builtins/tls/accept.md:38-41`;
  `openssl.rs` accept path; `macos/server.rs` "lnMs > 0 => deadline; else FOREVER";
  `schannel_server.rs`).
- Descriptor: `src/builtins/tls.rs` pads omitted `timeoutMs` with `0`
  (`default_argument_padding`, `resolve_call`); `0` currently means block.

So today `0` = block for both, in all three backends. The convention makes `0` =
one immediate attempt (`ErrTimeout`), omit = block (via the shared sentinel), and
`< 0` = `ErrInvalidArgument`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `tls::connect` call lines (tests+examples) | 14 | `grep -rn --include='*.mfb' -F 'tls::connect' tests examples | wc -l` |
| `tls::accept` call lines | 12 | `grep -rn --include='*.mfb' -F 'tls::accept' tests examples | wc -l` |
| TLS backends to change | 3 (openssl / macos / schannel) | `ls src/target/shared/code/tls/{openssl.rs,macos,schannel_server.rs}` |

Per-site flip census (literal `0` / omit / negative) is Phase 1's first task.

### Verified properties

- Each backend implements the wait independently with its own "FOREVER vs deadline"
  branch — VERIFIED from the codegen census (`macos/client.rs`, `macos/server.rs`,
  `openssl.rs`, `schannel_server.rs` all have a `> 0 ? deadline : FOREVER` path).
  Each needs the sentinel→block, `0`→one-attempt, `<0`→invalid change; RE-READ each.
- tls does not raise `ErrReadTimeout`/`ErrWriteTimeout` (it maps transfer failures
  to `ErrTlsFailed`) — VERIFIED from the tls man error table; the net error collapse
  (plan-73-C) does not touch tls read/write.
- Windows codegen has no `.ncodesum` golden; verification is import-surface +
  objdump + box 2230 runtime (per `windows-codegen-verification` note) — RE-CONFIRM.

## 3. Design Overview

The same three-way value change, reproduced per backend, then proven per platform:

1. **Descriptor (Phase 1).** `src/builtins/tls.rs`: omit padding for `CONNECT`/
   `ACCEPT` → `TIMEOUT_UNBOUNDED_SENTINEL`; reject negatives in resolve/validation
   (one check, all backends inherit).
2. **OpenSSL backend (Phase 2).** `openssl.rs`: sentinel→block, `0`→one non-blocking
   attempt (`ErrTimeout`), `>0` bounded, mapping the existing non-blocking-connect +
   poll path. Proven on the Linux box (has a real OpenSSL runtime).
3. **macOS backend (Phase 3).** `macos/client.rs` + `macos/server.rs`: replace the
   `>0 ? deadline : FOREVER` branch with sentinel→FOREVER, `0`→immediate (zero
   `dispatch_time`), `>0` deadline. Proven locally on macOS.
4. **Schannel backend (Phase 4).** `schannel_server.rs` (+ `schannel_impl.rs`):
   same mapping via `WSAPoll`. Proven on box 2230 (no `.ncodesum` golden — use the
   import-surface + objdump + runtime method).

**Correctness risk is the whole point of scheduling this last:** three hand-written
backends must agree, and each has a distinct wait primitive (poll/`dispatch_time`/
`WSAPoll`) and distinct hardware to prove it on. A source-level acceptance fixture
(same `.mfb`, three targets) is the cross-backend equalizer.

**Rejected alternative:** unify only the descriptor and let each backend keep its
`0`=block. Rejected — the flip lives in the backend's timeout branch, not the
descriptor; a descriptor-only change would leave `0` meaning block.

## Compatibility / Format Impact

- **Behavioral, intentional:** `tls::connect(h,p,0)` and `tls::accept(l,0)` change
  from block-forever to one immediate attempt (`ErrTimeout`); omit still blocks (now
  via the sentinel); negatives now rejected.
- **Unchanged:** cert verification, SNI, handshake, `ErrTlsFailed` mapping, the
  macOS failure-collapse platform note, host-resolution-outside-deadline.
- No `.mfp`/layout change.

## Phases

> Keep checkboxes current in-commit; fill `Commit:`; unticked = NOT DONE.
> A phase is done only when BOTH its codegen change AND its platform runtime proof
> pass — a codegen edit alone does not close a backend phase.

### Phase 1 — Descriptor: omit=sentinel, reject negatives

- [x] Census `tls::connect`/`tls::accept` sites. — no in-tree fixture passes a
      literal `0`/negative; all omit or pass a positive timeout (omit is the flip).
- [x] `src/builtins/tls.rs`: omit padding → `TIMEOUT_UNBOUNDED_SENTINEL`. — DONE
      (`CONNECT`/`ACCEPT` defaults). Negative-reject is per-backend, not the
      descriptor (runtime value — see Corrections D-C1). Unit tests unchanged
      (they assert padding *length*, not value).
- [x] Tests: descriptor unit tests pass; `cargo test` green. — MET.

Acceptance: descriptor unit tests pass; `cargo test` green. — MET.
Commit: c703e6d50

### Phase 2 — OpenSSL backend + Linux runtime proof

- [x] `openssl.rs`: connect + accept — up-front negative-reject; sentinel→blocking
      connect, `0`→non-blocking connect + poll(0), `>0` bounded; handshake `SO_*TIMEO`
      floored to 1µs for `0`, skipped for the sentinel; a handshake recv that hits
      the SO_*TIMEO (errno EAGAIN) → `ErrTimeout` (not the generic `ErrTlsFailed`),
      matching macOS. — DONE.
- [x] Regenerate `byte-identity/tls` + `/http` `.ncodesum` (all targets).
- [x] Cross-compile + run on box **2229 (Alpine riscv64, OpenSSL 3.5.7)**. RUNTIME
      PROVEN: `tls::connect(h,p,-1)` → `77050002` (ErrInvalidArgument); `tls::connect(h,p,0)`
      to a plain-TCP listener → `77050008` (ErrTimeout). (Correction D-C4: a *refused*
      port gives ErrNetworkFailed; the timeout needs a listening-but-non-TLS peer,
      which the fixture uses.)

Acceptance: box 2229 shows the new semantics; `cargo test` green; goldens
regenerated. — MET. Commit: c703e6d50

### Phase 3 — macOS backend + local runtime proof

- [x] `macos/client.rs` + `macos/server.rs`: up-front negative-reject;
      sentinel→DISPATCH_TIME_FOREVER, `0`→DISPATCH_TIME_NOW (immediate), `>0`
      deadline (via `dispatch_time`). — DONE.
- [x] Regenerate macOS `.ncodesum`; run the tls fixture locally on macOS. — DONE.
      RUNTIME PROVEN locally: `tls::connect(h,p,-1)` → `77050002`; `tls::connect(h,p,0)`
      → `77050008` (`tests/rt-behavior/tls/tls-timeout-convention-rt`).

Acceptance: macOS runtime shows the new semantics; `cargo test` green. — MET.
Commit: c703e6d50

### Phase 4 — Schannel backend + Windows runtime proof (highest risk last)

- [x] `schannel_impl.rs` connect: **NEW feature** (D-C5) — `socket_connect` now does a
      non-blocking connect + `WSAPoll` (sentinel→INFINITE, `0`→immediate, `>0` bounded)
      + handshake `SO_*TIMEO` (DWORD ms); up-front negative-reject; a handshake recv
      that hits the timeout (WSAETIMEDOUT 10060) → `ErrTimeout`. `schannel_server.rs`
      accept: selector flip (sentinel→block, `0`→WSAPoll(0), `<0`→invalid) + the same
      handshake bound + timeout mapping. Added `getsockopt`/`ioctlsocket` to the tls
      Win64 import list. FIXED a shared bug: `emit_set_sock_timeouts` passed the 5th
      setsockopt arg (optlen) in a register — wrong on Win64 (bug-384), so SO_*TIMEO
      silently failed and the handshake recv blocked forever; routed through
      `emit_external_int_call` (POSIX byte-identical). — DONE.
- [x] Verify + ship to box 2230 and prove the semantics. — RUNTIME PROVEN on box
      2230 (Windows/Schannel, `chcp 437`-wrapped): `tls::connect(h,p,-1)` → `77050002`;
      `tls::connect(h,p,0)` to a plain-TCP listener → `77050008` (ErrTimeout). The
      `.ncodesum` byte-identity goldens ARE committed for windows-x86_64 (tls + http).
- [x] Rewrite `src/docs/man/builtins/tls/{connect,accept}.md` to the convention (cite
      A's section). — DONE (both cite `mfb spec language builtin-functions`; examples
      fixed).

Acceptance: box 2230 shows the new semantics; man pages cite the section;
man_citations + spec-citation green; `cargo test` full green. — MET.
Commit: c703e6d50

## Validation Plan

- Tests: one source-level acceptance fixture per behavior (`connect(...,0)`,
  `accept(...,0)`, omit-blocks, negative-invalid) run on all three targets — the
  cross-backend equalizer.
- Coverage check: the `0`/omit/negative branches are exercised per backend.
- Runtime proof: Linux box (OpenSSL), macOS local, Windows box 2230 (Schannel) each
  show `,0` → `ErrTimeout` and omit → block. A blocked box means the backend phase
  is NOT done; say so explicitly (no silent skip).
- Doc sync: two tls man pages + citations.
- Acceptance: `cargo test`, `scripts/artifact-gate.sh` diffs=0 for Linux+macOS
  `.ncodesum`; Windows via import-surface + objdump + box run.

## Open Decisions

- **Schannel `WSAPoll` zero-timeout semantics** — confirm `WSAPoll(..., 0)` returns
  immediately (one attempt) rather than blocking on Schannel; if it differs, emit an
  explicit zero-wait path. Resolve during Phase 4 against box 2230. (§3)

## Corrections

- **D-C1 (descriptor + reject live in codegen; negative-reject is per-backend).**
  Phase 1 changed the omit padding to the sentinel in `src/builtins/tls.rs`
  `default_argument_padding` (`SENTINEL = crate::target::shared::code::TIMEOUT_UNBOUNDED_SENTINEL`).
  The plan's "reject negatives in resolve/validation (one check, all backends
  inherit)" is not possible — a negative `timeoutMs` is a runtime `Integer`, not a
  compile-time type — so each backend rejects it in its own prologue (mirroring net),
  before any socket/handshake allocation so nothing leaks.

- **D-C2 (Phase-1 descriptor change is behavior-preserving for omit).** The backends'
  pre-existing `timeoutMs > 0 ? bounded : FOREVER/block` branch already routes the
  sentinel (i64::MIN, not `> 0`) to the block path, so omit still blocks after Phase 1
  alone; the per-backend phases add the explicit `0`→immediate and `<0`→invalid.

- **D-C3 (macOS backend done + runtime-proven).** `macos/client.rs` + `macos/server.rs`:
  sentinel→DISPATCH_TIME_FOREVER, `0`→DISPATCH_TIME_NOW (immediate), `>0`→deadline,
  `<0`→ErrInvalidArgument (up-front, leak-free). RUNTIME-PROVEN locally on macOS: a
  plain-TCP listener (never completes TLS) gives `tls::connect(h,p,0)` → `77050008`
  (ErrTimeout) and `tls::connect(h,p,-1)` → `77050002` (ErrInvalidArgument).

- **D-C4 (OpenSSL backend codegen done).** `openssl.rs` connect + accept: sentinel→
  blocking, `0`→non-blocking + poll(0)/WSAPoll(0) immediate, `>0`→bounded, `<0`→
  ErrInvalidArgument (up-front); handshake `SO_*TIMEO` floored to 1µs for `0`,
  skipped for the sentinel (like net Phase 4). Runtime proof pending on box 2229
  (Alpine riscv64, OpenSSL 3.5.7) — the OpenSSL backend serves all Linux arches.

- **D-C5 (SCOPE DEFECT: schannel `tls::connect` never honored `timeoutMs`).** The
  plan (Current State + §3) treats all three backends as a symmetric "value-flip".
  FALSE for the Windows/Schannel **connect**: `schannel_impl.rs::lower_tls_connect`
  spills `timeoutMs` and then discards it (`let _ = TIMEOUT;` at line ~192);
  `socket_connect` is a **plain blocking `connect()` with no WSAPoll**. So schannel
  connect has NEVER bounded on `timeoutMs` — omit/`0`/`>0` all block. Making it
  convention-compliant is a *new feature* (port net::connectTcp's non-blocking-
  connect + `WSAPoll` + a handshake `SO_*TIMEO`), not a flip, and it is gated on box
  2230 (Windows) for runtime proof. `schannel_server.rs::lower_tls_accept` DOES have
  the `>0 ? WSAPoll : block` selector, so schannel **accept** is a clean flip like
  openssl. This scope was surfaced to the feature owner rather than silently
  absorbed. **DECISION (feature owner, 2026-08-01): implement it fully now** — port
  the non-blocking-connect + `WSAPoll` + handshake `SO_*TIMEO` machinery into
  schannel connect and runtime-prove on box 2230.

## Summary

Highest engineering risk in plan-73: one value-flip triplicated across three
hand-written TLS backends, each proven on its own hardware. Scheduled last so the
convention, constants, and the net/thread/audio migrations are already settled
before the cross-backend work begins.
