# bug-113 — net::listenTcp/bindUdp empty-host "bind all interfaces" is broken (getaddrinfo(NULL, NULL) always fails)

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7).
**Severity:** MED — a documented, all-target feature never works; the golden
locks in the failure.
**Class:** correctness (docs-vs-behavior; dead AI_PASSIVE path).

## Finding

`src/target/shared/code/net/mod.rs:341-366` (`lower_net_endpoint_helper`
null_host path) and net/io.rs:932-951 (`lower_net_bind_udp_helper` null_host
path).

For an empty host the helpers pass `node = NULL` to `getaddrinfo`, but the
`service` argument is always NULL too (the port is patched into `sin_port`
afterwards). POSIX requires at least one of node/service non-NULL;
glibc/musl/Darwin all return EAI_NONAME, so the intended AI_PASSIVE bind-all
path can never succeed — empty host always yields ERR_ADDRESS_INVALID.

The docs promise the opposite ("an empty host likewise binds all interfaces",
`src/docs/man/builtins/net/listenTcp.txt:21-22`, `bindUdp.txt:21`), the code
comment repeats the false claim, and the recorded golden locks in the failure:
`tests/rt-error/net/func_net_listenTcp_valid` `net::listenTcp("", 0)` → "Code:
77070001 Message: address invalid". The AI_PASSIVE hints flag and the null_host
branch are effectively dead.

## Trigger

`net::listenTcp("", 8080)` or `net::bindUdp("", 9999)` on any target →
ERR_ADDRESS_INVALID instead of binding 0.0.0.0.

## Fix

Pass the decimal port (or `"0"`) as the `service` string when host is NULL, so
`getaddrinfo(NULL, "8080", &hints_with_AI_PASSIVE, …)` returns the wildcard
address. Then re-baseline the rt-error golden (it currently encodes the bug).

## Prior art

audit-1-fs-net-thread.md reviews listen/bind but not this; the test golden
encodes the broken behavior without a bug doc.
