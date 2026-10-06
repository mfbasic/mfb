# bug-885: macOS `tls::listen` never validates `host` — `"localhost"` or any non-IPv4 string listens on ALL interfaces on a random port; `"::1"` binds IPv6; `"fe80::1"` hangs forever

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Security

Status: Open
Regression Test: tests/rt-behavior/tls/tls-listen-host-validation-rt (to add; macOS-relevant, asserts on every target)

On macOS, `tls::listen(host, port, cert, key)` hands `host` straight to
`nw_endpoint_create_host` with no check that it is an IPv4 literal (or the empty
bind-all spelling). When the string is not a numeric IPv4 address, Network.framework
drops the local endpoint and the listener binds **`*:<ephemeral port>`**: every
interface, and a port the caller never asked for. `tls::localAddress(listener)` echoes
the caller's host string back with the ephemeral port, so the program cannot see what
happened.

The dangerous case is ordinary code. `tls::listen("localhost", 8443, ...)` reads as
"loopback only, port 8443". It actually serves TLS on the LAN, on a random port.
`http::serverSSL(port, cert, key, "localhost")` is a pass-through, so the HTTPS server
inherits the same exposure (not run separately). `tcp::listen("localhost", ...)`
resolves correctly and binds loopback. `tcp::listen` with a garbage host raises
`ErrAddressInvalid`.

Correct behavior: `tls::listen` accepts exactly what `tcp::listen` accepts on the
same target, using the same resolution and rejection. `"localhost"` binds
127.0.0.1:`port`. A malformed host or an IPv6 literal raises `ErrAddressInvalid`
(77070001), which the man page already lists. The listener never binds a port other
than the requested one (or the ephemeral one for `port = 0`).

References:

- src/docs/spec/stdlib/17_transports.md §"The shared endpoint": "Addresses are IPv4 …
  the emitters use `AF_INET`, and an IPv6 literal is rejected with `ErrAddressInvalid`
  rather than silently mis-parsed."
- `mfb man tls listen` Errors: `77070001 ErrAddressInvalid — Network host, address, or port is invalid.`
- Related, not duplicate: bug-872 (IPv6 literal URL / `tcp::connect("::1")` wrong code),
  bug-876 (tcp/udp port wraps), bug-864 (EC key on macOS listen).
- Found by batch B06 (transports topic).

## Failing Reproduction

```
mkdir -p /tmp/hardening/B06/certs && cd /tmp/hardening/B06/certs
openssl req -x509 -newkey rsa:2048 -nodes -keyout rsa8.key -out rsa.crt -days 30 -subj "/CN=localhost" \
  -addext subjectAltName=IP:127.0.0.1,DNS:localhost -addext extendedKeyUsage=serverAuth
```

`/tmp/hardening/B06/ip6l/src/main.mfb`:

```
IMPORT io
IMPORT tls
IMPORT net
IMPORT os
IMPORT collections
FUNC main AS Integer
  LET h = collections::get(os::args(), 0)
  RES l = tls::listen(h, 19571, "/tmp/hardening/B06/certs/rsa.crt", "/tmp/hardening/B06/certs/rsa8.key") TRAP(e)
    io::print("listen ERR:" & toString(e.code))
    RETURN 1
  END TRAP
  LET la = tls::localAddress(l)
  io::print("local " & la.host & ":" & toString(la.port))
  os::sleep(1500)
  RETURN 0
END FUNC
```

```
/tmp/hardening/mfb build            # in /tmp/hardening/B06/ip6l
./build/ip6l.out localhost & sleep 0.7; lsof -nP -a -p $! -iTCP; wait
```

- Observed (requested 19571):

