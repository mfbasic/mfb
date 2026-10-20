# Changelog

All notable changes to the `logger` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to
the exported API, or to the line format a backend writes, requires a new major
version.

## 1.0.0

The first stable release. It replaces the unreleased 0.1.0 working version, whose
exact-level routing map and per-level fan-out are gone. Migrating a 0.1.0 caller
is a mechanical rewrite of the `Logger` value; see the list below.

### Changed

- `Logger` routes by threshold, the convention of mainstream logging libraries.
  It has two fields, `minLevel` and `backends`. An entry reaches every backend
  when its level is at least `minLevel`. Replace `common` and the `LogLevel`
  routing map with `backends` and a `minLevel`.
- Entries below `minLevel` are dropped before the timestamp is read or the line is
  formatted.
- A backend failure no longer stops the fan-out. `logger::log` attempts every
  backend, then fails with the first failure, so a broken destination never
  starves the others. A failure used to stop delivery at the failing backend.
- Escaping is reversible and covers the whole class of characters that can end a
  line, hide text, or drive a terminal. A backslash is written `\\`, a carriage
  return `\r`, a line feed `\n`, a tab `\t`, and every other control, format,
  separator, or unassigned character `\u{HEX}`, including ESC, NUL, DEL, C1
  controls, U+2028, U+2029, bidirectional overrides, and non-ASCII spaces. Before,
  only CR and LF were escaped, and a literal backslash was not doubled, so an
  escape in the output could be confused with the text it came from.
- `logger::log` returns at once when no backend is configured.
- The timestamp is read only for an entry that is delivered.

### Added

- `logger::isEnabled(config, level)` reports whether an entry at a level would be
  delivered, so a caller can skip building an expensive message.
- Tests for threshold routing, reversible escaping of each character class,
  failure isolation, and the first-failure rule.

### Documented

- A `Logger` cannot cross to another thread: its type holds file and socket
  resources, which the compiler refuses to send with `thread::start`. Each thread
  builds its own `Logger`, so no two threads write through one handle.
- Per-destination thresholds are not built in. Use one `Logger` per threshold, or
  a `CustomBackend` whose callback checks `entry.level`.
