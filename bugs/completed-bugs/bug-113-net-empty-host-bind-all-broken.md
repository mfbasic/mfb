# bug-113 — net::listenTcp/bindUdp empty-host "bind all interfaces" is broken: getaddrinfo(NULL, NULL, …) always fails

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7).
**Severity:** MED — a documented feature can never work on any target; a test
golden locks in the failure.
**Class:** correctness (docs-vs-behavior; dead intended path).

## Finding

`src/target/shared/code/net/mod.rs:341-366` (`lower_net_endpoint_helper`
null_host path) and `src/target/shared/code/net/io.rs:932-951`
(`lower_net_bind_udp_helper` null_host path).

For an empty host the helpers pass `node = NULL` to `getaddrinfo`, but the
`service` argument is **always NULL too** (the port is patched into `sin_port`
afterwards). POSIX requires at