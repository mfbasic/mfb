# bug-179 — macOS TLS closed-default record read as open → `nw_connection_cancel((void*)0x1)` SIGSEGV

Last updated: 2026-07-12
Severity: HIGH — a reachable memory-safety crash (SIGSEGV) on any closed/defaulted
macOS TLS handle on the resource drop path.
Class: Correctness / memory safety (backend layout drift).
Status: FIXED (plan-38, commit a91d3d56)

## Finding

The macOS Network.framework TLS backend (`src/target/shared/code/tls/macos.rs`)
placed its `closed` flag at record offset **0** (`REC_CLOSED = 0`) while offset 8
held the `nw_connection` pointer (`REC_CONN = 8`). The backend-independent
closed-resource default (`builder_value_semantics.rs`, `lower_default_value`)
zeroes the 80-byte record and sets **offset 8** — the canonical resource closed
flag shared by File, net, audio, and the OpenSSL/Linux TLS backend.

Consequence on macOS: a closed/defaulted `TlsSocket`/`TlsListener` record had
offset 0 (`REC_CLOSED`) = 0, so the close guard read it as **open** and did not
short-circuit; it then loaded offset 8 (`REC_CONN`) = **1** and called
`nw_connection_cancel((void*)0x1)` via `branch_link_register` → dispatch into
Network.framework on pointer `0x1` → **SIGSEGV**. The same offset-0-vs-8
divergence made every macOS TLS op read its guard from the wrong word, but only
`close` is on the reachable drop path.

## Trigger

On a macOS build, any closed or defaulted macOS TLS handle reaching `close`, e.g.
the `$trap_val` closed-default drop path:

```
RES sock = tls::connect("127.0.0.1", 1, 500, "") TRAP(e)
  RETURN e.code        ' resource handler diverges; scope-drop closes the
END TRAP               ' closed-default record → crash before the fix
```

Reproduced: with the pre-fix layout the program SIGSEGVs (exit 139); after the
fix it exits cleanly (0). Regression test:
`tests/rt-behavior/resources/closed-default-tls-drop-rt`.

## Fix

Moved the macOS TLS `closed` flag to the canonical offset 8 (swap `REC_CONN`→0,
`REC_CLOSED`→8; `REC_QUEUE`/`REC_CTX`/`REC_SIZE` unchanged). Every record access
already went through the named `REC_*` constants and the aarch64 trampolines
touch only the block context (`CTX_*`/`LCTX_*`), never the handle record, so the
swap needed no load/store edits. Added `const _: () = assert!(REC_CLOSED ==
RESOURCE_OFFSET_CLOSED)` — the compile-time guard that catches exactly this drift
(introduced by plan-38's offset-8 standardization). See plan-38 §F7.
