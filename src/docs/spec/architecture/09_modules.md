# Module Map

A map of the compiler's source modules and their responsibilities.

| Module | Responsibility |
| --- | --- |
| CLI / orchestration[[src/main.rs]] | CLI, manifest validation, project orchestration, package commands. |
| Lexer[[src/lexer.rs]] | Source tokenization. |
| Parser & AST[[src/ast/]] | Parser, AST model, source discovery, AST JSON output. |
| Name resolver[[src/resolver/]] | Name resolution and import/package symbol checks. |
| Monomorphizer[[src/monomorph/]] | Template/generic expansion into concrete AST. |
| Shape pass[[src/ir/shape.rs]] | The source rules whose evidence lowering erases (named-argument binding, EXIT/inline-TRAP boundaries, TESTING assertion arguments, literal spellings, native CONST/FREE facts, imported-package metadata), checked over the concrete HIR with lowering's typing oracle. Its stream renders first. |
| IR semantic verifier[[src/ir/verify/]] | IR semantic verification — the single source of truth for every other rule, run on both source-lowered IR and decoded-package IR. |
| Escape analysis[[src/ir/resource_escape.rs]] | Resource escape analysis (resource ownership/scope; see `./mfb spec language resource-management`). |
| IR & lowering[[src/ir/]] | Shared compiler IR and AST-to-IR lowering. |
| Internal sigil naming[[src/internal_name.rs]] | Compiler-internal sigil symbol naming for injected source packages. |
| Binary representation[[src/binary_repr/]] | MFPC binary representation lowering, encoding, decoding, package ABI inspection. |
| Source formatter[[src/fmt.rs]] | Lexical source formatter for `mfb fmt`. |
| Documentation renderer[[src/doc/mod.rs]] | Documentation model and HTML renderer for `mfb doc` / `mfb pkg doc`. |
| Project audit[[src/audit/]] | `mfb audit`: fallible-call/cleanup/permission/dependency reporting (collect/json/text/report). |
| Built-in dispatch[[src/codegen/builtins/mod.rs]] | Built-in package dispatch: the aggregate helpers iterate the descriptor registry (membership, arity, return type, expected arguments, argument types, default padding, builtin types), plus the parameter-name and source-injection tables. |
| Built-in descriptors[[src/codegen/registry/mod.rs]] | The clean-room builtin registry — the compiler-owned source of truth for every builtin package's functions, overloads, parameters, return types, builtin types, source-injection rule, and constants. Each `src/codegen/builtins/<pkg>/mod.rs` registers its package into the registry, which derives every data-only answer (membership, arity, return type, expected arguments) and resolves the argument-dependent ones. |
| Filesystem built-ins[[src/codegen/builtins/fs/mod.rs]] | Filesystem built-in signatures and validation. |
| General built-ins[[src/codegen/builtins/general/mod.rs]] | General-purpose built-in signatures. |
| Collections built-ins[[src/codegen/builtins/collections/mod.rs]] | Collections (`List`/`Map`) built-in signatures. |
| IO built-ins[[src/codegen/builtins/io/mod.rs]] | IO built-in signatures and validation. |
| JSON built-ins[[src/codegen/builtins/json/mod.rs]] | JSON built-in type and call signatures. |
| Math built-ins[[src/codegen/builtins/math/mod.rs]] | Math built-in signatures and constants. |
| String built-ins[[src/codegen/builtins/strings/mod.rs]] | String built-in signatures. |
| Thread built-ins[[src/codegen/builtins/thread/mod.rs]] | Thread built-in type and call signatures. |
| Date/time built-ins[[src/codegen/builtins/datetime/mod.rs]] | Date/time built-in signatures. |
| Terminal built-ins[[src/codegen/builtins/term/mod.rs]] | Terminal (`TermSize`) built-in signatures; the colour members speak `color::Color`. |
| Network built-ins[[src/codegen/builtins/net/mod.rs]] | Network (`Socket`/`Listener`/UDP) built-in signatures. |
| TLS built-ins[[src/codegen/builtins/tls/mod.rs]] | TLS (`tls::Socket`) built-in signatures. |
| HTTP built-ins[[src/codegen/builtins/http/mod.rs]] | HTTP built-in signatures. |
| CSV built-ins[[src/codegen/builtins/csv/mod.rs]] | CSV built-in signatures. |
| Regex built-ins[[src/codegen/builtins/regex/mod.rs]] | Regex built-in signatures. |
| `errorCode` package[[src/codegen/builtins/errorcode/mod.rs]] | `errorCode` integer-constant package. |
| Resource-type registry[[src/codegen/resource/mod.rs]] | Data-driven resource-type registry. |
| MFBASIC-source built-in packages[[src/codegen/builtins/]] | MFBASIC-source built-in packages injected at build (`collections`, `crypto`, `csv`, `datetime`, `encoding`, `http`, `json`, `net`, `regex`, `vector`); the regex Unicode file is a plain source companion, not a package source. |
| Unicode constant-fold oracles[[src/unicode/backend.rs]] | Compile-time (constant-fold) Unicode oracles: upper/lower/caseFold/normalizeNfc/graphemes on static strings. |
| Unicode lookup tables[[src/unicode/runtime_tables.rs]] | Compile-time Unicode lookup tables embedded in generated code. |
| Target registry & dispatch[[src/target.rs]] | Target parsing, backend registry, backend dispatch. |
| Shared IR-to-NIR entry[[src/target/shared/lower.rs]] | Shared IR-to-NIR entry: merges installed packages into IR, then lowers. |
| Native IR (NIR)[[src/target/shared/nir/]] | Native IR and import/runtime-call lowering. |
| Runtime helper discovery[[src/target/shared/runtime/]] | Runtime helper discovery and helper ABI metadata. |
| Native validation[[src/target/shared/validate/mod.rs]] | Native target, NIR, capability, and plan validation. |
| Shared native plan[[src/target/shared/plan/]] | Shared native plan lowering. |
| Shared native code generator[[src/codegen/]] | Shared native code-plan lowering (directory module with builder submodules). |
| macOS aarch64 backend[[src/target/macos_aarch64/]] | macOS aarch64 backend wrappers and platform behavior (AppKit app mode included). |
| Linux aarch64 backend[[src/target/linux_aarch64/]] | Linux aarch64 backend wrappers and platform behavior. |
| Linux x86-64 backend[[src/target/linux_x86_64/]] | Linux x86-64 backend wrappers and platform behavior. |
| Linux RISC-V 64 backend[[src/target/linux_riscv64/]] | Linux RISC-V 64 backend wrappers and platform behavior. |
| Linux GTK4 app-mode backend[[src/target/linux_gtk/]] | Shared GTK4 app-mode backend for the Linux targets. |
| MFP package writer[[src/target/package_mfp]] | MFP package container writer. |
| AArch64 backend[[src/arch/aarch64/]] | AArch64 ABI, operations, and binary instruction encoding. |
| x86-64 backend[[src/arch/x86_64/]] | x86-64 ABI, operations, and binary instruction encoding. |
| RISC-V 64 backend[[src/arch/riscv64/]] | RISC-V 64 ABI, operations, and binary instruction encoding. |
| macOS object/linker[[src/os/macos/]] | Mach-O object planning and executable writing. |
| Linux flavor selection[[src/os/linux/flavor.rs]] | Linux flavor enumeration (glibc/musl) and suffix/interpreter selection. |
| Linux ELF linker[[src/os/linux/link/]] | ELF object planning and executable writing. |
| Linux ELF object planning[[src/os/linux/object.rs]] | ELF container layout planning. |
| Built-in help text[[src/codegen/registry]] | Built-in package/function help text. |
| Embedded specification[[src/docs/spec/]] | Embedded language/architecture specification (`mfb spec`). |
| Diagnostic display[[src/rules/]] | Diagnostic display support. |
| Numeric helpers[[src/numeric.rs]] | Numeric parsing and representation helpers. |

## See Also

* ./mfb spec language — the language these modules implement
* ./mfb spec language resource-management — the resource/escape model behind the compiler's escape analysis
