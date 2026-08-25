//! `net::poll` — descriptor entry (native OS-seam). Return-type-overloaded on
//! argument shape: a scalar `Socket` yields `Boolean` (readiness query), a
//! `List OF RES net.Socket` yields a borrowed `Socket` (readiness multiplex, the
//! `pollList` code form / `os_alias`). Two `Implementation`s, the datetime/net
//! idiom. Docs in `src/docs/man/builtins/net/poll.md`.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Test whether a socket has data ready to read, or wait for the first ready socket among many."#;

const DESC: &str = r#"`net::poll` reports whether a connected `Socket` is readable. It returns `TRUE`
when a following `net::read` or `net::readText` can proceed without blocking —
including the case where the peer has closed and that read would report end of
stream — and `FALSE` when nothing became readable before the deadline. The
socket is borrowed and inspected only: no data is consumed, so a `TRUE` result
leaves the bytes in place for the next read.

`timeoutMs` bounds the wait, in milliseconds, following the language timeout
convention (see `mfb spec language builtin-functions` → "Timeout convention").
When it is **omitted, `poll` blocks** until the socket becomes readable and then
returns `TRUE` (omit = unbounded). `0` is a non-blocking check that returns
immediately with the socket's current readiness (the old omitted behavior — pass
`, 0` for it). A positive value waits up to that long. A negative `timeoutMs` is
rejected with `ErrInvalidArgument`. Because the host `poll` takes a C `int`, a
value above 2147483647 is clamped to that, which is roughly 24 days.

Given a `List OF RES net::Socket`, `net::poll` becomes a **readiness multiplex**: it
blocks until at least one socket in the list is readable, then returns the first
ready one (lowest list index). The returned `Socket` is a **borrowed** pointer —
an alias of a list element, exactly like `collections::get` — so the list retains
ownership and closes every socket exactly once on scope exit; you may read,
`return`, or `thread::transfer` through the returned handle, but you do not close
it. An empty list is rejected with `ErrInvalidArgument`. Because the multiplex
yields a resource and has no not-ready value to return, expiry raises `ErrTimeout`
rather than returning a sentinel (it is a producing call). The elements must be
marked `RES` (`List OF RES net::Socket`); a bare `List OF Socket` is a compile error,
as resource elements always require the `RES` marker.

A signal that interrupts the underlying wait re-issues it rather than surfacing a
failure. `net::poll` complements `net::setReadTimeout`: `poll` asks whether a read
would block right now, while `setReadTimeout` bounds how long a read that does
block may wait."#;

const EX: &str = r#"Check whether data is waiting, without blocking (pass `0` for the immediate
check — omitting the timeout would instead block until the socket is readable):

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  io::print(toString(net::poll(conn, 0)))
  RETURN 0
END FUNC
```

Wait up to a second for a peer to send something:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::writeText(client, "hi")
  IF net::poll(conn, 1000) THEN
    io::print(net::readText(conn, 16))
  END IF
  RETURN 0
END FUNC
```

Wait for the first ready socket among several (the readiness multiplex). The
returned socket is borrowed — the list still owns and closes both:

```
IMPORT net
IMPORT io
IMPORT collections

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES clientA = net::connectTcp("127.0.0.1", bound.port)
  RES connA = net::accept(server)
  RES clientB = net::connectTcp("127.0.0.1", bound.port)
  RES connB = net::accept(server)
  MUT socks AS List OF RES net::Socket = []
  socks = collections::append(socks, connA)
  socks = collections::append(socks, connB)
  net::writeText(clientB, "hi")
  RES ready AS net::Socket = net::poll(socks)
  io::print(net::readText(ready, 16))
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::poll` — calls the shared `lower_net_*_helper`
/// emitter and finalizes.
pub(crate) fn lower_poll(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.call == "net.pollList" {
        super::gen_poll::lower_net_poll_list_helper(&symbol, ctx.platform_imports, ctx.platform)?
    } else {
        super::gen_poll::lower_net_poll_helper(&symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "poll",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Socket, Integer or List OF RES Socket, Integer"),
        internal_only: false,
        implementations: vec![
            // Scalar readiness query: `poll(Socket[, timeoutMs]) AS Boolean`.
            Implementation {
                params: vec![
                    super::req("sock", "An open connected socket, as returned by `net::connectTcp` or `net::accept`. It is borrowed and inspected for readiness only; no data is read and the handle is not consumed.", &[], super::socket()),
                    super::opt("timeoutMs", "Optional. Omit to block until a socket is readable; `0` is an immediate non-blocking check/scan; a positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative.", ParameterType::Integer),
                ],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: super::native_body(lower_poll, &[]),
            },
            // Readiness multiplex: `poll(List OF RES net.Socket[, timeoutMs]) AS
            // Socket` (borrowed). Emits the `net.pollList` code form.
            Implementation {
                params: vec![
                    super::req("socks", "A non-empty list of open connected sockets. Each is borrowed and inspected for readiness; the list keeps ownership. An empty list raises `ErrInvalidArgument`.", &[], ParameterType::list_of(super::socket())),
                    super::opt("timeoutMs", "Optional. Omit to block until a socket is readable; `0` is an immediate non-blocking check/scan; a positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative.", ParameterType::Integer),
                ],
                return_type: ParameterType::named(super::SOCKET_TYPE_ID),
                errors: vec![],
                body: super::native_body(lower_poll, &["pollList"]),
            },
        ],
    });
}
