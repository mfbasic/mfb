# lookup

Resolve a host name to a list of IPv4 network addresses.

## Synopsis

```
net::lookup(host AS String) AS List OF Address
net::lookup(host AS String, port AS Integer) AS List OF Address
```

## Package

`net`

## Imports

```
IMPORT net
```

`net` is a built-in package, so no manifest dependency is required.
[[src/builtins/net.rs:is_net_call]]

## Description

`net::lookup` hands `host` to the host resolver and returns the matching results
as a `List OF Address`. `host` may be a host name such as `"example.com"` or a
textual IP address; the resolver is asked for `SOCK_STREAM` endpoints. The result
list is built in the resolver's own order.
[[src/target/shared/code/net/io.rs:lower_net_lookup_helper]]

Only IPv4 results are returned. The resolver's answer chain is walked twice —
once to count `AF_INET` nodes and once to fill the list — and every node of any
other address family is skipped. The returned list can therefore be shorter than
the resolver's full answer, and it is empty when the host resolves but has no
IPv4 address. Note that the resolver failing outright is an error, not an empty
list. [[src/target/shared/code/net/io.rs:lower_net_lookup_helper]]

Each returned `Address` carries a `host` field holding the textual IPv4 address
and a `port` field holding the requested port. `port` does not influence
resolution: it is not passed to the resolver as a service name but written
directly into each result's port field, so that the `Address` can be handed
straight to `net::connectTcp` or a UDP send. When `port` is omitted the compiler
supplies `0`, and every returned `Address` carries port `0`.
[[src/builtins/net.rs:arity]]

`net::lookup` exposes no resolver metadata — no record types, TTLs, or canonical
names — and adds no caching of its own beyond whatever the host resolver
provides. It opens no sockets and has no side effects; the resolver's answer
chain is released on both the success and the failure exits.

## Overloads

**`net::lookup(host AS String) AS List OF Address`**

Resolves `host` and stamps every returned `Address` with port `0`.

**`net::lookup(host AS String, port AS Integer) AS List OF Address`**

Resolves `host` and stamps every returned `Address` with the given port.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `host` | `String` | The host name or textual IP address to resolve. Passed to the host resolver as written; a malformed or unresolvable value raises an error. [[src/builtins/net.rs:call_param_names]] |
| `port` | `Integer` | Optional, defaulting to `0`. The port recorded on every returned `Address`. It is stored on the results and does not influence resolution. [[src/target/shared/code/net/io.rs:lower_net_lookup_helper]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Address` | One `Address` per IPv4 result, each carrying the textual host and the requested port, in the resolver's order. Empty when the host resolves but has no IPv4 address. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77070002` | `ErrAddressNotFound` | The host could not be resolved — it is malformed, or it has no address record at all. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_NOT_FOUND_CODE]] |
| `77070001` | `ErrAddressInvalid` | A resolved address could not be converted to its textual form, so it cannot be represented as an `Address`. [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77010001` | `ErrOutOfMemory` | The NUL-terminated copy of `host`, the result list, or one of the `Address` records could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Resolve a host and inspect the first address:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  LET addresses = net::lookup("127.0.0.1", 80)
  LET first = collections::get(addresses, 0)
  io::print(first.host & " " & toString(first.port))
  RETURN 0
END FUNC
```

Resolve without a port and print every result:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET addresses = net::lookup("localhost")
  FOR EACH address IN addresses
    io::print(address.host)
  NEXT
  RETURN 0
END FUNC
```

## See also

- `mfb man net connectTcp`
- `mfb man net sendTo`
- `mfb man net bindUdp`
- `mfb man net localAddress`
- `mfb man net remoteAddress`
