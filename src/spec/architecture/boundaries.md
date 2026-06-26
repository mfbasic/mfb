# Current Implementation Boundaries

Current implementation boundaries to know when extending the compiler.

The following boundaries are important when extending the compiler:

- Native executable support is target-limited to `macos-aarch64` and
  `linux-aarch64`.
- Native runtime-call support covers `io.*`, `fs.*`, and `thread.*` built-ins.
  `math` and `strings` operations are code-generated inline and do not go
  through the runtime-helper capability gate. `json` built-ins have no native
  backend support and are binary representation-only.
- `target/shared/validate.rs::validate_project` is currently a no-op, so target
  project-level checks must be implemented elsewhere or added there.
- Manifest source `include` and `exclude` patterns are not currently enforced
  by source discovery.
- *NOTE: `package_format.md` specifies that packages may carry a cryptographic
  signature. The current package writer always emits unsigned containers with
  `signatureType = 0` and `signatureLength = 0`.*
- `mfb pkg add` currently supports only absolute `file://` package URLs.
- Linux builds emit two output files per build (`-glibc.out` and `-musl.out`).
  The glibc flavor links `libpthread.so.0` separately; musl bundles pthread in
  `libc.musl-aarch64.so.1`.
- macOS executable writing supports `libSystem` imports only.

These boundaries should be treated as implementation facts, not necessarily
language or package-format design goals.
