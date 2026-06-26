# Artifact Summary

Every build artifact, the flag that produces it, and what it contains.

| Artifact | Command | Producer | Meaning |
| --- | --- | --- | --- |
| `<name>.ast` | `mfb build -ast` | `src/ast.rs` | Parsed source tree before monomorphization. |
| `<name>.ir` | `mfb build -ir` | `src/ir.rs` | Typed, architecture-independent compiler IR. |
| `<name>.hex` | `mfb build -br` | `src/binary_repr.rs` | Hex dump of MFPC binary representation. |
| `<name>.nir` | `mfb build -nir` | `src/target/shared/nir.rs` | Native IR for the selected target. |
| `<name>.nplan` | `mfb build -nplan` | `src/target/shared/plan.rs` | Native function/storage/call plan. |
| `<name>.nobj` | `mfb build -nobj` | `src/os/*/object.rs` | OS object/container layout plan. |
| `<name>.ncode` | `mfb build -ncode` | `src/target/shared/code/` | AArch64 code-generation plan. |
| `<name>.out` | `mfb build` executable (macOS) | `src/os/macos/link.rs` | Native executable (Mach-O). |
| `<name>-glibc.out` | `mfb build` executable (Linux) | `src/os/linux/link.rs` | Native executable (ELF, glibc). |
| `<name>-musl.out` | `mfb build` executable (Linux) | `src/os/linux/link.rs` | Native executable (ELF, musl). |
| `<name>.mfp` | `mfb build` package | `src/target/package_mfp` | Compiled MFB package. |
