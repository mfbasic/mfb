# net

TCP and UDP network sockets, host name resolution, and URL parsing

## Synopsis

```
IMPORT net
RES client = net::connectTcp(net::toUrl("http://example.com/").host, 80)
RES listener = net::listenTcp("0.0.0.0", 8080)
RES peer = net::accept(listener)
net::writeText(peer, "hello\n")
RES sock = net::bindUdp("0.0.0.0", 0)
net::sendTextTo(sock, net::localAddress(sock), "ping")
```

## Description

The `net` package provides the host network interface: resolving names to
addresses, opening and serving connected TCP streams, binding connectionless UDP
datagram sockets, and parsing URLs. `net::lookup` resolves a host to addresses;
`net::connectTcp` opens an outbound stream; `net::listenTcp` and `net::accept`
serve inbound streams; and `net::bindUdp` opens a datagram socket. Sockets are
read and written with the read/write family, configured with the timeout
setters, and inspected with the address queries.

The package defines six socket-and-address types. `Socket` is a connected TCP
stream, `Listener` is a TCP socket in the listening state, and `UdpSocket` is a
bound UDP datagram socket; all three are opaque, owned, non-copyable resource
handles. Each is closed automatically by lexical drop when its binding leaves
scope, so `net::close` is needed only to release a handle earlier; using a
resource after it is closed raises an error, and resource handles cannot be
stored as collection elements or carried in records. Because they cannot live in
a `List`, there is no list-of-sockets poll overload — poll one `Socket` at a
time. `Address` is a plain record with a `host` field (`String`) and a `port`
field (`Integer`), returned by `net::lookup` and the address queries and accepted
as a destination by `net::connectTcp` and the UDP send functions. `Datagram`
(fields `from AS Address` and `bytes AS List OF Byte`) and `DatagramText` (fields
`from AS Address` and `value AS String`) are the records `net::receiveFrom` and
`net::receiveTextFrom` return, pairing each payload with its sender. [[src/builtins/net.rs:builtin_type_fields]]

`net::toUrl` parses an absolute `http`/`https` href into a `Url`, an ordinary
copyable value record with fields `scheme`, `username`, `password`, `host`,
`path`, `query`, and `fragment` (`String`) plus `port` (`Integer`); a universal
`toString` on a `Url` renders it back to an href. To connect to a parsed `Url`,
pass its parts: `net::connectTcp(u.host, u.port)`. [[src/builtins/net_package.mfb:__net_toUrl]]

Hosts are UTF-8 `String` values naming either a textual IP address or a name
passed to the host resolver; `"0.0.0.0"`, `"::"`, or an empty string bind every
local interface, and a local port of 0 requests an ephemeral port that
`net::localAddress` reads back. `net::lookup` returns only IPv4 results. Ports
and timeouts are `Integer` values, and every timeout is expressed in
milliseconds; a read or write timeout of 0 blocks indefinitely, while a positive
value bounds a single read or write and fails it when the deadline elapses. Most
transfer functions come in a paired byte/text form: the byte form transfers a
`List OF Byte` verbatim, while the text form transfers a `String`'s UTF-8 bytes
directly and validates received bytes as UTF-8. TCP streams report end of stream
as an error rather than an empty result, so read in a loop until a
connection-closed error is raised; UDP preserves message boundaries, delivering
each datagram whole or rejecting an oversized one rather than truncating it.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | raised by any function when an internal allocation fails, such as a NUL-terminated host copy, a resource handle, an `Address` or datagram record, or the buffer or collection holding a result [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77020004` | `ErrEncoding` | raised by `readText` and `receiveTextFrom` when the received bytes are not valid UTF-8 [[src/target/shared/code/error_constants.rs:ERR_ENCODING_CODE]] |
| `77030004` | `ErrResourceClosed` | raised by any function taking a `Socket`, `Listener`, or `UdpSocket` when the resource has already been closed, and by the address queries and timeout setters when the host OS reports the handle is otherwise no longer usable [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77030006` | `ErrCloseFailed` | raised by `close` when the host OS reports a failure while releasing the handle [[src/target/shared/code/error_constants.rs:ERR_CLOSE_FAILED_CODE]] |
| `77050002` | `ErrInvalidArgument` | raised by `read`, `readText`, `receiveFrom`, and `receiveTextFrom` when `maxBytes` is not positive, and by `poll`, `setReadTimeout`, and `setWriteTimeout` when a timeout is negative [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050003` | `ErrInvalidFormat` | raised by `toUrl` when the href is malformed — missing scheme separator, empty or unterminated host, or a non-digit or out-of-range port [[src/builtins/net_package.mfb:__net_parsePort]] |
| `77050007` | `ErrUnsupported` | raised by `toUrl` when the URL scheme is neither `http` nor `https` [[src/builtins/net_package.mfb:__net_toUrl]] |
| `77050008` | `ErrTimeout` | raised by `connectTcp` when a positive `timeoutMs` is given and the connection does not complete before the deadline elapses [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77070001` | `ErrAddressInvalid` | raised by `lookup`, `listenTcp`, and `bindUdp` when a host or port cannot be resolved into a local endpoint, and by the address queries and UDP receive functions when an address reported by the OS cannot be represented as an `Address` [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_INVALID_CODE]] |
| `77070002` | `ErrAddressNotFound` | raised by `lookup`, `connectTcp`, `sendTo`, and `sendTextTo` when a host cannot be resolved, including when it is malformed or has no address record [[src/target/shared/code/error_constants.rs:ERR_ADDRESS_NOT_FOUND_CODE]] |
| `77070003` | `ErrNetworkFailed` | raised by `connectTcp`, `listenTcp`, `accept`, `bindUdp`, and the UDP send and receive functions when a socket cannot be created, bound, listened on, accepted, or transferred for a host reason other than a timeout [[src/target/shared/code/error_constants.rs:ERR_NETWORK_FAILED_CODE]] |
| `77070004` | `ErrConnectionClosed` | raised by `read`, `readText`, `write`, and `writeText` when the peer has closed the connection (an end-of-stream read) or the transfer otherwise fails for a reason other than a timeout [[src/target/shared/code/error_constants.rs:ERR_CONNECTION_CLOSED_CODE]] |
| `77070005` | `ErrReadTimeout` | raised by `read`, `readText`, `receiveFrom`, and `receiveTextFrom` when the socket's read timeout elapses before any data arrives [[src/target/shared/code/error_constants.rs:ERR_READ_TIMEOUT_CODE]] |
| `77070006` | `ErrWriteTimeout` | raised by `write`, `writeText`, `sendTo`, and `sendTextTo` when the socket's write timeout elapses before the data could be sent [[src/target/shared/code/error_constants.rs:ERR_WRITE_TIMEOUT_CODE]] |
| `77070007` | `ErrMessageTooLarge` | raised by `receiveFrom` and `receiveTextFrom` when an incoming datagram's payload exceeds `maxBytes`, and by `sendTo` and `sendTextTo` when a payload is too large to send as a single datagram [[src/target/shared/code/error_constants.rs:ERR_MESSAGE_TOO_LARGE_CODE]] |
