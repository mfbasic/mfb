# bug-748: Windows `tls::accept` fails a ClientHello that arrives in more than one piece

Last updated: 2026-09-26
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Closed
Regression Test: `codegen::builtins::tls::gen_schannel::schannel_tests::accept_retries_an_incomplete_first_call_without_a_context`
(codegen pin; Schannel cannot run from the Mac). The runtime proof is on box 2230,
below. `tests/net/rt_tls_accept_handshake_budget.rs:tls_accept_bounds_the_whole_handshake`
also sends the ClientHello in pieces as its positive control, but that test is
macOS/Linux only.

STATUS: FIXED (46945f084)

Shipped as designed: the `FIRSTF` store moved after `branch_if_incomplete`. It was
filed as bug-743 and renumbered to 748 (743 was already taken), so the fix
commits' subjects say bug-743. No deviation.

Found while fixing bug-739 on box 2230. On Windows (Schannel), `tls::accept`
raises `ErrTlsFailed` (77070008) for a well-formed TLS client whose ClientHello
reaches the server in more than one `recv`. Two pieces of 3 bytes, then the
rest, is enough. TCP gives no guarantee that a ClientHello arrives in one
segment: a slow link, a small MSS, a middlebox, or a large ClientHello (many
cipher suites, post-quantum key shares) all split it. macOS and Linux complete
the same handshake.

**The single correct behavior a fix produces:** `tls::accept` on Windows
completes the handshake however the ClientHello bytes are split across
`recv`s, as on macOS and Linux.

References:

- `mfb man tls accept`: accept "runs the server side of the TLS handshake". The
  page names no condition on how the client's bytes arrive.
- Microsoft, `AcceptSecurityContext` (Schannel): pass NULL for `phContext` on the
  first call, and on `SEC_E_INCOMPLETE_MESSAGE` read more data and call again
  with the same arguments.
- bug-739, during which this was found.

## Failing Reproduction

`bugs/repro/bug-739-tls-accept-trickle-server.mfb` built with
`mfb build --target windows-x86_64`, and `bugs/repro/bug-739-tls-accept-trickle-client.py`
(`trickle STEP_MS PIECES CHUNK`), both on box 2230, with an 8000 ms bound:

| Client | Unfixed | Fixed |
| --- | --- | --- |
| 2 pieces of 3 bytes, 50 ms apart, then the rest | `FAILED 77070008` (client: connection aborted) ✗ | `OK` ✓ |
| 20 pieces of 10 bytes, 50 ms apart | `FAILED 77070008` ✗ | `OK` ✓ |
| whole ClientHello at once (`late 3000 4500`) | `OK` ✓ | `OK` ✓ |

- Observed: `ErrTlsFailed` for a legitimate client.
- Expected: the handshake completes.

## Root Cause

`src/codegen/builtins/tls/gen_schannel_server.rs:lower_tls_accept`: after each
`AcceptSecurityContext` call the loop stored `FIRSTF = 1` ("the context now
exists, pass `&ctxt` from now on") and only THEN tested for
`SEC_E_INCOMPLETE_MESSAGE` (`branch_if_incomplete`) to go back and `recv` more.
An incomplete FIRST call creates no context, so the retry passed a pointer to
an uninitialised `CtxtHandle` instead of NULL, and Schannel rejected it. Any
split after the first ASC call was already fine, because by then a context
really did exist.

## Goal

- An incomplete first `AcceptSecurityContext` call is retried with a NULL
  context. The store of `FIRSTF` moves after the incomplete branch.

### Non-goals (must NOT change)

- The retry itself: the bytes stay in `RECV` (`RECV_LEN` kept) and the next
  `recv` appends to them.
- The client side (`gen_schannel.rs` connect). Its first
  `InitializeSecurityContext` takes no input, so it always creates the context.
  Unaffected.

## Blast Radius

- `gen_schannel_server.rs:lower_tls_accept`: fixed here.
- `gen_schannel.rs` connect handshake: unaffected (see Non-goals).
- `gen_schannel_read_close.rs` / `DecryptMessage`: a different state machine;
  an incomplete record is handled by its own read loop. Not affected by
  `FIRSTF`.

## Fix Design

Move `FIRSTF = 1` after `branch_if_incomplete`. One instruction reorder.

## Phases

### Phase 1 — failing test

- [x] Codegen pin `accept_retries_an_incomplete_first_call_without_a_context`:
      RED against the unfixed `gen_schannel_server.rs`, GREEN after.

Commit: 42f6ef85b

### Phase 2 — the fix

- [x] Reorder in `lower_tls_accept`.

Commit: 46945f084

### Phase 3 — validation

- [x] Box 2230: the matrix above.
- [x] Windows `.ncodesum` for `tests/byte-identity/tls` regenerated (with
      bug-739's change to the same function), and the full `cargo test` run.

Commit: 39099e32b (goldens). Full `cargo test --release` is green and the
artifact gate has 0 diffs on the tip with main merged in.

## Summary

A one-line ordering bug in the Schannel accept loop that broke any server
whose client's ClientHello arrived split. The fix is a reorder; the risk is
nil beyond the Windows golden hash.
