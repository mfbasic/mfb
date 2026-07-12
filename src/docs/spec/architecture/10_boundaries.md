# Current Implementation Boundaries

Current implementation boundaries to know when extending the compiler.

The following boundaries are important when extending the compiler:

- Native executable support is target-limited to `macos-aarch64`,
  `linux-aarch64`, `linux-x86_64`, and `linux-riscv64`.
- Native runtime-call support covers the `crypto.*`, `datetime.*`, `fs.*`,
  `io.*`, `net.*`, `os.*`, `term.*`, `thread.*`, and `tls.*` built-ins. `math`, `strings`, and `general`
  operations are code-generated inline and do not go through the runtime-helper
  capability gate. The exact supported set is each backend's `runtime_calls`
  declaration.[[src/target/macos_aarch64/mod.rs:runtime_calls]] `json` built-ins
  have no dedicated native runtime helper — `json` is supplied as an injected
  MFBASIC source package.
- Target project-level validation is a no-op, so those checks are distributed
  across the front-end, NIR, plan, code-plan, and OS/linker passes rather than
  centralized.
- Packages may be emitted signed (`mfb build --sign owner`, ed25519) or, by
  default, unsigned; the reader accepts both. The on-disk signature-header
  encoding is owned by `./mfb spec package container-format`.
- `mfb pkg add` accepts an absolute `file://` package URL or an
  `<owner>#<package>[@version]` registry ident.
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
