# Module Map

A map of the compiler's source modules and their responsibilities.

| Module | Responsibility |
| --- | --- |
| CLI / orchestration[[src/main.rs]] | CLI, manifest validation, project orchestration, package commands. |
| Lexer[[src/lexer.rs]] | Source tokenization. |
| Parser & AST[[src/ast/]] | Parser, AST model, source discovery, AST JSON output. |
| Name resolver[[src/resolver/]] | Name resolution and import/package symbol checks. |
| Monomorphizer[[src/monomorph/]] | Template/generic expansion into concrete AST. |
| Source-syntax checker[[src/syntaxcheck/]] | Source-syntax checking only (named-argument binding, EXIT/inline-TRAP boundaries, lambda capture escape, package metadata). It emits no rule in `ir::RELOCATED_TO_IR_VERIFY` (enforced by a `debug_assert!` in `report`), but does emit the semantic rules that were never relocated — the `NATIVE_*` and `TESTING_*` families among them. |
| IR semantic verifier[[src/ir/verify/]] | IR semantic verification — the single source of truth for every **relocated** semantic rule (`RELOCATED_TO_IR_VERIFY`), run on both source-lowered IR and decoded-package IR. |
| Escape analysis[[src/escape.rs]] | Resource escape analysis (resource ownership/scope; see `./mfb spec language resource-management`). |
| IR & lowering[[src/ir/]] | Shared compiler IR and AST-to-IR lowering. |
| Internal sigil naming[[src/internal_name.rs]] | Compiler-internal sigil symbol naming for injected source packages. |
| Binary representation[[src/binary_repr/]] | MFPC binary representation lowering, encoding, decoding, package ABI inspection. |
| Source formatter[[src/fmt.rs]] | Lexical source formatter for `mfb fmt`. |
| Documentation renderer[[src/doc/mod.rs]] | Documentation model and HTML renderer for `mfb doc` / `mfb pkg doc`. |
| Project audit[[src/audit/]] | `mfb audit`: fallible-call/cleanup/permission/dependency reporting (collect/json/text/report). |
| Built-in dispatch[[src/builtins/mod.rs]] | Built-in package dispatch and parameter name tables. |
| Filesystem built-ins[[src/builtins/fs.rs]] | Filesystem built-in signatures and validation. |
| General built-ins[[src/builtins/general.rs]] | General-purpose built-in signatures. |
| Collections built-ins[[src/builtins/collections.rs]] | Collections (`List`/`Map`) built-in signatures. |
| IO built-ins[[src/builtins/io.rs]] | IO built-in signatures and validation. |
| JSON built-ins[[src/builtins/json.rs]] | JSON built-in type and call signatures. |
| Math built-ins[[src/builtins/math.rs]] | Math built-in signatures and constants. |
| String built-ins[[src/builtins/strings.rs]] | String built-in signatures. |
| Thread built-ins[[src/builtins/thread.rs]] | Thread built-in type and call signatures. |
| Date/time built-ins[[src/builtins/datetime.rs]] | Date/time built-in signatures. |
| Terminal built-ins[[src/builtins/term.rs]] | Terminal (`TermColor`/`TermSize`) built-in signatures. |
| Network built-ins[[src/builtins/net.rs]] | Network (`Socket`/`Listener`/UDP) built-in signatures. |
| TLS built-ins[[src/builtins/tls.rs]] | TLS (`TlsSocket`) built-in signatures. |
| HTTP built-ins[[src/builtins/http.rs]] | HTTP built-in signatures. |
| CSV built-ins[[src/builtins/csv.rs]] | CSV built-in signatures. |
| Regex built-ins[[src/builtins/regex.rs]] | Regex built-in signatures. |
| `errorCode` package[[src/builtins/errorcode.rs]] | `errorCode` integer-constant package. |
| Resource-type registry[[src/builtins/resource.rs]] | Data-driven resource-type registry. |
| MFBASIC-source built-in packages[[src/builtins/]] | MFBASIC-source built-in packages injected at build (`collections`, `crypto`, `csv`, `datetime`, `encoding`, `http`, `json`, `net`, `regex`, `vector`); the regex Unicode file is a plain source companion, not a package source. |
| Unicode constant-fold oracles[[src/unicode_backend.rs]] | Compile-time (constant-fold) Unicode oracles: upper/lower/caseFold/normalizeNfc/graphemes on static strings. |
| Unicode lookup tables[[src/unicode_runtime_tables.rs]] | Compile-time Unicode lookup tables embedded in generated code. |
| Target registry & dispatch[[src/target.rs]] | Target parsing, backend registry, backend dispatch. |
| Shared IR-to-NIR entry[[src/target/shared/lower.rs]] | Shared IR-to-NIR entry: merges installed packages into IR, then lowers. |
| Native IR (NIR)[[src/target/shared/nir/]] | Native IR and import/runtime-call lowering. |
| Runtime helper discovery[[src/target/shared/runtime/]] | Runtime helper discovery and helper ABI metadata. |
| Native validation[[src/target/shared/validate/mod.rs]] | Native target, NIR, capability, and plan validation. |
| Shared native plan[[src/target/shared/plan/]] | Shared native plan lowering. |
| Shared native code generator[[src/target/shared/code/]] | Shared native code-plan lowering (directory module with builder submodules). |
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
| Built-in help text[[src/docs/man/]] | Built-in package/function help text. |
| Embedded specification[[src/docs/spec/]] | Embedded language/architecture specification (`mfb spec`). |
| Diagnostic display[[src/rules/]] | Diagnostic display support. |
| Numeric helpers[[src/numeric.rs]] | Numeric parsing and representation helpers. |

## See Also

* ./mfb spec language — the language these modules implement
* ./mfb spec language resource-management — the resource/escape model behind the compiler's escape analysis
