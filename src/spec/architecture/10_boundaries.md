# Current Implementation Boundaries

Current implementation boundaries to know when extending the compiler.

The following boundaries are important when extending the compiler:

- Native executable support is target-limited to `macos-aarch64` and
  `linux-aarch64`.
- Native runtime-call support covers the `datetime.*`, `fs.*`, `general.*`,
  `io.*`, `net.*`, `term.*`, `thread.*`, and `tls.*` built-ins. `math` and
  `strings` operations are code-generated inline and do not go through the
  runtime-helper capability gate. The exact supported set is each backend's
  `runtime_calls` declaration. `json` built-ins have no dedicated native runtime
  helper — `json` is supplied as an injected MFBASIC source package.
- `target/shared/validate.rs::validate_project` is currently a no-op, so target
  project-level checks must be implemented elsewhere or added there.
- Packages may be emitted signed (`mfb build --sign owner`, ed25519,
  `signatureType = 1`) or, by default, unsigned (`signatureType = 0`,
  `signatureLength = 0`). The reader accepts both.
- `mfb pkg add` currently supports only absolute `file://` package URLs.
- Linux builds emit two output files per build (`-glibc.out` and `-musl.out`).
  The glibc flavor links `libpthread.so.0` separately; musl bundles pthread in
  `libc.musl-aarch64.so.1`.
- macOS executable writing imports from `libSystem` in console mode, and
  additionally from `Network.framework`/`libz` (TLS) and
  `libobjc`/`AppKit`/`Foundation` (app mode).

These boundaries should be treated as implementation facts, not necessarily
language or package-format design goals.