| host arg | program prints | `lsof` LISTEN |
| --- | --- | --- |
| `127.0.0.1` | `local 127.0.0.1:19571` | `127.0.0.1:19571` ✓ |
| `""` / `0.0.0.0` | `local 0.0.0.0:19571` | `*:19571` ✓ (documented bind-all) |
| `localhost` | `local localhost:54211` | `TCP *:54211 (LISTEN)` ✗ |
| `not-an-ip` | `local not-an-ip:54210` | `TCP *:54210 (LISTEN)` ✗ |
| `999.1.1.1` | `local 999.1.1.1:54212` | `TCP *:54212 (LISTEN)` ✗ |
| `garbage::zz` | `local garbage::zz:54209` | (not captured) ✗ |
| `::1` | `local ::1:19571` | `TCP [::1]:19571` ✗ (IPv6 bound) |
| `fe80::1` | nothing; `tls::listen` never returns (killed by a 4 s `alarm`, rc=142; one run stayed alive >100 s) | — ✗ |

  A TCP connect from the LAN address reached the `localhost` listener:
  `openssl s_client -connect 192.168.1.148:54213` printed `Connecting to 192.168.1.148`. The
  probe never accepts, so the connection then got `errno=54`, a reset after the
  connection was established.
- Expected: `localhost` → `127.0.0.1:19571`; the malformed ones, `::1` and `fe80::1` →
  `listen ERR:77070001`.

Contrast (same binary build, `/tmp/hardening/B06/tcpbad`): `tcp::listen` with
`not-an-ip`, `999.1.1.1`, `garbage::zz`, `fe80::1` → `ERR:77070001`; `localhost` → ok.

| Environment | Result |
| --- | --- |
| macOS 15 (Darwin 24.6) aarch64, /tmp/hardening/mfb | fails ✗ |
| Linux (OpenSSL backend) | not run — code resolves `host` with `getaddrinfo` (`src/codegen/builtins/tls/gen_openssl.rs`, listen path), so likely immune |
| Windows (Schannel) | not run |

## Root Cause

`src/codegen/builtins/tls/gen_macos/server.rs:lower_tls_listen_macos` builds the
local endpoint with `nw_endpoint_create_host(host, itoa(port))` directly from the
caller's C string. The only special case is the empty host, which becomes
`"0.0.0.0"`. Nothing parses or resolves the host first. Network.framework treats a
non-numeric "host" endpoint as unusable for a local bind and silently listens on the
wildcard address with an ephemeral port. It accepts an IPv6 literal as-is. A
link-local `fe80::1` with no scope id never reaches `ready`, so the ready-wait loop
waits forever. The host block is then parked as `REC_LHOST` and is what
`tls::localAddress` returns (bug-465 comment in the same function), which hides the
substitution.

## Goal

- On macOS, `tls::listen` with a non-IPv4-literal host either resolves it to an IPv4 address the way `tcp::listen` does (`localhost` → 127.0.0.1) or raises `ErrAddressInvalid`. It never binds a wildcard or a port other than the requested one.
- `tls::listen("fe80::1", …)` returns (raises) promptly.

### Non-goals (must NOT change)

- Empty host / `"0.0.0.0"` bind-all behavior (bug-597).
- `tls::localAddress` reporting for a valid IPv4 host.

## Blast Radius

- `tls::listen` macOS backend — fixed by this bug.
- `http::serverSSL` (pass-through to `tls::listen`) — same exposure, fixed by the same change.
- `tls::connect` macOS with `::1` raises `ErrTlsFailed` (77070008), measured (`/tmp/hardening/B06/ip6`). This is a separate wrong code; add it to bug-872's `tls::connect` row.
- `tcp::listen` / `udp::bind` — unaffected (validated; measured).

## Golden impact

Pure native-codegen change in `src/codegen/builtins/tls/gen_macos/server.rs`
(`lower_tls_listen_macos`): `FRAME_SIZE` grew (480 → 528) and new instructions
(`getaddrinfo`/`inet_ntop`/`freeaddrinfo` calls plus a new `addr_invalid_fail` exit) were
inserted into the non-empty-host path, before `nw_endpoint_create_host`. Expect every
fixture with a `macos-aarch64.ncodesum` golden that imports `tls` or `http` and reaches a
`tls::listen`/`http::serverSSL` call to change ONLY on that target — e.g.
`tests/byte-identity/tls/`, `tests/byte-identity/http/`, `tests/syntax/tls/listen_valid`,
and any `rt-behavior`/`rt-error` fixture under `tls`/`http` with a macOS golden. Linux
(`gen_openssl.rs`) and Windows (`gen_schannel_server.rs`) backends are untouched, so their
`.ncodesum` goldens for the same fixtures are NOT expected to move, nor is any
`.ast`/`.ir` (those are target-independent and this is a codegen-only change, no MFBASIC
source text changed).
