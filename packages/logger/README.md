# logger

`logger` is a source package for one-line UTC log entries. It formats each call
as `[TIMESTAMP] [LEVEL] MESSAGE`, delivers it to every common backend first, and
then to the backends configured for that exact `LogLevel`. A backend in both
lists is deliberately called twice.

Build and test the package from the repository root:

```sh
mfb build packages/logger
mfb test packages/logger
packages/logger/smoke.sh target/release/mfb
packages/logger/runtime-smoke.sh target/release/mfb
packages/logger/check-doc-examples.sh target/release/mfb
```

Copy `packages/logger/logger.mfp` beside the consuming project and declare it in
that project's `project.json`:

```json
{
  "packages": [
    { "name": "logger", "version": "=0.1.0", "source": "file:packages/logger.mfp" }
  ]
}
```

## Backends and routing

`ConsoleBackend` calls `io::print` with the formatted entry. It carries no
configuration, so it is written `logger::ConsoleBackend[]`.

`FileBackend` calls `fs::writeAll` on the supplied open `fs::File`, appending one
newline per dispatch. `NetworkBackend` calls `tcp::write` on the supplied
connected `tcp::Socket`, also newline-terminated — that newline is what frames
the stream, so a reader splits entries exactly as it would reading the file.
`CustomBackend` receives the structured `LogEntry`, including the same UTC ISO
timestamp every other backend got for that `log` call.

Routing is by exact level, never by threshold: a backend listed under `WARN` does
not see `ERROR`. List it under every level you want it to see, or put it in
`common`.

The caller creates, flushes when needed, and closes every file and socket. The
package never opens, closes, flushes, retries, reconnects, batches, or filters
those handles; a backend's failure propagates to the caller and stops the
fan-out for that entry.

```mfb
IMPORT fs
IMPORT logger
IMPORT net
IMPORT tcp

FUNC audit(entry AS logger::LogEntry) AS Nothing
  ' Use entry.level, entry.timestamp, and entry.message.
END FUNC

FUNC main() AS Integer
  RES file AS fs::File = fs::open("app.log", "a")
  RES listener = tcp::listen("127.0.0.1", 0)
  LET address = tcp::localAddress(listener)
  RES socket AS tcp::Socket = tcp::connect("127.0.0.1", address.port)
  RES peer AS tcp::Socket = tcp::accept(listener)
  LET console AS logger::LogBackend = logger::ConsoleBackend[]
  LET fileBackend AS logger::LogBackend = logger::FileBackend[file := file]
  LET network AS logger::LogBackend = logger::NetworkBackend[socket := socket]
  LET custom AS logger::LogBackend = logger::CustomBackend[callback := audit]
  LET byLevel AS Map OF logger::LogLevel TO List OF logger::LogBackend = Map OF logger::LogLevel TO List OF logger::LogBackend {logger::LogLevel.ERROR := [custom]}
  LET app = logger::Logger[common := [console, fileBackend, network], backends := byLevel]

  logger::log(app, logger::LogLevel.INFO, "started")
  logger::log(app, logger::LogLevel.ERROR, "request failed")
  RETURN 0
END FUNC
```

The `ERROR` call above reaches `custom` after all common backends, while the
`INFO` call does not. `net` is imported because the example reads the `port`
field of the `net::Address` returned by `tcp::localAddress`; imports are not
transitive.

## One entry is one line

A carriage return or line feed in the message is written as `\r` or `\n` — two
ordinary characters — rather than ending the line. Without that, a message built
from untrusted input could forge a complete, indistinguishable entry at any level
and any timestamp, on the console, in the file, and on the socket alike.
`CustomBackend` still receives the unmodified `LogEntry`, because it is handed
structure rather than a line.

A `Logger` is an ordinary value with no lock of its own: two threads logging to
backends that share one `fs::File` or one `tcp::Socket` can interleave their
bytes. Give each thread its own handle, or serialize the calls yourself.

Generate the package metadata page with:

```sh
mfb pkg doc packages/logger/logger.mfp --out logger-doc.html
```
