# bug-465: `tcp` and `tls` do not mirror each other — and `tcp::read`'s end-of-stream documentation is wrong in a way that breaks its own example

Last updated: 2026-08-30
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (documentation) + Footgun (API asymmetry) + one functional gap

Status: Open
Regression Test: `tests/rt-behavior/tcp/tcp-read-eof-raises-rt/` (new), `tests/rt-behavior/tls/tls-listener-local-address-rt/` (new)

`tcp` and `tls` expose the same 11 function names — `accept`, `close`, `connect`,
`listen`, `localAddress`, `poll`, `read`, `remoteAddress`, `setReadTimeout`,
`setWriteTimeout`, `write` — and are meant to be drop-in mirrors, so that a
protocol package can be written once against a transport shim. A pairwise audit of
all 11 found one **documentation defect that inverts a stated contract**, one
**functional gap**, three **behavioral/API asymmetries**, and stale prose left by
the `readText`/`writeText` removal.

**The headline finding is the opposite of what it looks like.** `tcp::read` is
documented to return an empty `List OF Byte` at end of stream ("the normal end of
a stream, not an error") while `tls::read` raises. **That is false.** Both raise
`ErrConnectionClosed`, with the identical message. The transports already mirror
each other at EOF; it is the `tcp` documentation that diverges from the `tcp`
code. The docs are dangerous rather than merely wrong: `tcp::read`'s own "Read
until the peer closes" example loops on `IF len(chunk) = 0 THEN reading = FALSE`,
a condition that **can never be true**, so the documented idiom is an infinite
loop terminated only by the raise it claims does not happen.

**The single correct behavior a fix produces:** `tcp`'s documentation states that
`tcp::read` raises `ErrConnectionClosed` at end of stream and its examples use a
`TRAP`-terminated drain loop; `tls::localAddress` accepts a `Listener` as
`tcp::localAddress` does; and the remaining asymmetries are either aligned or
deliberately documented as intentional with the reason stated.

References:

- `src/codegen/builtins/tcp/func_read.rs:17` — the false claim.
- `src/codegen/builtins/tcp/gen_io.rs:584-586,689-692` — the code that contradicts it.
- `src/codegen/builtins/tcp/func_poll.rs:39` — the same false claim, restated.
- `src/codegen/builtins/tcp/mod.rs` MODULE_DESC — carries it a third time.
- bug-464 (`bugs/bug-464-sockets-and-listeners-not-thread-sendable.md`) — the other tcp/tls asymmetry found in the same audit (sendability). Independent fix.
- Found during: the 2026-08-30 review of the `websockets` section of `planning/todo.md`, which recorded the *documented* (wrong) divergence as a real hazard. That note must be corrected as part of this fix — see Phase 3.

## Failing Reproduction

All probes run on macos-aarch64 with `target/release/mfb`.

### 1. `tcp::read` at a clean EOF — raises, does not return empty

```
IMPORT tcp
IMPORT net
IMPORT io

FUNC probe() AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect("127.0.0.1", bound.port)
  RES conn = tcp::accept(server)
  tcp::close(client)                          ' clean EOF on `conn`
  LET chunk AS List OF Byte = tcp::read(conn, 16)
  io::print("RETURNED list, len=" & toString(len(chunk)))
  RETURN 0
END FUNC

FUNC main AS Integer
  LET r AS Integer = probe() TRAP(e)
    io::print("RAISED: " & e.message)
    RETURN 0
  END TRAP
  RETURN 0
END FUNC
```

- Observed: `RAISED: Socket peer closed the connection or the connection is no longer usable.`
- Expected per the docs: `RETURNED list, len=0`.
- Expected after this fix: the observed behavior is correct and stays; the **documentation** changes to match it.

### 2. `tls::read` at EOF — identical (the contrast case, proving mirror-ness)

Same shape against a real peer (`tls::connect("example.com", 443)`, `GET / HTTP/1.0`, drain to close):

- Observed: `RAISED at EOF: Socket peer closed the connection or the connection is no longer usable.`

Byte-identical message to the `tcp` case. **The two transports already agree.**

### 3. `tls::localAddress` cannot take a Listener

```
RES server = tls::listen("127.0.0.1", 0, "cert.pem", "key.pem")
LET bound = tls::localAddress(server)     ' no such overload
```

- Observed: rejected — `tls::localAddress` is registered with `expected_arguments: Some("Socket")` (`src/codegen/builtins/tls/func_local_address.rs:67`), Socket-only.
- Expected: accepted, as `tcp::localAddress(listener)` is.

This is not hypothetical: **`tls::remoteAddress`'s own man example** binds `tls::listen("127.0.0.1", 0, …)` — port 0, OS-assigned — and there is no way for that program, or any client, to discover which port it got. `tcp::localAddress`'s documentation calls the port-0 read-back "the only race-free way to bind"; `tls` cannot do it.

## Root Cause

**Finding 1 (docs vs. code).** `tcp::read` lowers through
`lower_net_read_helper(…, text = false)` (`src/codegen/builtins/tcp/func_read.rs:82`
→ `gen_io.rs:478`). After the platform `recv`, the shared (not platform-gated)
sequence at `gen_io.rs:584-586` is:

```rust
abi::compare_immediate(abi::return_register(), "0"),
abi::branch_eq(&peer_closed),
abi::branch_lt(&read_fail),
```

and `peer_closed` (`gen_io.rs:689-692`) emits `ErrConnectionClosed`. A `recv`
returning 0 — the textbook clean EOF — is therefore a raise, on **every** target,
not a zero-length success. The prose at `func_read.rs:17` ("An empty list means
the peer closed its end of the connection — the normal end of a stream, not an
error") describes a behavior that the emitter has no path to produce. The likely
origin is inheritance from the pre-split `net` package prose plus the ordinary
POSIX `recv` convention; nothing gates doc prose against the emitter, so it never
surfaced. This same claim was then propagated into `func_poll.rs:39` and the `tcp`
MODULE_DESC, and into `planning/todo.md`.

**Finding 3 (the localAddress gap)** is a plain missing overload: `tcp` registers
Socket and Listener forms, `tls` registers only Socket. `tls::listen` binds
through the same path `tcp::listen` does ("The endpoint is resolved and bound
exactly as `tcp::listen` does" — `tls/func_listen.rs`), so the listener has a real
fd and the address is retrievable; nothing but the missing registration prevents it.

## Goal

- `tcp::read`'s documentation states that a clean EOF raises `ErrConnectionClosed`, and its drain example uses `TRAP`.
- The same correction lands in `tcp::poll`'s description and the `tcp` MODULE_DESC.
- `tls::localAddress(listener AS tls::Listener) AS Address` exists and returns the bound address, so port-0 binding is usable on `tls`.
- Each remaining asymmetry in the matrix below is either aligned or documented as deliberate, with the reason stated in the prose.
- A regression test pins the EOF contract for **both** transports, so a future change to either cannot silently re-diverge.

### Non-goals (must NOT change)

- **Do NOT "fix" `tcp::read` to return an empty list.** The tempting reading of this bug is that the code is wrong and should match the docs. It is the reverse: the raise is the behavior `tls` also has, `http`'s internal readers are built on it, and changing it would silently break every existing drain loop that relies on the raise. The docs move to the code, never the other way. This is the explicit wrong fix.
- No change to `ErrConnectionClosed`'s code, message, or which conditions raise it.
- No change to `tls::read`'s behavior — it is already correct and correctly documented.
- No change to `tcp::poll`/`tls::poll` readiness semantics; only the sentence describing what a subsequent read does.
- The backlog-default and close-idempotency asymmetries (matrix rows 4 and 5) are **behavioral** changes with existing callers; this bug documents them and decides them in Open Decisions, but must not silently change either.

## Blast Radius

The full pairwise audit of all 11 shared function names. Verified by rendering
both man pages for each pair and reading the registry entries — not from memory.

| # | Pair | Mirror? | Finding |
| --- | --- | --- | --- |
| 1 | `read` | **behavior YES / docs NO** | Both raise `ErrConnectionClosed` at EOF (probed). `tcp` docs claim empty-list. **Fixed by this bug.** |
| 2 | `localAddress` | **NO — functional gap** | `tcp` has Socket+Listener overloads; `tls` is Socket-only. **Fixed by this bug.** |
| 3 | `poll` | docs only | Same false empty-list claim in `tcp::poll`'s prose (`func_poll.rs:39`). Signatures/semantics otherwise mirror (scalar→Boolean, list→first-ready, `ErrTimeout` on the list form). **Fixed by this bug.** |
| 4 | `close` | **NO — deliberate?** | `tcp::close` treats an already-closed handle as an **error**; `tls::close` treats it as **success** (both documented, and each explicitly cites the other as differing). A real semantic split. **Open Decision.** |
| 5 | `listen` | **NO — default differs** | `tcp` backlog defaults to **128** (`func_listen.rs:23`); `tls` backlog defaults to **0 = host default** (`tls/func_listen.rs:19`). Same concept, different default. **Open Decision.** |
| 6 | `write` | near | Both full-write, both Bytes+String overloads. `tls` documents the empty-list no-op (`tls/func_write.rs:16`); `tcp` documents nothing for that case. `tcp` documents raising when the peer has closed; `tls` does not. Doc parity only. **Fixed by this bug (prose).** |
| 7 | `connect` | near | Both host/port and `Address` forms, both with `timeoutMs`. `tls` adds `serverName` (TLS-only, correct). `tcp` documents the per-overload positional caveat; `tls` does not, though it has the same shape. Doc parity only. |
| 8 | `accept` | YES | Same signature, same timeout convention, both borrow the listener, both return an independent socket. |
| 9 | `setReadTimeout` | YES | Same signature/semantics; `tls` additionally documents that a timed-out read resumes the same outstanding receive. |
| 10 | `setWriteTimeout` | YES | Same signature/semantics. |
| 11 | `remoteAddress` | YES | Socket-only on both, same return, same borrow. |

Additional sites carrying the finding-1 defect:

- `src/codegen/builtins/tcp/func_read.rs:17` + its "Read until the peer closes" example — **fixed by this bug**; the example is not merely stale, it is non-terminating as written.
- `src/codegen/builtins/tcp/func_poll.rs:39` — **fixed by this bug**.
- `src/codegen/builtins/tcp/mod.rs` MODULE_DESC — **fixed by this bug**.
- `planning/todo.md` websockets section — **fixed by this bug**; it currently records the false divergence as a design hazard and prescribes a "transport shim that normalizes this", which is unnecessary work founded on the wrong premise.
- Stale `readText`/`writeText` residue in `tls` prose — **fixed by this bug**: `tls::accept` says *"read and write it with tls::read, tls::read, tls::write, and tls::write"* (each name duplicated where the text form used to be); `tls::write` says *"Use `tls::write` to send a String"* (self-reference) and *"`tls::read` or `tls::read`"*. Cosmetic but visibly broken in `mfb man`.
- `udp::receive` — **unaffected**, and correctly so: a zero-length datagram is genuinely ordinary and is documented as *not* end-of-stream. UDP must not be aligned with the stream transports here.
- `http`'s internal readers (`helper_read_net.rs`, `helper_read_tls.rs`) — **unaffected**; `http::handleRequest` already documents "A read that fails is treated as end of stream", i.e. it is built against the raise, confirming the raise is the real contract.

## Fix Design

Findings 1, 3 and the prose items are independent and low-risk; the two Open
Decisions are not, and should not be bundled.

The correctness risk is concentrated in **not over-correcting**. The instinct on
reading "docs say empty list, code raises" is to change the code. Per Non-goals,
the code is right. The regression test must therefore assert the *raise* for both
transports, so that the test itself documents which side won and a later reader
cannot re-litigate it from the prose.

For finding 3, `tls::localAddress` gains a `Listener` overload registered exactly
as `tcp::localAddress`'s is; the listener record holds its fd at
`TLS_LISTENER_OFFSET_FD` (`tls/gen_shared.rs:52`), which is the same
`RESOURCE_OFFSET_HANDLE` the socket form already reads, so the lowering is the
existing `getsockname` path pointed at the listener record.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] Add `tests/rt-behavior/tcp/tcp-read-eof-raises-rt/` pinning `ErrConnectionClosed` on a clean EOF (reproduction 1). This passes immediately — it is a **characterization** test locking in the true contract before the docs are rewritten around it. Note that in the fixture comment.
- [ ] Add the `tls` counterpart to the same fixture family so both contracts are pinned side by side.
- [ ] Add `tests/rt-behavior/tls/tls-listener-local-address-rt/` — bind port 0, read back the port, connect to it. **Fails to build today** (no overload). This is the RED test.
- [ ] Confirm the matrix above against the registry entries (not just the man prose) for all 11 pairs, and record any correction here.

Acceptance: the `tls` localAddress test fails for the documented reason; both EOF tests pass and pin the current behavior; the matrix is confirmed against the registry.
Commit: —

### Phase 2 — the fixes

- [ ] Rewrite `tcp::read`'s EOF prose and replace the non-terminating drain example with a `TRAP`-terminated one (`func_read.rs`).
- [ ] Correct the same claim in `func_poll.rs:39` and the `tcp` MODULE_DESC.
- [ ] Add the `Listener` overload to `tls::localAddress`.
- [ ] Repair the `readText`/`writeText` residue in `tls::accept` and `tls::write` prose.
- [ ] Add the missing `tcp::write` empty-list and peer-closed notes for parity with `tls::write`.

Acceptance: `mfb man tcp read|poll`, `mfb man tcp`, `mfb man tls accept|write` render corrected text; the `tls` localAddress test passes; the EOF tests still pass unchanged.
Commit: —

### Phase 3 — regenerate, validate, resync the downstream note

- [ ] Regenerate any drifted `.ncodesum` (the `tls::localAddress` overload is a real codegen change) and gate with `artifact-gate.sh all`; prove the delta is only ours.
- [ ] `cargo test --release --no-fail-fast` plus `test-accept.sh` (goldens are not in the cargo suite).
- [ ] **Correct `planning/todo.md`'s websockets section**: delete the "stream-shape trap" paragraph asserting the EOF divergence, and drop the transport-shim recommendation founded on it.
- [ ] Re-run all three reproductions.

Acceptance: full suite green; gate at 0 unexplained diffs; todo.md no longer records the false divergence.
Commit: —

## Validation Plan

- Regression tests: two EOF characterization fixtures (tcp + tls, asserting the raise) and one RED-then-green `tls::localAddress(listener)` fixture.
- Runtime proof: the three reproductions above; the EOF pair must show the identical `ErrConnectionClosed` message on both transports.
- Doc sync: `tcp` read/poll/MODULE_DESC, `tls` accept/write/localAddress, `planning/todo.md`.
- Full suite: `cargo test --release --no-fail-fast`, `test-accept.sh`, `artifact-gate.sh all`.

## Open Decisions

- **`close` on an already-closed handle (matrix row 4).** `tcp` errors, `tls` succeeds. Recommended: **align on `tls`'s idempotent-success**, because closing-then-dropping is the common shape and both packages already auto-close by lexical drop; erroring makes the safe idiom hazardous. But this is a behavior change with an existing `TYPE_USE_AFTER_MOVE`/`ErrResourceClosed` contract and existing goldens — it needs its own bug, not this one. Alternative: keep both and document each as deliberate.
- **`listen` backlog default (matrix row 5).** `tcp` = 128, `tls` = 0/host-default. Recommended: **align on 128**, the explicit and predictable value, since a TLS server has no reason to want a shallower queue than a plaintext one. Also a behavior change; same caveat.
- **Whether `tcp::connect`'s per-overload positional caveat should be added to `tls::connect`.** Recommended: yes, doc-only, fold into Phase 2.

## Summary

The engineering risk here is inverted from how the bug presents. The `tcp`/`tls`
EOF behaviors **already mirror** — both raise `ErrConnectionClosed`, proven by
probe on both transports — and the defect is that `tcp`'s documentation says the
opposite in three places and ships a drain example that cannot terminate. The one
genuine functional gap is `tls::localAddress`'s missing `Listener` overload, which
makes port-0 binding unusable on TLS and breaks an example in `tls`'s own man
page. Two further asymmetries (close idempotency, backlog default) are real but
are behavior changes deferred to Open Decisions. Untouched: `ErrConnectionClosed`
semantics, `tls::read`, and `udp`'s deliberately different zero-length-datagram
contract.
