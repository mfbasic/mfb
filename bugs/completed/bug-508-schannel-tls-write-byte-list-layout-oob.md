# bug-508: Windows Schannel `tls::write(sock, List OF Byte)` reads the payload with the String layout → OOB read, remote crash of every MFBASIC HTTPS server on Windows

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (memory safety — out-of-bounds read / remote DoS)

Status: FIXED (<HASH>). Found in audit-3, Surface 6 CRY-50; agent-demonstrated on Windows box 2230, mechanism code-verified by the lead. Sibling of bug-497 (OS-50) on the Schannel backend; fixed in the same change.

Regression Test: an execution test on box 2230 asserting `tls::write(sock, byteList)` writes exactly `len(byteList)` bytes.

## Summary

The Schannel `tls::write` lowering ignores the payload-type selector and always
reads the payload with the **String** layout (`length @ +0`, `data @ +8`). A
`List OF Byte` payload has a different layout (`count @ +8`, data ~40 bytes in), so
its block-header word at offset 0 is used as the write length (~16.4 MiB observed)
and the data pointer lands inside the header. The result is an out-of-bounds read;
against a Windows MFBASIC HTTPS server that writes a byte-list body, a remote peer
crashes the process (`c0000005`). This is the Schannel mirror of the CRITICAL
bug-497 (which mis-selects the *other* direction on the macOS/SysV backends).

## Mechanism

```rust
// src/codegen/builtins/tls/gen_schannel_io.rs:334 (lower_tls_write)
) -> Result<TlsBodyParts, String> {
    ...
    // data pointer + length: String/List OF Byte both carry [u64 len][bytes].
    abi::add_immediate(&v10, abi::c_arg(1), 8),          // data := arg+8
    abi::store_u64(&v10, abi::stack_pointer(), SRC),
    abi::load_u64(&v10, abi::c_arg(1), 0),               // length := arg+0
    abi::store_u64(&v10, abi::stack_pointer(), REMAIN),
    ...
}
let _ = text;   // <-- the payload-type selector is discarded
```

The comment's premise ("String/List OF Byte both carry `[u64 len][bytes]`") is
false: a `List OF Byte` collection block is `[…header…][count @ +8][…][data @ +40]`.
Reading `length` from offset 0 of a collection block yields a large garbage value
and the data base is inside the header — an OOB read whose length is not the
payload's.

## Reproduction

Agent-demonstrated on Windows box 2230: a byte-list server crashes with
`c0000005`; the equivalent String control sends exactly 5 bytes. Lead
code-verified the `let _ = text;` and the offset-0 length read in current source.

## Best fix

Honor the `text` selector: when the payload is a `List OF Byte`, read the length
from the collection `count` field and the data from the collection data base
(the same offsets the byte path uses elsewhere); when it is a `String`, keep the
`+0`/`+8` layout. Better, share one payload-view helper between the SysV/macOS and
Schannel backends so the two cannot diverge (cross-ref bug-497). Add a sink check
that rejects a block whose tag is not the expected kind.

## Non-goals

No MFBASIC surface change; the String path's bytes must not move; do not break the
UTF-16 marshal for the text form.

## Prior art

bug-497 (OS-50, CRITICAL) is the same String↔byte-list layout-confusion family on
the macOS/SysV `tcp/tls/udp` write path; bug-157 fixed a related `tls::write` byte
base arithmetic. This Schannel instance is distinct (different backend, opposite
payload type). Searched `lower_tls_write`, `schannel`, `writeText`, `text`.

## STATUS: FIXED (<HASH>)

Landed with bug-497 on `worktree-B-497`, merged into `main`.

`lower_tls_write` (Schannel) no longer discards `text`: it emits the payload view
through the shared `codegen::memory::marshal::push_write_payload_view`, the same
helper the tcp/udp/OpenSSL/macOS write emitters use, so the five backends cannot
diverge again. Byte form: header check (`kind`/`keyType`/`valueType`, →
`ErrInvalidArgument` at `<sym>_bad_payload`), length from the collection `count`
(+8), data at `HEADER + capacity * stride`. Text form: `length @ +0`, `data @ +8`
as before (the UTF-8 bytes are copied into the Schannel send buffer unchanged; no
UTF-16 marshal is involved in `write`). `ORIGLEN` is stored from the shared
length register in both forms.

**Verification.** Windows execution (box 2230) was NOT run in this change. The
fix is verified by emitted-code inspection on `-target windows-x86_64`
(execution-free): `tests/codegen_net_write_payload_view.rs` asserts
`_mfb_rt_tls_tls_write` reads the payload's three header bytes off `rdx`,
branches to `_mfb_rt_tls_tls_write_bad_payload`, loads its length from `+8` and
never from `+0` — RED on main (main's body reads `ldr_u64 [rdx,#0]`) — and
`_mfb_rt_tls_tls_writeText` still reads `+0`. The Schannel unit tests
`write_payload_tests` pin the same two facts on the emitter directly. The
documented regression test (an execution test on box 2230 asserting
`tls::write(sock, byteList)` writes exactly `len(byteList)` bytes) remains to be
run there; the emitted layout now matches the SysV/macOS byte path that
`rt_native_write_overload_call_argument.rs` proves by execution.

**Deviation from the non-goal "the String path's bytes must not move":** the
text form's five instructions were reordered to the shared canonical order
(length first, then source) so one helper serves every backend; the instructions
and their effect are identical. The `windows-x86_64` `tls` `.ncodesum` moves for
the byte-form change anyway.
