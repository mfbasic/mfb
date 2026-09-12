# bug-597: `tls::listen("", …)` raises `ErrInvalidAddress` on Linux (and Windows) instead of binding every interface

Last updated: 2026-09-12
Effort: small
Severity: MEDIUM — a documented bind form fails at runtime on two of three backends, and the page's own example uses it
Class: Correctness / docs-vs-behaviour

Status: Open
Regression Test: `tests/net/rt_tls_listener_local_address.rs::tls_listen_binds_every_interface_when_the_host_is_empty` (it already exists and runs on Linux; see below)

## How it was found

The bug-472 man-example gate, run over the whole corpus on box 2223 (Linux aarch64),
failed `tls::accept` example 1. Its failure was `77070001 Network host, address, or
port is invalid`, and every other TLS server example failed on its missing certificate.
The example calls `tls::listen("", 8443, "cert.pem", "key.pem")`.

## Reproduction

One-file project, a real self-signed certificate (`openssl req -x509 … -addext
extendedKeyUsage=serverAuth`), `tls::listen(<host>, 0, "cert.pem", "key.pem")`, then
print whether `tls::localAddress` reports a port:

| host | Linux 2223 (main `04c81a605`) | macOS (round-3 `d2e923967`) |
|---|---|---|
| `""` | **raised 77070001** | bound |
| `"0.0.0.0"` | bound | bound |
| `"127.0.0.1"` | bound | bound |

Linux `tcp::listen("", 0)` binds, so this is not the host environment.

`mfb man tls listen`: "An empty string or "0.0.0.0" binds all interfaces".

## Root cause

This is bug-113's defect, reintroduced in the TLS helpers. The OpenSSL
(`lower_tls_listen_openssl`) and Schannel (`lower_tls_listen` in
`gen_schannel_server.rs`) listen helpers pass a NULL node for the empty host, and also
always pass a NULL service. POSIX `getaddrinfo` requires at least one of the two to be
non-NULL: glibc and musl return `EAI_NONAME`, and Winsock documents the same
requirement. So the `AI_PASSIVE` bind-all path could never succeed. bug-113 fixed exactly
this for the shared socket helpers (`src/codegen/os/socket/shared.rs`, "Stage the C string
"0" and point service at it"), and the TLS helpers were never given it. macOS
Network.framework does not use `getaddrinfo` for the local endpoint; it binds the static
`"0.0.0.0"`, which is why the macOS column passes.

## Why the existing pin did not catch it

bug-575 added `tls_listen_binds_every_interface_when_the_host_is_empty`, gated
`#![cfg(any(target_os = "macos", target_os = "linux"))]`. It was run on macOS, and its
Linux result was never observed before bug-575 merged.

## Evidence

| run (box 2223, Linux aarch64, `cargo test --release --test rt_tls_listener_local_address -- --test-threads=1`) | result |
|---|---|
| main `04c81a605` | `tls_listen_binds_every_interface_when_the_host_is_empty` **FAILED** ("unexpected first line from the TLS server: \"\""), 1 passed / 1 failed, exit 101 |
| main + this fix | 2 passed / 0 failed, exit 0 |

Emitted-code pins: `listen_bind_all_passes_a_non_null_service`, one each in the OpenSSL
(`gen_openssl.rs` tests) and Schannel (`gen_schannel_tests.rs`) modules. Windows has
no execution proof from this host; box 2230 is owed a run.

## Fix

Both helpers now reserve a `service` slot, zeroed on the named-host path. On the
bind-all path it points at an on-frame C string `"0"`, the bug-113 shape. The real port
still overwrites `sin_port` afterwards. A named host is unchanged: service stays NULL.
