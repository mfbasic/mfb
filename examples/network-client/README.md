# network-client

The client half of [`examples/network-server`](../network-server). Tries all
three transports against the same host and port, in order — TCP, then UDP,
then TLS — running each attempt to completion before starting the next, so
one server mode being up never hides what the other two would have done. It
leaves an attempt after the fifth `Update` line, sending `BYE` first.

## Build and run

```sh
mfb build examples/network-client
```

- macOS / Linux — `./examples/network-client/build/network-client.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/network-client`,
  then run `examples/network-client/build/network-client.exe`

Pair it with a running `examples/network-server` (its own README covers
building and running it):

```sh
./examples/network-client/build/network-client.out
```

## Usage

```
network-client [--host HOST] [--port PORT] [--server-name NAME] [--allow-self-signed]
```

- `--host HOST` — server address. Default `127.0.0.1`.
- `--port PORT` — server port. Default `7000` — matches
  `network-server`'s own default (`examples/network-server/src/main.mfb`).
- `--server-name NAME` — name the TLS certificate must match, sent as SNI.
  Defaults to `HOST`.
- `--allow-self-signed` — accept a TLS chain whose only fault is an
  untrusted root; the name and dates are still checked. Needed against
  `network-server`'s shipped self-signed certificate:
  ```sh
  ./examples/network-client/build/network-client.out --server-name localhost --allow-self-signed
  ```
  Without it, the TLS attempt is expected to fail — that failure is the point
  of running it without the flag.
- `--help` — show the usage text and exit.

## Network dependency

Connects to a real TCP/UDP/TLS endpoint at `HOST:PORT` — nothing works
without a reachable `network-server` (or something else speaking its wire
protocol) on that host and port.
