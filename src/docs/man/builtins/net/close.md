# close

Close a network resource and release its OS handle.

## Synopsis

```
net::close(sock AS Socket) AS Nothing
net::close(listener AS Listener) AS Nothing
net::close(sock AS UdpSocket) AS Nothing
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

`net::close` releases the operating-system socket behind a network resource and
marks the handle closed, so any later `net::` call that takes the same value
raises an error rather than touching a stale descriptor. It spans all three
`net` handle types: a connected TCP `Socket`, a TCP `Listener`, and a bound UDP
`UdpSocket`. [[src/builtins/net.rs:resolve_call]]

`net::close` is the only `net` call that **consumes** its handle. Every other
function borrows the resource and leaves it open; `close` moves the value into
the call, after which it cannot be referenced again.
[[src/builtins/net.rs:consumes_argument]]

Closing a `Socket` or `UdpSocket` tears down the connection or binding, so a peer
reading from a closed connection observes the end of the stream. Closing a
`Listener` stops it from accepting new connections but does not affect sockets
already returned by `net::accept`; each of those is an independent resource with
its own lifetime.

Closing is otherwise automatic. Every `net` resource is closed by lexical drop
when the binding holding it leaves scope, so `net::close` is needed only when the
handle must be torn down earlier — to free a listening port for reuse, to let a
peer observe the end of the stream promptly, or to bound how many descriptors a
long-running program holds open. Closing a resource and then letting it drop is
safe: the drop sees the closed flag and does nothing.
[[src/builtins/net.rs:resource_close_function]]

Unlike `tls::close`, `net::close` treats an already-closed handle as an error
rather than a no-op. The handle record's closed word is checked first, and a
non-zero value refuses the call. The same word also carries the *moved* bit that
`thread::transfer` sets, so a handle that was transferred to another thread is
refused too — but with `ErrResourceMoved`, which names the real reason it is
unusable, instead of `ErrResourceClosed`. The closed flag is set before the
result of the host `close` is examined, so a host failure surfaces
`ErrCloseFailed` exactly once and a second `net::close` on the same value is
refused rather than closing a descriptor number that may by then name an
unrelated file. [[src/target/shared/code/fs/io.rs:lower_fs_close_helper]]

## Overloads

**`net::close(sock AS Socket) AS Nothing`**

Closes a connected TCP socket, tearing down the connection.

**`net::close(listener AS Listener) AS Nothing`**

Closes a TCP listener, stopping further accepts. Sockets already accepted from
it are unaffected and stay open.

**`net::close(sock AS UdpSocket) AS Nothing`**

Closes a bound UDP socket, releasing its binding.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `resource` | `Socket`, `Listener`, or `UdpSocket` | The network resource to close. Consumed by the call and unusable afterwards. This parameter also accepts the alternate named-argument spellings `sock` and `listener`, so `net::close(resource := s)`, `net::close(sock := s)`, and `net::close(listener := l)` all bind position 0. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `close` returns no value. After a successful return the OS handle has been released and the resource is marked closed; the value has been consumed and must not be used again. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | The resource has already been closed, whether by an earlier `net::close` on the same value or by a prior scope-drop. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77030009` | `ErrResourceMoved` | The handle was moved to another thread by `thread::transfer` and is no longer usable by the sender. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_MOVED_CODE]] |
| `77030006` | `ErrCloseFailed` | The host reports a failure while releasing the descriptor. The handle is still marked closed, so it cannot be closed a second time. [[src/target/shared/code/error_constants.rs:ERR_CLOSE_FAILED_CODE]] |

## Examples

Release a listening port as soon as it is no longer needed:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  RES server = net::listenTcp("127.0.0.1", 0)
  LET bound = net::localAddress(server)
  RES client = net::connectTcp("127.0.0.1", bound.port)
  RES conn = net::accept(server)
  net::close(server)
  net::writeText(client, "hi")
  io::print(net::readText(conn, 16))
  RETURN 0
END FUNC
```

Close both UDP sockets explicitly at the end of an exchange:

```
IMPORT net

FUNC main AS Integer
  RES server = net::bindUdp("127.0.0.1", 0)
  RES client = net::bindUdp("127.0.0.1", 0)
  net::close(server)
  net::close(client)
  RETURN 0
END FUNC
```

## See also

- `mfb man net connectTcp`
- `mfb man net listenTcp`
- `mfb man net accept`
- `mfb man net bindUdp`
- `mfb man net read`
- `mfb man net write`
- `mfb man tls close`
