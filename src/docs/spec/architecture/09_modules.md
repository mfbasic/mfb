# Module Map

A map of the compiler's source modules and their responsibilities.

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | CLI, manifest validation, project orchestration, package commands. |
| `src/lexer.rs` | Source tokenization. |
| `src/ast.rs` | Parser, AST model, source discovery, AST JSON output. |
| `src/resolver.rs` | Name resolution and import/package symbol checks. |
| `src/monomorph.rs` | Template/generic expansion into concrete AST. |
| `src/typecheck.rs` | Type system, expression checking, flow validation. |
| `src/escape.rs` | Resource escape analysis (resource ownership/scope; see `./mfb spec language resource-management`). |
| `src/ir.rs` | Shared compiler IR and AST-to-IR lowering. |
| `src/internal_name.rs` | Compiler-internal sigil symbol naming for injected source packages. |
| `src/binary_repr.rs` | MFPC binary representation lowering, encoding, decoding, package ABI inspection. |
| `src/fmt.rs` | Lexical source formatter for `mfb fmt`. |
| `src/doc.rs` | Documentation model and HTML renderer for `mfb doc` / `mfb pkg doc`. |
| `src/audit/*` | `mfb audit`: fallible-call/cleanup/permission/dependency reporting (collect/json/text/report). |
| `src/builtins/mod.rs` | Built-in package dispatch and parameter name tables. |
| `src/builtins/fs.rs` | Filesystem built-in signatures and validation. |
| `src/builtins/general.rs` | General-purpose built-in signatures. |
| `src/builtins/collections.rs` | Collections (`List`/`Map`) built-in signatures. |
| `src/builtins/io.rs` | IO built-in signatures and validation. |
| `src/builtins/json.rs` | JSON built-in type and call signatures. |
| `src/builtins/math.rs` | Math built-in signatures and constants. |
| `src/builtins/strings.rs` | String built-in signatures. |
| `src/builtins/thread.rs` | Thread built-in type and call signatures. |
| `src/builtins/datetime.rs` | Date/time built-in signatures. |
| `src/builtins/term.rs` | Terminal (`TermColor`/`TermSize`) built-in signatures. |
| `src/builtins/net.rs` | Network (`Socket`/`Listener`/UDP) built-in signatures. |
| `src/builtins/tls.rs` | TLS (`TlsSocket`) built-in signatures. |
| `src/builtins/http.rs` | HTTP built-in signatures. |
| `src/builtins/csv.rs` | CSV built-in signatures. |
| `src/builtins/regex.rs` | Regex built-in signatures. |
| `src/builtins/errorcode.rs` | `errorCode` integer-constant package. |
| `src/builtins/resource.rs` | Data-driven resource-type registry. |
| `src/builtins/*_package.mfb` | MFBASIC-source built-in packages injected at build (`collections`, `csv`, `datetime`, `http`, `json`, `net`, `regex`, `regex_unicode`). |
| `src/unicode_backend.rs` | Compile-time (constant-fold) Unicode oracles: upper/lower/caseFold/normalizeNfc/graphemes on static strings. |
| `src/unicode_runtime_tables.rs` | Compile-time Unicode lookup tables embedded in generated code. |
| `src/target.rs` | Target parsing, backend registry, backend dispatch. |
| `src/target/shared/lower.rs` | Shared IR-to-NIR entry: merges installed packages into IR, then lowers. |
| `src/target/shared/nir.rs` | Native IR and import/runtime-call lowering. |
| `src/target/shared/runtime.rs` | Runtime helper discovery and helper ABI metadata. |
| `src/target/shared/validate.rs` | Native target, NIR, capability, and plan validation. |
| `src/target/shared/plan.rs` | Shared native plan lowering. |
| `src/target/shared/code/` | Shared native code-plan lowering (directory module with builder submodules). |
| `src/target/macos_aarch64/*` | macOS aarch64 backend wrappers and platform behavior (`app.rs` = AppKit app mode). |
| `src/target/linux_aarch64/*` | Linux aarch64 backend wrappers and platform behavior (`gtk.rs` = GTK4 app mode). |
| `src/target/package_mfp` | MFP package container writer. |
| `src/arch/aarch64/*` | AArch64 ABI, operations, and binary instruction encoding. |
| `src/os/macos/*` | Mach-O object planning and executable writing. |
| `src/os/linux/flavor.rs` | Linux flavor enumeration (glibc/musl) and suffix/interpreter selection. |
| `src/os/linux/link.rs` | ELF object planning and executable writing. |
| `src/os/linux/object.rs` | ELF container layout planning. |
| `src/docs/man/*` | Built-in package/function help text. |
| `src/docs/spec/*` | Embedded language/architecture specification (`mfb spec`). |
| `src/rules.rs` | Diagnostic display support. |
| `src/numeric.rs` | Numeric parsing and representation helpers. |

## See Also

* ./mfb spec language — the language these modules implement
* ./mfb spec language resource-management — the resource/escape model behind `src/escape.rs`
