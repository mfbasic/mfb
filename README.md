# MFBASIC

**MFBASIC** is a modern, functional dialect of BASIC, and `mfb` is its
ahead-of-time compiler and toolchain. It compiles a project directly to a native
executable — or to a signed, distributable `.mfp` package — with no garbage
collector, no runtime VM, and no external language runtime.

```basic
IMPORT io

SUB main()
  io::print("Hello, world")
END SUB
```

```console
$ mfb build examples/hello_world
$ ./examples/hello_world/hello_world.out
Hello, world
```

## Why MFBASIC

MFBASIC keeps BASIC's readable, keyword-oriented syntax but rebuilds the
semantics around value ownership and automatic error propagation:

- **Immutable by default.** `LET` bindings never change; `MUT` opts into
  reassignment; `RES` owns a unique handle (file, socket, …).
- **Value ownership with deterministic cleanup.** Every value has a single owner
  and is reclaimed when its scope exits — no garbage collector, no reference
  counting, no user-visible `free`. Resources close on every exit path,
  including error routing.
- **Checked arithmetic.** Integer overflow fails instead of wrapping; an observed
  `Float` is never NaN or infinity.
- **Errors without exceptions.** Every call either produces its value or fails
  with an `Error`. Success auto-unwraps; failure auto-propagates to the nearest
  `TRAP` or to the caller. No `TRY`, no `GOTO`, no `null`, no `Option`.
- **Pattern matching over closed unions.** `TYPE` records, `UNION` sums, and
  `ENUM` members, deconstructed with `MATCH` and checked for exhaustiveness at
  compile time.
- **Functional core.** First-class `LAMBDA` values, the `|>` pipeline operator,
  and free functions in packages — no classes, no methods, no `null`.
- **Isolated threads.** Workers share nothing and communicate over bounded, typed
  message queues.

For a complete one-page walkthrough, run `mfb man tour` (or `mfb man tour c`,
`java`, `go`, `typescript`, `python` to see the same ideas in your language's
terms).

## Getting started

MFBASIC is built from source with the Rust toolchain pinned in
`rust-toolchain.toml`.

```console
$ cargo build --release
$ ./target/release/mfb help
```

Create and build a project:

```console
$ mfb init myapp        # scaffold an executable project
$ mfb build myapp       # validate and compile to a native executable
$ ./myapp/myapp.out     # run it
```

## Project layout

An MFBASIC project is a directory with a `project.json` manifest and source
files (`.mfb`). A minimal executable manifest:

```json
{
  "name": "hello_world",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [
    { "root": "src", "role": "main", "include": ["**/*.mfb"] }
  ],
  "entry": "main",
  "targets": ["native"]
}
```

See the [`examples/`](examples/) directory for complete programs — including
`hello_world`, `life` (a full-screen Conway's Game of Life built on the `term::`
TUI package), `hangman`, `hello_input`, and `audio`.

## The `mfb` toolchain

| Command | Purpose |
| --- | --- |
| `mfb init <path>` | Scaffold a new executable project |
| `mfb init-pkg <path>` | Scaffold a new package (library) project |
| `mfb build [path]` | Validate and compile a project (native or `.mfp`) |
| `mfb test [path]` | Build and run the project's `TESTING` blocks |
| `mfb fmt [path]` | Format source (indentation / capitalization) |
| `mfb audit [path]` | Report security and code-audit findings |
| `mfb doc [path]` | Render HTML docs from source |
| `mfb man [pkg] [func]` | Show built-in package and function help |
| `mfb spec [topic]` | Show the language specification |
| `mfb pkg <cmd>` | Manage packages (add, publish, verify, …) |
| `mfb repo <cmd>` | Repository owner registration, auth, and trust |

Run `mfb <command> --help` for the options of any command. `mfb build --target
<os-arch>` cross-compiles; `mfb build --app` builds a windowed application.

## Standard library

Built-in packages, brought in with `IMPORT` and called with `::`, cover:

- **Core & data** — `general`, `types`, `collections`, `strings`, `unicode`,
  `math`, `bits`, `filters`, `lambda`
- **Encoding & data formats** — `encoding`, `json`, `csv`, `regex`
- **I/O & system** — `io`, `fs`, `term`, `datetime`, `os`
- **Concurrency** — `thread`
- **Networking** — `net`, `tls`, `http`
- **Security** — `crypto`
- **Math** — `vector`
- **Testing** — `testing` (with `mfb test`)

Browse them with `mfb man`, or `mfb man <package> <function>` for any built-in.

## Documentation

- `mfb man tour` — a one-page tour of the whole language
- `mfb man` — the built-in package and function reference
- `mfb spec language` — the full language specification (with a worked example)
- `mfb spec architecture` — how `mfb build` turns source into a native
  executable or a signed `.mfp` package

## License

MFBASIC is released under the [MIT](LICENSE) license.
