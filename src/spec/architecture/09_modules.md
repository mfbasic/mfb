# Module Map

A map of the compiler's source modules and their responsibilities.

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | CLI, manifest validation, project orchestration, package commands. |
| `src/lexer.rs` | Source tokenization. |
| `src/ast.rs` | Parser, AST model, AST JSON output. |
| `src/resolver.rs` | Name resolution and import/package symbol checks. |
| `src/monomorph.rs` | Template/generic expansion into concrete AST. |
| `src/typecheck.rs` | Type system, expression checking, flow validation. |
| `src/ir.rs` | Shared compiler IR and AST-to-IR lowering. |
| `src/binary_repr.rs` | MFPC binary representation lowering, encoding, decoding, package ABI inspection. |
| `src/builtins/mod.rs` | Built-in package dispatch and parameter name tables. |
| `src/builtins/fs.rs` | Filesystem built-in signatures and validation. |
| `src/builtins/general.rs` | General and collection built-in signatures. |
| `src/builtins/io.rs` | IO built-in signatures and validation. |
| `src/builtins/json.rs` | JSON built-in type and call signatures. |
| `src/builtins/math.rs` | Math built-in signatures and constants. |
| `src/builtins/strings.rs` | String built-in signatures. |
| `src/builtins/thread.rs` | Thread built-in type and call signatures. |
| `src/unicode_backend.rs` | Unicode normalization and grapheme code generation. |
| `src/unicode_runtime_tables.rs` | Compile-time Unicode lookup tables embedded in generated code. |
| `src/target.rs` | Target parsing, backend registry, backend dispatch. |
| `src/target/shared/lower.rs` | Shared IR-to-NIR lowering pass. |
| `src/target/shared/nir.rs` | Native IR and import/runtime-call lowering. |
| `src/target/shared/runtime.rs` | Runtime helper discovery and helper ABI metadata. |
| `src/target/shared/validate.rs` | Native target, NIR, capability, and plan validation. |
| `src/target/shared/plan.rs` | Shared native plan lowering. |
| `src/target/shared/code/` | Shared native code-plan lowering (directory module with builder submodules). |
| `src/target/macos_aarch64/*` | macOS aarch64 backend wrappers and platform behavior. |
| `src/target/linux_aarch64/*` | Linux aarch64 backend wrappers and platform behavior. |
| `src/target/package_mfp` | MFP package container writer. |
| `src/arch/aarch64/*` | AArch64 ABI, operations, and binary instruction encoding. |
| `src/os/macos/*` | Mach-O object planning and executable writing. |
| `src/os/linux/flavor.rs` | Linux flavor enumeration (glibc/musl) and suffix/interpreter selection. |
| `src/os/linux/link.rs` | ELF object planning and executable writing. |
| `src/os/linux/object.rs` | ELF container layout planning. |
| `src/man/*` | Built-in package/function help text. |
| `src/spec/*` | Embedded language/architecture specification (`mfb spec`). |
| `src/rules.rs` | Diagnostic display support. |
| `src/numeric.rs` | Numeric parsing and representation helpers. |
