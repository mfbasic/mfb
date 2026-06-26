# 21. Build Artifacts

MFBASIC uses source files for authoring, portable binary representation packages, and native binaries for executables.

| Artifact | Extension | Purpose |
|----------|-----------|---------|
| Source file | `.mfb` | Human-authored source code. The `.mfb` files selected by a project's `project.json` together form that project's source package (§13). |
| Package | `.mfp` | Architecture-neutral binary representation package. Its payload is **structured Binary Representation** (a faithful, versioned serialization of the compiler's IR) plus an embedded package manifest, public API metadata, dependency metadata, and optional native-link metadata. A compiled package can be built on one platform and imported on any platform that supports the same MFB binary representation/package version. |
| Executable | platform-native | Final application binary for the target OS/CPU. Executables compile application code plus imported `.mfp` packages to native code. |

At the artifact level: a **package** build stops at Binary Representation
(`IR -> Binary Representation (.mfp)`), a faithful structured serialization with no
flattening or structure loss. An **executable** build lowers IR to native, and
consuming a package decodes its Binary Representation back into IR, merges it into
the project IR, and lowers everything through one native path — there is no separate
package-to-native bridge. The full backend pipeline (IR → NIR → native), the
package decode-and-merge-under-identity-prefix sequence, and native dependency
resolution are owned by `./mfb spec architecture flows`.

Package compilation emits `.mfp` packages containing portable Binary Representation plus the embedded package manifest, dependency metadata, native-link metadata, and public API metadata needed for import, type checking, IR merging, and verification. This metadata includes each exported type and function's ownership properties: copyability, movability, resource-handle status, closure-capture requirements, thread-sendability, drop requirements, and collection element constraints. A package containing `LINK` declarations emits a reusable native binding `.mfp`: importers consume the package API and do not repeat the `LINK` declarations.

## 21.1 `.mfp` Binary Representation verification

Every `.mfp` package is verified before its Binary Representation can be decoded, merged into the project IR, or lowered. Verification operates on the **decoded IR**, not a flat opcode stream, and is deterministic: it must reject malformed packages before any package code runs. Because the Binary Representation is structured (nested regions with explicit ends), structure is explicit — there is no control-flow graph to reconstruct and no "jump into a trap or cleanup region" to reject — and most invariants reuse the compiler's existing IR-level passes.

In outline, the verifier checks that package metadata and public API metadata are
well-formed and consistent with the IR, that the signature/hash/trust record is
valid when the build mode requires it, and that the decoded IR satisfies the same
type-correctness, definite-assignment, resource-linearity, drop/cleanup,
`MATCH`-exhaustiveness, return/effect-agreement, single-bottom-`TRAP`, and
native-link-manifest invariants the compiler already enforces on the project's own
IR. The complete invariant catalogue and which invariants are re-checked at import
time are owned by `./mfb spec package verifier-rules`.

Verification failure rejects the package with a toolchain diagnostic. It is not recoverable by program `TRAP` code because no package code has started running.

> **Current implementation status.** The list above is the verifier contract this
> format is designed to support. The shipping decoder already verifies the package
> container before decoding. The outer `MFP` package magic and container major
> version (required to be `1`) are checked when the header is read
> (`read_mfp_header` in `src/main.rs`); the inner `MFPC` payload magic and MFPC
> major version (required to be `2`, `MFPC_MAJOR_VERSION`) and the header-vs-manifest
> identity match are checked when the binary representation is decoded
> (`src/binary_repr.rs`) — and malformed input is rejected before any merge. The
> structural IR verifier (`ir::verify_package` in `src/ir.rs`) currently enforces a
> subset of the invariants: function names are non-empty and unique, type names are
> unique, and every `MATCH` carries at least one case. The remaining invariants
> (full IR-node type-correctness, definite-assignment, linear resource ownership,
> drop/cleanup validity, return/effect agreement, and native-link manifest
> validity) are the design target and are partly enforced earlier by the
> resolver/type-checker on the project's own IR before encoding; they are not yet
> all re-checked on a decoded third-party package. The full verifier-invariant
> catalogue is owned by `./mfb spec package verifier-rules`. [[src/main.rs:read_mfp_header]] [[src/binary_repr.rs:MFPC_MAJOR_VERSION]] [[src/ir.rs:verify_package]]

A future VM is not foreclosed: it would either interpret the structured, typed Binary Representation directly or lower it through the same `IR -> NIR -> native` path. The artifact contract remains: packages are portable `.mfp` Binary Representation packages; executables are native platform binaries.

## See Also

* ./mfb spec architecture flows — end-to-end IR → NIR → native build and package decode/merge
* ./mfb spec package verifier-rules — full `.mfp` verifier-invariant catalogue
* ./mfb spec package container-format — `.mfp`/`MFPC` container byte layout
