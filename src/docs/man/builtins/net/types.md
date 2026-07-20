# types

the net package record types

## Synopsis

```
net::Address
net::Datagram
net::DatagramText
net::Url
```

## Package

net

## Imports

```
IMPORT net
```

`net` is a built-in package, so `IMPORT net` needs no manifest
dependency. [[src/builtins/net.rs:augmented_project]]

## Description

The `net` package defines four record types. All four are ordinary, flat,
copyable value records: they hold no resource handle and no hidden state, so
they can be assigned, copied, stored in collections, held in other records,
returned from a function, and sent across threads. All four are recognized once
`IMPORT net` is in scope, and each resolves under either spelling — bare
(`LET a AS Address = …`) or package-qualified (`LET a AS net::Address = …`). The
conventional spelling is bare for `Address`, `Datagram`, and `DatagramText`, and
qualified for `Url` (`LET u AS net::Url = net::toUrl(href)`), which is the form
used throughout this manual. [[src/builtins/net.rs:is_builtin_type]]

`Address` is the package's endpoint value: a host plus a port. It is produced by
`net::lookup` (as a `List OF Address`), `net::localAddress`, and
`net::remoteAddress`, and it is accepted as a destination by `net::connectTcp`,
`net::sendTo`, and `net::sendTextTo`. [[src/builtins/net.rs:builtin_type_fields]]

`Datagram` and `DatagramText` are the UDP receive results. They pair one received
payload with the `Address` it came from, which is what makes a connectionless
socket usable: `net::receiveFrom` returns the byte form and
`net::receiveTextFrom` returns the text form, whose `value` has already been
validated as UTF-8. UDP preserves message boundaries, so each record carries
exactly one whole datagram. [[src/builtins/net.rs:call_return_type_name]]

`Url` is the parsed form of an absolute `http`/`https` href, produced by
`net::toUrl`. Every field is normalized on parse: the scheme is lowercased, `port`
is filled in with the scheme default (80 for `http`, 443 for `https`) when the
href carries none, `path` is `"/"` when the href had no path, and absent userinfo,
query, and fragment become empty strings rather than being unset. A universal
`toString` on a `Url` renders it back to an href. To open a connection to a parsed
`Url`, pass its parts: `net::connectTcp(u.host, u.port)`. [[src/builtins/net_package.mfb:__net_toUrl]]

The package's other three types — `Socket`, `Listener`, and `UdpSocket` — are
opaque, owned, non-copyable resource handles with no readable fields, so they are
not tabulated here; see `mfb man net`. [[src/builtins/net.rs:resource_close_function]]

## Types

### net::Address

A network endpoint: a host and a port. [[src/builtins/net.rs:ADDRESS_TYPE]]

| Field | Type | Description |
| --- | --- | --- |
| `host` | `String` | The host as text: either a textual IP address (`"127.0.0.1"`, or an IPv6 literal without brackets) or a name to be passed to the host resolver. `"0.0.0.0"`, `"::"`, and `""` mean every local interface when binding. |
| `port` | `Integer` | The TCP or UDP port number, `0 .. 65535`. A local port of `0` requests an ephemeral port, which `net::localAddress` reads back once assigned. |

### net::Datagram

One received UDP datagram, as raw bytes, paired with its sender. Returned by `net::receiveFrom`. [[src/builtins/net.rs:DATAGRAM_TYPE]]

| Field | Type | Description |
| --- | --- | --- |
| `from` | `Address` | The sender's endpoint, as reported by the OS. |
| `bytes` | `List OF Byte` | The datagram payload, verbatim and whole; one complete datagram, never a truncated fragment. |

### net::DatagramText

One received UDP datagram, decoded as text, paired with its sender. Returned by `net::receiveTextFrom`. [[src/builtins/net.rs:DATAGRAM_TEXT_TYPE]]

| Field | Type | Description |
| --- | --- | --- |
| `from` | `Address` | The sender's endpoint, as reported by the OS. |
| `value` | `String` | The datagram payload decoded as UTF-8; a datagram whose bytes are not valid UTF-8 is rejected with an error rather than delivered here. |

### net::Url

An absolute `http` or `https` URL, parsed into its components. Returned by `net::toUrl`. [[src/builtins/net_package.mfb:Url]]

| Field | Type | Description |
| --- | --- | --- |
| `scheme` | `String` | `"http"` or `"https"`, lowercased. No other scheme parses. |
| `username` | `String` | The userinfo before `:`; `""` when the href carries no userinfo. |
| `password` | `String` | The userinfo after `:`; `""` when absent. |
| `host` | `String` | The registered name or IP literal; an IPv6 literal appears without its surrounding brackets. |
| `port` | `Integer` | The explicit port, or the scheme default (`80` for `http`, `443` for `https`) when the href gave none. |
| `path` | `String` | The path, always beginning with `"/"`; `"/"` when the href had no path. |
| `query` | `String` | The raw query string without the leading `?`; `""` when absent. Decode it with `net::parseQuery`. |
| `fragment` | `String` | The raw fragment without the leading `#`; `""` when absent. |

## See also

- `mfb man net`
- `mfb man net lookup`
- `mfb man net receiveFrom`
- `mfb man net toUrl`
- `mfb man net parseQuery`
- `mfb man http` — the HTTP client and server built on `net::Url`
