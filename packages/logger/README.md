# logger

`logger` writes one-line UTC log entries. Each call is filtered by a minimum
level, formatted once as `[TIMESTAMP] [LEVEL] MESSAGE`, and delivered in order to
every backend you configured: the console, an open file, a connected socket, or
your own function.

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
    { "name": "logger", "version": "=1.0.0", "source": "file:packages/logger.mfp" }
  ]
}
```

## Configuring a logger

A `logger::Logger` is two fields: `minLevel`, the least severe level delivered,
and `backends`, every destination that receives a delivered entry, in order.

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
  LET app AS logger::Logger = logger::Logger[
    minLevel := logger::LogLevel.INFO,
    backends := [
      logger::ConsoleBackend[],
      logger::FileBackend[file := file],
      logger::NetworkBackend[socket := socket],
      logger::CustomBackend[callback := audit]
    ]
  ]

  logger::log(app, logger::LogLevel.DEBUG, "dropped: below minLevel")
  logger::log(app, logger::LogLevel.INFO, "started")
  logger::log(app, logger::LogLevel.ERROR, "request failed")
  RETURN 0
END FUNC
```

`net` is imported because the example reads the `port` field of the `net::Address`
returned by `tcp::localAddress`; imports are not transitive.

## Backends

`ConsoleBackend[]` calls `io::print` with the formatted line.

`FileBackend` calls `fs::writeAll` on an open `fs::File`, appending one newline per
entry. Open the file in append mode, flush when you need the bytes on disk, and
close it yourself.

`NetworkBackend` calls `tcp::write` on a connected `tcp::Socket`, newline-terminated.
That newline frames the stream: a reader splits entries exactly as it would
reading the file.

`CustomBackend` receives the structured `LogEntry`, including the timestamp that
every other backend got for that call. Call `logger::formatEntry` inside it to get
the same line the other backends write.

## Routing

A backend receives an entry when the entry's level is at least `minLevel`. The
levels, least to most severe, are `DEBUG`, `INFO`, `WARN`, `ERROR`.

To send different levels to different destinations, use one `Logger` per
threshold, or a `CustomBackend` whose callback checks `entry.level`. Use
`logger::isEnabled` to skip building a message that would be dropped.

## Failures

Every backend is attempted, even after an earlier one fails. If any fail,
`logger::log` then fails with the first failure, so a broken destination never
starves the others. The caller owns the handles, so the package never reconnects
or retries. Wrap a call in a `TRAP` where logging must not interrupt your code.

## One entry is one line

Before a message reaches any backend, every character that could end the line,
hide text, or reorder what a reader sees is written as an escape. A backslash
becomes `\\`, a carriage return `\r`, a line feed `\n`, a tab `\t`, and every
other control, format, separator, or unassigned character `\u{HEX}`. ESC becomes
`\u{1B}`, for example, so a message cannot drive the terminal.

The escape is reversible: every backslash in the output is doubled, so an escape
in a log line is never confused with text that looked like one. `CustomBackend`
still receives the unmodified `LogEntry`.

## Threads

A `Logger` cannot be sent to another thread: its type holds file and socket
resources, and the compiler refuses to pass it to `thread::start`. Each thread
that logs builds its own `Logger` inside its worker function, so no two threads
write through one handle:

```mfb
IMPORT logger
IMPORT thread

ISOLATED FUNC worker(w AS ThreadWorker OF Nothing TO Integer, name AS String) AS Integer
  LET own AS logger::Logger = logger::Logger[minLevel := logger::LogLevel.INFO, backends := [logger::ConsoleBackend[]]]
  logger::log(own, logger::LogLevel.INFO, "from " & name)
  RETURN 0
END FUNC

FUNC main() AS Integer
  RES a = thread::start(worker, "A")
  RES b = thread::start(worker, "B")
  LET x AS Integer = thread::waitFor(a)
  LET y AS Integer = thread::waitFor(b)
  RETURN x + y
END FUNC
```

To move a file or socket to a worker, use `thread::transfer`; a `Logger` built
from it then belongs to that worker.

## Generated documentation

```sh
mfb pkg doc packages/logger/logger.mfp --out logger-doc.html
```

See `CHANGELOG.md` for what changed in 1.0.0.
