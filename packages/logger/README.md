# logger

`logger` is a source package for one-line UTC log entries. It formats each call
as `[TIMESTAMP] [LEVEL] MESSAGE`, delivers every common backend first, and then
delivers the backends configured for that exact `LogLevel`. A backend that occurs
in both lists is deliberately called twice.

Build and test the package from the repository root:

```sh
mfb build packages/logger
mfb test packages/logger
packages/logger/smoke.sh target/release/mfb
packages/logger/runtime-smoke.sh target/release/mfb
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

`ConsoleBackend` calls `io::print` with the formatted entry. Its exported
`use_colors` field is a version-0.1.0 configuration placeholder: it currently
does not emit terminal color sequences.

`FileBackend` calls `fs::writeAll` on the supplied open `fs::File`, appending one
newline for every dispatch. `NetworkBackend` calls `tcp::write` on the supplied
connected `tcp::Socket` and sends no added newline. `CustomBackend` receives the
structured `LogEntry`, including the same UTC ISO timestamp used by every other
backend for that `log` call.

The caller creates, flushes when needed, and closes every file and socket. The
package never opens, closes, flushes, retries, reconnects, batches, or filters
those handles; backend failures propagate to the caller.

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
  LET console AS logger::LogBackend = logger::ConsoleBackend[use_colors := FALSE]
  LET fileBackend AS logger::LogBackend = logger::FileBackend[file := file]
  LET network AS logger::LogBackend = logger::NetworkBackend[socket := socket]
  LET custom AS logger::LogBackend = logger::CustomBackend[callback := audit]
  LET byLevel AS Map OF logger::LogLevel TO List OF logger::LogBackend = _
    Map OF logger::LogLevel TO List OF logger::LogBackend {logger::LogLevel.ERROR := [custom]}
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

Generate the package metadata page with:

```sh
mfb pkg doc packages/logger/logger.mfp --out logger-doc.html
```
