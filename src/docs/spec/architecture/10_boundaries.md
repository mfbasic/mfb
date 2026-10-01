# Current Implementation Boundaries

Current implementation boundaries to know when extending the compiler.

The following boundaries are important when extending the compiler:

- Native executable support is target-limited to `macos-aarch64`,
  `linux-aarch64`, `linux-x86_64`, `linux-riscv64`, and `windows-x86_64`.
  [[src/target.rs:NATIVE_BACKENDS]]
- Runtime-call support is checked against the selected backend's capability
  table; native-direct builtins bypass this gate. See
  `./mfb spec architecture native` for helper selection and validation.
  [[src/target/shared/validate/capabilities.rs:validate_capabilities]]
- Target project-level validation is a no-op, so those checks are distributed
  across the front-end, NIR, plan, code-plan, and OS/linker passes rather than
  centralized.
- Packages may be emitted signed (`mfb build --sign owner`, ed25519) or, by
  default, unsigned; the reader accepts both. The on-disk signature-header
  encoding is owned by `./mfb spec package container-format`.
- `mfb pkg add` accepts an absolute `file://` package URL or an
  `<owner>#<package>[@version]` registry ident.
- Linux console builds emit two executable files per build (`-glibc.out` and `-musl.out`),
  one per libc flavor; macOS console builds emit a single Mach-O and Windows
  builds emit a PE32+ `.exe`. App artifacts are specified in
  `./mfb spec architecture artifacts`. The per-flavor library and
  `DT_NEEDED` selection and the macOS import sets (console `libSystem`, TLS, and
  app-mode toolkit frameworks) are owned by
  `./mfb spec linker static-and-dynamic-output`.

These boundaries should be treated as implementation facts, not necessarily
language or package-format design goals.

## See Also

* ./mfb spec package container-format — the package signature-header encoding
* ./mfb spec linker static-and-dynamic-output — per-OS library and flavor selection
