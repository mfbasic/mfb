# network-server

A broadcast server that speaks plain TCP, UDP, or TLS, and optionally runs each
connection on a worker thread. `examples/network-client` is the matching client.

## Building

This example is **two projects**: the executable here, and the `connworker`
package under [`worker/`](worker/). The package has to be built first and
installed at `packages/connworker.mfp` before the executable will build:

```sh
mfb build examples/network-server/worker
mkdir -p examples/network-server/packages
cp examples/network-server/worker/connworker.mfp examples/network-server/packages/
mfb build examples/network-server
```

`scripts/build-examples.sh` does exactly this before its per-target builds.

The split is not organisational taste — it is forced by the language. A thread
entry point must be an `EXPORT ISOLATED FUNC` of an *imported package*, and
`IMPORT self` is rejected in an executable:

```
error[2-201-0019 IMPORT_SELF_IN_EXECUTABLE]: IMPORT self is only valid in a package project
```

so a program that spawns a worker needs a package to spawn it from. The wire
helpers (`counterText`, `hasBye`, the tick constants) live there too, so the
single-threaded server and the workers share one wire format rather than two
copies of it.

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
(SANs `DNS:localhost`, `IP:127.0.0.1`) for local use. Note that
`examples/network-client`'s TLS attempt against them is *expected* to fail:
`tls::connect` always verifies against the system trust store and has no way to
accept a self-signed certificate — see
[`bug-477`](../../bugs/bug-477-tls-connect-cannot-accept-a-self-signed-certificate.md).
`openssl s_client -CAfile certs/cert.pem` drives the server fine.
