# bug-459: an explicit `tls::close(listener)` runs the SOCKET close body — SIGSEGV on macOS

Last updated: 2026-08-30
Effort: small (< 1h)
Class: Miscompile (wrong runtime helper selected; crash)

Status: Fixed (plan-110-F Phase 1)
Regression Test: `ir::tests::lower_coverage_tests::explicit_tls_listener_close_rewrites_to_the_listener_body`
plus its mirror `closing_a_tcp_listener_does_not_reach_the_tls_listener_body`, the corrected
`tests/syntax/tls/close_valid` `.ir` golden, and `scripts/check-tls-loopback.sh` (which is what
found it).

## Symptom

```basic
IMPORT tls
FUNC main AS Integer
  RES listener = tls::listen("127.0.0.1", 18443, "chain.pem", "key.pem")
  tls::close(listener)
  RETURN 0
END FUNC
```

Segmentation fault on macOS. No client, no accept, no I/O — binding a listener and closing it is
enough.

```
EXC_BAD_ACCESS (SIGSEGV) KERN_INVALID_ADDRESS at 0x0000000000000054
  libdispatch.dylib  dispatch_async + 192
  Network            nw_connection_async_if_needed + 92
  Network            nw_connection_cancel_inner(NWConcrete_nw_connection*, bool) + 240
  Network            nw_connection_cancel + 116
  <program>
```

`dispatch_async` faults on `ldr w9, [x19, #0x54]` with `x19 == 0`: the object handed to
`nw_connection_cancel` has a null dispatch queue, because it is not a connection at all — it is the
`nw_listener`.

## Root cause

`tls::close` is one member spanning two resources. The `Listener` overload is rewritten during IR
lowering onto the internal `tls.closeListener` body (`src/ir/lower.rs`), and that rewrite selected
on the type NAME:

```rust
.filter(|type_| type_.name() == crate::codegen::builtins::tls::TLS_LISTENER_TYPE)
```

`TLS_LISTENER_TYPE` is the **bare** name the registry descriptor is declared under (`"TlsListener"`
before plan-110-D, `"Listener"` after). `expression_type(...).name()` yields the
**package-qualified** identity (`"tls.TlsListener"` / `"tls.Listener"`), because built-in resources
have been qualified end to end since plan-97/bug-441. So the filter matched nothing, the rewrite
never fired, and `tls::close(listener)` fell through to the plain `tls.close` body — the one that
cancels and releases an `nw_connection`.

Scope drop was never affected: the `Listener` resource descriptor names `tls.closeListener` as its
`close_function` directly, with no name comparison in the path. That is why the crash needed an
*explicit* close, and why every program that just let the listener drop looked fine.

## When it broke, measured

The `.ir` golden of `tests/syntax/tls/close_valid` records it exactly:

```
$ git show b61003c20^:tests/syntax/tls/close_valid/golden/func_tls_close_valid.ir \
    | grep -o '"target": "tls\.[a-zA-Z]*"' | sort | uniq -c
   2 "target": "tls.accept"
   3 "target": "tls.close"
   1 "target": "tls.closeListener"      <-- the rewrite firing
   ...
$ git show main:tests/syntax/tls/close_valid/golden/func_tls_close_valid.ir | ...
   4 "target": "tls.close"              <-- and gone
```

`b61003c20` is **bug-441**, "package-scope the remaining builtin resources". It qualified the
identities and re-baselined this golden with the loss already baked in. A byte-identity golden
recorded the regression instead of catching it: nothing asserted what the target should BE, only
that it had not changed since the last regeneration.

Present on `main` as well as on the plan-110 branch — plan-110-D renamed the constant from
`TlsListener` to `Listener` but did not introduce the mismatch.

## Why nothing caught it for that long

`tls::listen`/`accept` have no runtime fixture at all. `tests/syntax/tls/*` are compile-only, and
the single runtime TLS fixture (`rt-behavior/tls/tls-connect-google-rt`) is a *client* against the
public internet. A TLS server had never been executed by any test. plan-110-F Phase 1 added
`scripts/check-tls-loopback.sh` — an MFBASIC server dialled by `openssl s_client` over loopback —
and it crashed on its first run.

## Fix

Compare the package-qualified identity:

```rust
.filter(|type_| type_.name() == crate::codegen::builtins::tls::TLS_LISTENER_TYPE_ID)
```

Matching the bare name is now wrong in the other direction too: since plan-110-B, `tcp` declares a
`Listener` of its own, so a bare `"Listener"` would capture `tcp::Listener` and route it into tls's
body. The mirror test pins that.
