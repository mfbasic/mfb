# bug-249: a `tls::listen` + `tls::accept` program fails to build — `_mfb_str_error_tls_failed` data object not emitted

Last updated: 2026-07-16
Effort: small (<1h)
Severity: MEDIUM
Class: correctness (build failure on valid source)

Status: FIXED (2026-07-16).

The error-message data-object gate
(`src/target/shared/code/data_objects.rs`) keyed the `ErrTlsFailed` string set
on the client-side calls only — `tls.connect`/`read`/`readText`/`write`/
`writeText`/`close`. The three server-side calls (`tls.listen`, `tls.accept`,
`tls.closeListener`) were missing, so a program built from them emitted the
helper bodies (which carry a `_mfb_str_error_tls_failed` relocation) without
ever emitting the string. Fix: add the three to the trigger list.

Why `tls.close` did not already cover the repro: a listen+accept program that
lets scope-drop close its resources issues **no** NIR `tls.close` call at all —
drops are codegen-emitted, and `module_uses_call` only sees NIR calls (plus
resource-union drops). The pre-existing `tests/syntax/tls/accept_valid` fixture
closes explicitly, which is why it never caught this.

The emitted message set already was the union of every tls helper's error
strings, so no message row needed adding — only the trigger list. Audited: the
union across all 7 helpers on both backends (openssl.rs + macos.rs) is
TLS_FAILED / ADDRESS_INVALID / ADDRESS_NOT_FOUND / NETWORK_FAILED /
CONNECTION_CLOSED / RESOURCE_CLOSED / INVALID_ARGUMENT / ENCODING / TIMEOUT,
all present, plus ALLOCATION which is emitted unconditionally.

Verified: repro builds for macos-aarch64, linux-x86_64, linux-aarch64,
linux-riscv64; 3 new regression tests pass and all 3 fail with the fix
reverted; full acceptance suite shows zero golden churn (949 tests, same 2
pre-existing unrelated `.audit` mismatches before and after).

No previously-building program changes output: any program reaching the
newly-added calls without a client-side call failed to build before this fix.

Regression Test: tests/tls_listen_accept_build.rs

A program that calls both `tls::listen` and `tls::accept` fails to build with:

```
error: native code data relocation target '_mfb_str_error_tls_failed' is not a data object or defined symbol
```

The `tls.accept` helper's error paths reference the `ErrTlsFailed` message string
(`ERR_TLS_FAILED_SYMBOL` / `_mfb_str_error_tls_failed`), but the gate that decides
which error-message data objects to emit does not pull that string in for this
helper set, so the relocation has no target and the code plan is rejected.

Found while verifying bug-202 (the TLS accept handshake timeout). Confirmed
**pre-existing** and unrelated to that fix: the same failure reproduces with the
bug-202 change stashed, on an otherwise clean tree.

## Failing Reproduction

```mfb
IMPORT io
IMPORT tls
FUNC main AS Integer
  RES l = tls::listen("127.0.0.1", 18443, "/tmp/cert.pem", "/tmp/key.pem")
  RES s = tls::accept(l, 1500) TRAP(e)
    RETURN 0
  END TRAP
  RETURN 0
END FUNC
```

`mfb build -target=linux-x86_64 <project>` → the error above, exit 1. No binary is
produced. Expected: the program builds (its runtime behavior is then the ordinary
listen/accept path).

Note this is why bug-202 could only be validated by the tls acceptance suite and
not by an end-to-end stalled-client run — no `listen`+`accept` program can be built
until this is fixed.

## Root Cause

The error-message data-object emission gate (`src/target/shared/code/mod.rs`, the
table around the `ERR_TLS_FAILED_CODE`/`ERR_TLS_FAILED_MESSAGE`/
`ERR_TLS_FAILED_SYMBOL` row, ~:3243) is keyed on a helper/runtime-call set that
does not include the `tls.accept` (and/or `tls.listen`) helper, even though those
helpers' `push_error_message_address(… ERR_TLS_FAILED_SYMBOL …)` paths emit a data
relocation against it.

## Non-goals

- Do not change the tls error semantics or which code `ErrTlsFailed` carries.

## Blast Radius

- The error-string emission gate only. Audit the same gate for the other `tls::`
  helpers (`connect` works today, so the gate covers its set) and for any other
  helper whose error paths reference a message symbol the gate does not pull in.

## Fix Design

Include the `ErrTlsFailed` message in the emitted set whenever any `tls::` helper
that can raise it is emitted (`accept`/`listen`, not just `connect`), mirroring how
the other conditional error strings are gated. Add an acceptance fixture that
builds a `tls::listen` + `tls::accept` program so the gate stays honest.
