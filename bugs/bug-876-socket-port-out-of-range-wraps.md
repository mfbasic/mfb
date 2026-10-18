# bug-876: `tcp::connect`/`tcp::listen`/`udp::bind` accept any Integer port and silently use its low 16 bits

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/tcp/port-out-of-range-rt (new) — name suggested

## Cause (coordinator, bug-staging-24)

- Rule: a port is an unsigned 16-bit number; outside 0..65535 it is `ErrAddressInvalid` (77070001, "Network host, address, or port is invalid", `mfb spec stdlib transports`) before any socket call.
- Where: `src/codegen/os/socket/shared.rs:lower_net_endpoint_helper` stores bits 8..15 and 0..7 of the 64-bit port into `sin_port` with no bounds check, the one endpoint path `tcp::connect`/`tcp::listen`/`udp::bind` share.
- Fix: the shared endpoint helper refuses a port outside 0..65535 with `ErrAddressInvalid` before `getaddrinfo`; the three members inherit it (declared-errors tables updated if they lack it).

The host/port socket builders write only the low two bytes of the `port` Integer into `sin_port`
and never range-check it. `tcp::connect("127.0.0.1", p + 65536)` connects to port `p`;
`tcp::listen("127.0.0.1", 70000)` binds port 4464; `udp::bind("127.0.0.1", -1)` binds 65535;
`tcp::connect(host, p - 65536)` (a negative port) also connects to `p`. A typo or an unvalidated
user-supplied port reaches a *different* service with no error — silent, and in a client that
derives the port from input, a way to steer a connection.

Correct behavior: a port outside 0..65535 fails with `ErrAddressInvalid` (`77070001`, "Network
host, address, or port is invalid") before any socket call (or `ErrInvalidArgument`, whichever the
transports spec pins).

References:

- src/docs/spec/diagnostics/02_error-codes.md registry: `7-707-0001 ErrAddressInvalid` "Network host, address, or port is invalid."
- `mfb man tcp connect`: port "Written directly into the resolved address."
- Transports is not B05's assigned topic; found while provoking runtime error codes for diagnostics/02_error-codes. Check B06 for a duplicate.

## Failing Reproduction

```
IMPORT io
IMPORT tcp
IMPORT net
FUNC main() AS Nothing
  RES l = tcp::listen("127.0.0.1", 0)
  LET a = tcp::localAddress(l)
  io::print("listening port " & toString(a.port))
  RES c = tcp::connect("127.0.0.1", a.port + 65536)
  io::print("connected via port+65536 to " & toString(tcp::remoteAddress(c).port))
END FUNC
```

- Observed: `listening port 55210` / `connected via port+65536 to 55210` (exit 0).
- Expected: `tcp::connect` fails `ErrAddressInvalid` (77070001).

Also measured: `tcp::listen("127.0.0.1", 70000)` → bound port `4464`; `udp::bind("127.0.0.1", -1)` →
bound port `65535`; `tcp::connect("127.0.0.1", 70000)` with nothing on 4464 → `ErrNetworkFailed`
(77070003), not `ErrAddressInvalid`.

## Root Cause

`src/codegen/os/socket/shared.rs:lower_net_endpoint_helper`: after `getaddrinfo`, "Overwrite sin_port
at ai_addr + 2/3 with the requested port" — `load_u64(v10, PORT_OFFSET)`, `shift_right_immediate(v11, v10, 8)`,
`store_u8(v11, +2)`, `store_u8(v10, +3)` — stores bits 8..15 and 0..7 of the 64-bit Integer with no
bounds check, so every higher bit (and the sign) is discarded.

## Goal

- Every host/port entry point (tcp connect/listen, udp bind/send-to, tls connect/listen, net::lookup if it takes a port) rejects port < 0 or > 65535 with one documented code.

### Non-goals (must NOT change)

- Port 0 for listen/bind ("any free port") keeps working.

## Blast Radius

- Every caller of `lower_net_endpoint_helper` (tcp/udp/tls endpoints) — shared, fixed together.
- `net::Address` records constructed by user code with an out-of-range `port` field passed to the `Address` overloads — not checked here; same hazard likely.

## Addendum (batch B06)

- macOS `tls::listen` / `tls::connect` with port 70000 / 65536 / -1 do not wrap but raise `ErrNetworkFailed` (77070003) instead of `ErrAddressInvalid` — the transports disagree on the out-of-range-port error; the fix should give all of tcp/udp/tls one validated contract (measured by the review agent, `/tmp/hardening/B06/tlsport`).
