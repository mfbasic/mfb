# bug-249: a `tls::listen` + `tls::accept` program fails to build — `_mfb_str_error_tls_failed` data object not emitted

Last updated: 2026-07-15
Effort: small (<1h)
Severity: MEDIUM
Class: correctness (build failure on valid source)

Status: Open
Regression Test: tests/ (a program calling tls::listen and tls::accept builds)

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
