# bug-885: macOS `tls::listen` never validates `host` — `"localhost"` or any non-IPv4 string listens on ALL interfaces on a random port; `"::1"` binds IPv6; `"fe80::1"` hangs forever

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Security

Status: Resolved (2026-10-05)
Regression Test: tests/net/rt_tls_listen_host_validation.rs (macOS-only; the other backends already resolve correctly)

## Resolution

Fixed in `lower_tls_listen_macos` (`gen_macos/server.rs`): a non-empty host
is now resolved with `getaddrinfo` under an AF_INET hint (mirroring
`tcp::listen`) before `nw_endpoint_create_host`, and the resolved address is
formatted back to a dotted-decimal string with `inet_ntop` for the bind
target; `REC_LHOST` (what `tls::localAddress` reports) keeps the caller's
original string, unaffected. `src/target/macos_aarch64/plan.rs` needed the
matching static import declaration (`_getaddrinfo`/`_freeaddrinfo`/
`_inet_ntop` for `tls.listen`) — without it, calling these symbols broke
import resolution for every program importing `http`/`net`, even ones that
never call `tls::listen`, since `http::serverSSL`'s body unconditionally
references it.

RED: `bug-885 RED: pin macOS tls::listen host validation` (commit
`bf202c337`) — measured against the unfixed backend: `tls::listen("localhost",
54216, ...)` bound an ephemeral port 21224 instead (wildcard-address
fallback), and `tls::listen("not-an-ip", ...)` hung past the test's 10 s
bound (the same class of hang the bug doc measured for `"fe80::1"`).
GREEN: `bug-885: validate and resolve tls::listen's host on macOS` (commit
`c4c9ed650`).

Runtime proof beyond the committed test (this Mac, `lsof`):
`tls::listen("localhost", 19876, ...)` reports `localAddress` port 19876 (the
exact requested port) and `lsof -nP -a -p <pid> -iTCP` shows
`127.0.0.1:19876 (LISTEN)`, not `*:19876`. `"::1"`, `"fe80::1"`, `"not-an-ip"`,
`"999.1.1.1"`, `"garbage::zz"` all raise `77070001` instantly (0 s) — matching
`tcp::listen`'s behavior on the same inputs byte-for-byte.

### Follow-up: the 64-byte `inet_ntop` buffer leaked, plus two missing `freeaddrinfo`s

Coordinator review caught a leak the above runtime checks didn't exercise: the
64-byte `RESOLVEDCSTR` scratch buffer was never freed, and the `alloc_fail`/
inet_ntop-fail exits skipped `freeaddrinfo` after a successful `getaddrinfo`.
Measured with `mfb build --debug -O0` on a 20-iteration
`tls::listen("127.0.0.1", 0, cert, key)` + `tls::close` loop: `main` (no
resolve path touches a numeric host) reported `alloc_calls 120, free_calls 60,
live_bytes 56960`; this fix's first cut reported `alloc_calls 140` (same
`free_calls`/`live_bytes`) — the 20 extra allocations were the leaked buffers,
one per call.

Two changes:
1. `RESOLVEDCSTR` is freed on every path once `nw_endpoint_create_host` has
   read it (success or failure), and also on the `inet_ntop`-failure exit
   (new `inet_ntop_fail` label, which also now calls `freeaddrinfo` — a
   `getaddrinfo` success was never otherwise balanced on that exit).
2. Added an `inet_pton` check before the resolve path: when `host` is
   already a numeric IPv4 literal (the common case — `"127.0.0.1"`, not
   `"localhost"`), skip `getaddrinfo`/`inet_ntop` and the allocation entirely.
   This isn't just freeing the leak — it removes the extra alloc+free pair
   altogether for the case that never needed resolving, matching `main`'s
   allocation COUNT exactly, not just its leak-freedom.
   `src/target/macos_aarch64/plan.rs` gained `_inet_pton` alongside the
   existing `_getaddrinfo`/`_freeaddrinfo`/`_inet_ntop` imports for
   `tls.listen`.

Re-measured after both changes, same 20-iteration loop: `alloc_calls 120,
free_calls 60, live_bytes 56960` — exact parity with `main`. (The pre-existing
2848 B/iteration — `live_bytes` not reaching 0 even on `main` — predates this
bug and is out of scope here; the coordinator is filing it separately.)

New regression test (first cut): `numeric_host_listen_adds_no_per_call_resolve_allocation`
in `tests/net/rt_tls_listen_host_validation.rs`, in the `--debug`-report
alloc/free-counting style of `tests/runtime/rt_closure_single_owner.rs` —
builds the same loop at N and 2N iterations and asserts `alloc_calls`/
`free_calls` scale exactly linearly (no fixed per-call overhead) and stay at
the measured 6-allocs/3-frees-per-iteration baseline. Confirmed RED against
the pre-this-fix build (70 vs the expected 60 for N=10) before confirming
GREEN against the final build.

#### Second review round: the test pinned a pre-existing, unrelated leak

Coordinator review of that first-cut test: `alloc_n == free_n * 2` and
`per_iter_free <= 3` both asserted bug-984 (filed separately) — `main` itself
already frees only 3 of 6 blocks per NUMERIC-host `tls::listen`+`close` call,
and fixing bug-984 (free_calls becomes 6/iteration) would fail those
assertions for the right fix. Worse, the `inet_pton` short-circuit means a
NUMERIC host no longer reaches `getaddrinfo`/`inet_ntop`/`RESOLVEDCSTR` at
all, so a numeric-only loop cannot exercise the code that leaked in the first
place — only a NAME host (`"localhost"`) does.

Renamed to `resolve_path_leaks_nothing_beyond_the_numeric_path` and reworked:
build the loop with both `"localhost"` and `"127.0.0.1"` at N and 2N, and
assert the per-iteration `(allocs - frees)` gap is IDENTICAL for both hosts.
Measured: `127.0.0.1` gap is 3 (bug-984, unaffected by this bug); the
assertion is that `"localhost"`'s gap is ALSO 3 — i.e. the resolve path adds
no leak beyond what bug-984 already costs a numeric host, independent of
bug-984 ever being fixed. Kept the linear-scaling asserts (hold regardless of
bug-984) and the numeric-host `alloc_calls <= 6` bound (unaffected by
bug-984, which is about frees, not allocs).

RED (commit `029fe5589`): disabled only the `RESOLVEDCSTR` free that runs
after `nw_endpoint_create_host` (not the `inet_pton` skip, not the
`inet_ntop_fail` exit's free) and reran — `localhost`'s gap went to 4 while
`127.0.0.1`'s stayed at 3, failing `4 == 3` with the leaked buffer's exact
size (one extra unfreed 64 B block per iteration, name-host only). Reverted
to the real fix: GREEN, 3/3 passed.



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
