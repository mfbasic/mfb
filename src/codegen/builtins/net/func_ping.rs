//! `net::ping` — descriptor entry (native OS-seam, plan-110-A). Two overloads: a
//! host string and a `net::Address`; the Address form lowers to the `net.pingAddr`
//! code form (an `os_alias`). Both carry three optional trailing arguments, padded
//! by `builder_values`.
//!
//! The native backends live in [`super::gen_ping`]; see that module for why macOS,
//! Linux and Windows need three different implementations.

use crate::codegen::registry::{Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

const INTRO: &str = r#"Send one ICMP echo request and report how the host answered."#;

const DESC: &str = r#"`net::ping` sends a single ICMP echo request to a host and returns a `PingResult`
describing the outcome. The destination is named either by a host string — a name
such as `"example.com"` or a textual IP address — or by an `Address` record, whose
`host` field supplies the destination and whose `port` field is **ignored**: ICMP
is not a transport protocol and has no port. The `address` on the returned result
likewise always carries port `0`.

A reachable host answers with `PingStatus.Ok`, and only then do `rttMs`, `ttl`,
and `size` carry measured values: the round-trip time in milliseconds as a
`Float`, the TTL observed on the reply, and the number of payload bytes echoed
back. Every other status zeroes all three, because there is nothing to report.
`rttMs` is a `Float` rather than an `Integer` because a loopback round trip takes
tens of microseconds and would otherwise always read as `0`.

Four outcomes are statuses, not errors, because they are answers about the
network rather than failures of the call:

- `PingStatus.Ok` — an echo reply came back.
- `PingStatus.Timeout` — nothing came back before `timeoutMs` elapsed. The
  `address` field still reports the destination that was aimed at.
- `PingStatus.Unreachable` — a router reported the destination unreachable, and
  `address` names the router that said so.
- `PingStatus.TtlExceeded` — the request outlived its `ttl` in transit and a
  router reported it, so `address` names the hop where it expired. Lowering `ttl`
  deliberately is how a traceroute is built.

Genuine failures raise errors instead. A host that does not resolve raises
`ErrAddressInvalid`. An out-of-range argument raises `ErrInvalidArgument`, and is
rejected before anything is resolved or opened. Most importantly, **an operating
system that refuses to let this program use ICMP raises an error, not a status**:
that is a fact about the machine, not about the peer, and reporting it as
`Unreachable` would be a lie about the network.

That refusal is a real deployment condition, not a theoretical one. Sending ICMP
needs no special privilege on macOS or Windows, but Linux permits it only when the
process's group falls inside the `net.ipv4.ping_group_range` sysctl. Distributions
disagree about the default — some ship a range covering every ordinary user, and
some ship an empty range that denies everyone — so a program that pings should be
prepared for the call to fail outright on a machine where it is not allowed.

`timeoutMs` follows the language timeout convention (see `mfb spec language
builtin-functions` → "Timeout convention"). Omitted, the call waits indefinitely
for a reply, which means it can never report `Timeout`. `0` performs one immediate
check and reports `Timeout` unless a reply is already waiting. A positive value
bounds the wait. A negative value raises `ErrInvalidArgument`.

`ttl` sets the outgoing hop limit and must be `1` to `255`; it defaults to `64`.
`size` is the number of payload bytes to send, defaults to `56`, and may be `0` to
send a bare echo header. Its maximum is `8184`, the smallest limit across the
supported platforms, so one value is portable everywhere.

Only IPv4 destinations are supported. `net::ping` sends exactly one echo request
and does not retry, so a caller that wants an average or a loss rate calls it in a
loop and aggregates the results itself."#;

const EX: &str = r#"Ping the loopback address and report the round trip:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET result = net::ping("127.0.0.1", 1000)
  MATCH result.status
    CASE PingStatus.Ok
      io::print("up in " & toString(result.rttMs) & " ms")
    CASE PingStatus.Timeout
      io::print("no answer")
    CASE ELSE
      io::print("unreachable")
  END MATCH
  RETURN 0
END FUNC
```

Ping a resolved `Address`, and treat a refusal by the OS as the error it is:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  LET target = collections::get(net::lookup("127.0.0.1"), 0)
  LET result = net::ping(target, 1000) TRAP(e)
    io::print("cannot ping: " & e.message)
    RETURN 1
  END TRAP
  io::print(result.address.host & " ttl " & toString(result.ttl))
  RETURN 0
END FUNC
```

Find the first hop by sending a request that is meant to expire:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET hop = net::ping("example.com", 2000, 1)
  IF hop.status = PingStatus.TtlExceeded THEN
    io::print("first hop is " & hop.address.host)
  END IF
  RETURN 0
END FUNC
```"#;

/// `abi_function` body for `net::ping` — selects the host or `Address` form from the
/// code-form name and calls the shared emitter, which dispatches by platform family.
pub(crate) fn lower_ping(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_ping::lower_net_ping_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        ctx.call == "net.pingAddr",
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(crate::codegen::os::socket::shared::void_result(ctx.call))
}

const TIMEOUT_DESC: &str = "Optional. How long to wait for a reply, in milliseconds. Omit to wait indefinitely (in which case `Timeout` can never be reported); `0` checks once and reports `Timeout` unless a reply is already waiting; a positive value bounds the wait; a negative value raises `ErrInvalidArgument`.";
const TTL_DESC: &str = "Optional, defaulting to `64`. The outgoing hop limit, `1` to `255`. A request that expires in transit comes back as `PingStatus.TtlExceeded` naming the hop that dropped it. Outside that range raises `ErrInvalidArgument`.";
const SIZE_DESC: &str = "Optional, defaulting to `56`. The number of payload bytes to send, `0` to `8184`. `0` sends a bare echo header. The maximum is the smallest limit across the supported platforms, so it is portable. Outside that range raises `ErrInvalidArgument`.";

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let ret = || ParameterType::named(super::PING_RESULT_TYPE);
    pkg.add_function(RegistryFunction {
        name: "ping",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, Integer, Integer, Integer or Address, Integer, Integer, Integer"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![
                    super::req(
                        "host",
                        "The host name or textual IP address to ping. Passed to the host resolver as written; a value that does not resolve raises `ErrAddressInvalid`.",
                        &[],
                        ParameterType::String,
                    ),
                    super::opt("timeoutMs", TIMEOUT_DESC, ParameterType::Integer),
                    super::opt("ttl", TTL_DESC, ParameterType::Integer),
                    super::opt("size", SIZE_DESC, ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_ping, &[]),
            },
            Implementation {
                params: vec![
                    super::req(
                        "address",
                        "A destination record supplying the host to ping, typically from `net::lookup`. Its `port` field is ignored: ICMP has no transport port.",
                        &[],
                        ParameterType::named(super::ADDRESS_TYPE),
                    ),
                    super::opt("timeoutMs", TIMEOUT_DESC, ParameterType::Integer),
                    super::opt("ttl", TTL_DESC, ParameterType::Integer),
                    super::opt("size", SIZE_DESC, ParameterType::Integer),
                ],
                return_type: ret(),
                errors: vec![],
                body: super::native_body(lower_ping, &["pingAddr"]),
            },
        ],
    });
}
