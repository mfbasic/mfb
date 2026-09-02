# network-server

A broadcast server that speaks plain TCP, UDP, or TLS, and optionally runs each
connection on a worker thread. `examples/network-client` is the matching client.

## Building

One project, one command:

```sh
mfb build examples/network-server
```

The source is two files. [`src/main.mfb`](src/main.mfb) is the command line, the
three transports and the single-threaded polling loop; [`src/wire.mfb`](src/wire.mfb)
is the wire format (`counterText`, `hasBye`, the tick constants) plus the two
`ISOLATED FUNC` worker entries that `--thread` runs. They are one project, so the
helpers are `PUBLIC` and named without a prefix, and the single-threaded server
and the workers share one wire format rather than two copies of it.

This example used to be **two** projects — the executable plus a `connworker`
package — because a thread entry had to be an `EXPORT ISOLATED FUNC` reached
through an import, and an executable had no way to name one of its own. That
rule is gone: any `ISOLATED FUNC` is a thread entry, so `serveTcpConnections`
lives beside the code that starts it and `thread::start(serveTcpConnections, …)`
names it directly.

## Running

```sh
cd examples/network-server

./build/network-server.out --tcp
./build/network-server.out --udp
./build/network-server.out --tls certs/cert.pem certs/key.pem

./build/network-server.out --tcp --thread          # one worker serves every connection
./build/network-server.out --tls certs/cert.pem certs/key.pem --thread
```

`--host`, `--port` and `--help` apply throughout. Run with no arguments for the
full help screen.

## The protocol

Line-oriented UTF-8, every line newline-terminated:

| Direction | Line | When |
| --- | --- | --- |
| server → client | `Hello <uuid>` | once, on connect |
| server → client | `Update <uuid> NN` | every 500 ms, `NN` counting from `01` per connection |
| client → server | `BYE` | asks the server to drop the connection |

The server prints `Connected <uuid>` and `Disconnect <uuid>` to standard output.
A TCP or TLS client may also just close; UDP has no close to observe, so there
`BYE` — or fifteen seconds of silence — is the only signal.

## `--thread`

Without it, one thread runs one polling loop over every connection.

With it, the main thread does nothing but `accept`, and hands each accepted
socket to a worker over the thread **resource channel** (`thread::transfer` /
`thread::accept`). From that moment the connection belongs entirely to the
worker: it assigns the UUID, greets, ticks, and reports. `thread::transfer`
closes the sending end, so no two threads can ever hold the same socket, and no
locking is needed anywhere.

For `--tls` the handshake still completes on the main thread — `tls::accept`
*is* the handshake — and only the finished session crosses. The negotiated keys
and provider state travel with the handle, which is what
[`bug-464`](../../bugs/completed/bug-464-sockets-and-listeners-not-thread-sendable.md)
made possible.

One worker serves every connection rather than one worker per connection,
because a `Thread` handle is itself an owned handle and cannot be stored in a
collection:

```
error[2-203-0056 TYPE_COLLECTION_OWNERSHIP_VIOLATION]: ordinary collections
cannot store resource or thread ownership
```

so a thread-per-connection server would have no way to hold, poll, or reap its
handles.

`--udp --thread` is rejected: a UDP server has one bound socket shared by every
peer, so there is no per-connection handle to hand over.

## Certificates

`certs/cert.pem` and `certs/key.pem` are a self-signed `CN=localhost` pair
(SANs `DNS:localhost`, `IP:127.0.0.1`) for local use.

Being self-signed, they are not in any machine's trust store, so a client has to
be told to accept them. `examples/network-client` does that with an explicit
opt-in:

```sh
./build/network-client.out --port 7413 --server-name localhost --allow-self-signed
```

Without that flag the TLS attempt fails, which is the correct default —
`tls::connect` verifies against the system trust store. The flag relaxes the
trust anchor and nothing else: the certificate must still match `--server-name`
and must still be in date. `openssl s_client -CAfile certs/cert.pem` also drives
the server fine.

**The pair expires**, and re-generating it is one command:

```sh
certs/regenerate.sh
```

It is deliberately short-lived (397 days) because macOS refuses a TLS server
certificate whose validity window exceeds ~398 days, or which lacks an
`serverAuth` extended key usage, as "not standards compliant" — regardless of
what the client trusts. A longer-lived pair works on Linux and Windows and fails
on macOS only, which is a confusing way to discover the rule.
