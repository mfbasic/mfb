# Current Implementation Boundaries

Current implementation boundaries to know when extending the compiler.

The following boundaries are important when extending the compiler:

- Native executable support is target-limited to `macos-aarch64` and
  `linux-aarch64`.
- Native runtime-call support covers the `datetime.*`, `fs.*`, `io.*`, `net.*`,
  `term.*`, `thread.*`, and `tls.*` built-ins. `math`, `strings`, and `general`
  operations are code-generated inline and do not go through the runtime-helper
  capability gate. The exact supported set is each backend's `runtime_calls`
  declaration.[[src/target/macos_aarch64/mod.rs:runtime_calls]] `json` built-ins
  have no dedicated native runtime helper — `json` is supplied as an injected
  MFBASIC source package.
- `target/shared/validate.rs::validate_project` is currently a no-op, so target
  project-level checks must be implemented elsewhere or added there.
- Packages may be emitted signed (`mfb build --sign owner`, ed25519) or, by
  default, unsigned; the reader accepts both. The on-disk signature-header
  encoding is owned by `./mfb spec package container-format`.
- `mfb pkg add` currently supports only absolute `file://` package URLs.
- Linux builds emit two output files per build (`-glibc.out` and `-musl.out`),
  one per libc flavor; macOS emits a single Mach-O. The per-flavor library and
  `DT_NEEDED` selection and the macOS import sets (console `libSystem`, TLS, and
  app-mode toolkit frameworks) are owned by
  `./mfb spec linker static-and-dynamic-output`.

These boundaries should be treated as implementation facts, not necessarily
language or package-format design goals.

## See Also

* ./mfb spec package container-format — the package signature-header encoding
* ./mfb spec linker static-and-dynamic-output — per-OS library and flavor selection
