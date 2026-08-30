# ICMP echo (`net::ping`)

The model behind `net::ping` — what the four `PingStatus` values mean, which
outcomes are statuses rather than errors, and why the same program can succeed on
one machine and fail outright on another. The per-function signature and parameter
reference is `./mfb man net ping`; this topic specifies the behavior behind it.

`net` also owns the URL model (`./mfb spec stdlib url`) and name resolution
(`./mfb man net`); the transports moved to `tcp`/`udp`/`tls`
(`./mfb spec stdlib transports`). Ping shares only the `Address` record with them.

## Status or error

The distinction that shapes the whole API: **a `PingStatus` answers a question
about the network; an Error reports that the call itself could not be
performed.**

| Outcome | Reported as |
|---|---|
| A reply came back | `PingStatus.Ok` |
| Nothing came back before the deadline | `PingStatus.Timeout` |
| A router said the destination is unreachable | `PingStatus.Unreachable` |
| A router said the request outlived its TTL | `PingStatus.TtlExceeded` |
| The host name does not resolve | Error (`ErrAddressInvalid`) |
| An argument is out of range | Error (`ErrInvalidArgument`) |
| **The OS refuses this program the use of ICMP** | Error (`ErrNetworkFailed`) |

The last row is the one worth stating explicitly. A machine that forbids ICMP has
told you nothing about the peer, so reporting `Unreachable` would be a lie about
the network. It is an error.[[src/codegen/builtins/net/gen_ping.rs:lower_ping_posix]]

`PingStatus`'s variants are ordered `Ok`, `Timeout`, `Unreachable`,
`TtlExceeded`; a variant's ordinal is its declaration index, and the native
backends write those ordinals directly into
`PingResult.status`.[[src/codegen/builtins/net/mod.rs:PING_STATUS_TYPE]]

## The result record

`PingResult` is `{ status, address, rttMs, ttl, size }` in that order.

- **`address`** always names something useful. For `Ok` it is the responder; for
  `Unreachable` and `TtlExceeded` it is the router that reported the problem
  (which is what makes a traceroute possible); for `Timeout` it is the
  destination that was aimed at. Its `port` is always `0` — ICMP is not a
  transport protocol and has no port, and any port supplied on an input `Address`
  is ignored.
- **`rttMs` is a `Float`**, not an Integer, because a loopback round trip takes
  tens of microseconds and would otherwise always read as `0`. It is measured
  with the monotonic clock across the send and receive, not taken from any
  protocol field.
- **`ttl`** is the hop limit observed on the reply, which is a property of the
  *responder's* stack: 64 on typical Unix hosts, 128 on Windows, 255 from many
  routers. It is not a hop count.
- **Only `Ok` carries measurements.** Every other status reports `rttMs = 0.0`,
  `ttl = 0`, and `size = 0`, because there was no reply to measure.

## Arguments

`ping` takes a host string or an `Address`, then three optional arguments.

`timeoutMs` follows the language timeout convention
(`./mfb spec language builtin-functions`): omitted waits indefinitely — and so can
never report `Timeout` — `0` performs one immediate check, a positive value bounds
the wait, and a negative value raises.

`ttl` defaults to `64` and must be `1` to `255`; it is written into a one-byte IP
header field. `size` defaults to `56`, may be `0` (a bare echo header), and is
capped at **8184** bytes. Every range is validated *before* the resolver runs or
any handle is opened, so a rejected call allocates nothing.

The 8184 cap is the smallest maximum across the supported platforms, published so
that one number is portable. Reaching it on macOS requires the backend to raise
the socket receive buffer: the default raw receive space there is 8192 bytes and
silently drops a reply at the documented maximum even though the request was sent,
which would surface as a bogus
`Timeout`.[[src/codegen/builtins/net/gen_ping.rs:PING_RECV_BUFFER]]

Exactly one echo request is sent per call. There is no retry, so a caller wanting
an average or a loss rate loops and aggregates. Only IPv4 destinations are
supported.

## Permission

Sending ICMP is unprivileged on macOS and Windows. On **Linux it is permitted only
when the process's group falls inside the `net.ipv4.ping_group_range` sysctl**,
and distributions disagree about the default — some ship a range covering every
ordinary user, some ship the empty range `1 0`, which denies everyone. A program
that pings must therefore be prepared for the call to raise on a machine where it
is not allowed, and that is a deployment fact rather than a rare edge case.

## Implementation model

Three backends, because the platforms differ in every fact the reply parser
depends on.[[src/codegen/builtins/net/gen_ping.rs:lower_net_ping_helper]]

| | macOS | Linux | Windows |
|---|---|---|---|
| Facility | `SOCK_DGRAM`/`IPPROTO_ICMP` | same | `iphlpapi` `IcmpSendEcho` |
| Reply buffer | IPv4 header present | bare ICMP message | `ICMP_ECHO_REPLY` struct |
| Reply TTL from | the IP header | an `IP_RECVTTL` control message | the reply struct |
| Echo identifier | preserved | rewritten by the kernel | managed by the OS |
| Socket demultiplexing | **none — every ICMP socket sees every reply** | per socket | n/a |

Two consequences follow. On macOS the reply match checks the echo **identifier and
sequence number**, because another process's ping is delivered to this socket
too; on Linux the identifier cannot be checked (the kernel replaced it) and need
not be (the kernel already demultiplexed), so the match is on sequence alone. And
because unrelated packets can arrive, a non-matching packet does not end the call:
the receive loop keeps polling against the deadline.

An ICMP error reply is matched back to its request through the original IP header
and first eight bytes that the error quotes — the quoted echo header carries the
sequence number that was sent.

Windows has no unprivileged ICMP socket at all (Winsock's raw ICMP requires
Administrator), so it uses the `iphlpapi` ICMP API, which builds, matches, and
times the echo itself. Its whole-millisecond `RoundTripTime` is not used for
`rttMs` — it would report `0` on loopback — so the exchange is timed with
`QueryPerformanceCounter` instead, mirroring the POSIX monotonic
clock.[[src/codegen/builtins/net/gen_ping.rs:lower_ping_windows]]

## See Also

* ./mfb man net ping — the signature, parameters, and error list
* ./mfb spec stdlib url — the other half of `net`, the URL model
* ./mfb spec language builtin-functions — the shared timeout convention
