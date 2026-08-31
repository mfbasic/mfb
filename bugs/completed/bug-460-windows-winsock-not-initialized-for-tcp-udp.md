# bug-460: a Windows `tcp`- or `udp`-only program never initializes Winsock

Last updated: 2026-08-30
Effort: small (< 1h)
Severity: HIGH
Class: Platform regression (every socket call fails; misleading diagnostic)

Status: Fixed (plan-110-F Phase 2)
Regression Test: `tests/cli_windows_winsock_startup.rs` — four cases: a tcp-only, a udp-only and a
tls-only program must initialize Winsock in the entry, and a socket-free one must not.

## Symptom

On Windows, the first socket call of a `tcp`- or `udp`-only program raises:

```
Error: 7-707-0001
Network host, address, or port is invalid.
```

Measured on box 2230 (Windows 11, 10.0.26100.9168):

```basic
IMPORT tcp
FUNC main AS Integer
  RES s = tcp::listen("127.0.0.1", 0)     ' raises 7-707-0001
  RETURN 0
END FUNC
```

The diagnostic names neither Winsock nor initialization, and the address it calls invalid is a
literal `127.0.0.1`.

It hides behind `net::lookup`. Add one line and everything works:

```basic
IMPORT net
IMPORT tcp
FUNC main AS Integer
  LET a = net::lookup("127.0.0.1", 0)     ' <-- this makes the rest work
  RES s = tcp::listen("127.0.0.1", 0)     ' listen ok, host=127.0.0.1, port>0=TRUE
  RETURN 0
END FUNC
```

## Root cause

Winsock refuses every call until `WSAStartup` has run in the process. The compiler emits it once, in
the program entry, gated so a socket-free program stays byte-identical
(`src/codegen/engine/builder/mod.rs`):

```rust
let uses_net = runtime_symbols
    .iter()
    .any(|symbol| symbol.starts_with("_mfb_rt_net_") || symbol.starts_with("_mfb_rt_tls_"));
```

Those were the only two families that touched Winsock when it was written. plan-110-B/C moved the
transports into `tcp` and `udp`, which carry their **own** runtime families (`_mfb_rt_tcp_*`,
`_mfb_rt_udp_*`), and nothing updated the predicate. A tcp- or udp-only program therefore emitted no
`WSAStartup` at all, and every subsequent call returned `WSANOTINITIALISED` — which the helper's
error classification maps onto `ErrNetworkFailed`/`7-707-0001`.

`net::lookup` masked it because it is a `_mfb_rt_net_` symbol: one call anywhere in the program
flipped the gate, and the initialization then covered every later `tcp::`/`udp::` call.

Measured directly in the emitted entry rather than inferred:

```
$ mfb build -target windows-x86_64 -ncode <tcp-only project>
  entry instructions: 509, external calls: SetConsoleOutputCP, BCryptGenRandom
$ mfb build -target windows-x86_64 -ncode <net-only project>
  entry instructions: 295, external calls: WSAStartup, SetConsoleOutputCP, BCryptGenRandom
```

Note that the import TABLE lists `WSAStartup` in both — it is attributed to the helper that needs
it, so grepping the whole `.ncode` proves nothing. The gate lives in the entry, so the entry's own
instruction stream is what has to be read; the regression test does that by brace-matching the
`entrySymbol` function out of the dump.

## Fix

Name every family that reaches Winsock:

```rust
const WINSOCK_FAMILIES: [&str; 4] =
    ["_mfb_rt_net_", "_mfb_rt_tcp_", "_mfb_rt_udp_", "_mfb_rt_tls_"];
let uses_net = runtime_symbols
    .iter()
    .any(|symbol| WINSOCK_FAMILIES.iter().any(|f| symbol.starts_with(f)));
```

Verified on box 2230 after the fix: `tcp::listen` as the first socket call returns
`host=127.0.0.1`, an ephemeral port, and the full connect/accept/read/write/remoteAddress,
variable-host, wildcard-bind, `connect(Address)` and udp send/receive matrix all pass.

## Note on the related carried-in defect

This is **not** the same as plan-110-B §C5 (Windows loopback reporting `0.0.0.0` and
`ErrNetworkFailed`). That one was recorded against a `net` program, which did initialize Winsock,
and was fixed earlier in plan-110-D. bug-460 is a distinct regression introduced by the transport
split itself, and would have been invisible to any probe that called `net::lookup` first — which
every earlier Windows probe did.
