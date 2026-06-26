# 21. Build Artifacts

MFBASIC uses source files for authoring, portable binary representation packages, and native binaries for executables.

| Artifact | Extension | Purpose |
|----------|-----------|---------|
| Source file | `.mfb` | Human-authored source code. The `.mfb` files selected by a project's `project.json` together form that project's source package (§13). |
| Package | `.mfp` | Architecture-neutral binary representation package. Its payload is **structured Binary Representation** (a faithful, versioned serialization of the compiler's IR) plus an embedded package manifest, public API metadata, dependency metadata, and optional native-link metadata. A compiled package can be built on one platform and imported on any platform that supports the same MFB binary representation/package version. |
| Executable | platform-native | Final application binary for the target OS/CPU. Executables compile application code plus imported `.mfp` packages to native code. |

The backend pipeline is:

```text
.mfb source
  -> IR (typed, structured program representation)
  -> Binary Representation (.mfp package)
       or
  -> NIR -> native executable
```

A package build stops at Binary Representation: `IR -> Binary Representation (.mfp)`, a faithful structured serialization with no flattening or structure loss. An executable build lowers the project's own IR through `IR -> NIR -> native`. Consuming a package **decodes** its Binary Representation back into IR functions, merges them into the project IR, and lowers everything through that same single `IR -> NIR -> native` path — there is no separate package binary representation-to-native bridge.

Package compilation emits `.mfp` packages containing portable Binary Representation plus the embedded package manifest, dependency metadata, native-link metadata, and public API metadata needed for import, type checking, IR merging, and verification. This metadata includes each exported type and function's ownership properties: copyability, movability, resource-handle status, closure-capture requirements, thread-sendability, drop requirements, and collection element constraints. A package containing `LINK` declarations emits a reusable native binding `.mfp`: importers consume the package API and do not repeat the `LINK` declarations.

Executable compilation consumes `.mfb` application source, the resolved `mfb.lock`, and imported `.mfp` packages. The compiler decodes each imported package's Binary Representation and merges its IR functions into the project IR under the package's identity prefix, resolving package-qualified MFBASIC calls to functions in the merged IR. After the IR merge, the native backend lowers everything through `IR -> NIR -> native`, resolves all native dependencies declared by the merged packages, performs target OS/native linking as needed, and emits a native binary for the selected target platform.

## 21.1 `.mfp` Binary Representation verification

Every `.mfp` package is verified before its Binary Representation can be decoded, merged into the project IR, or lowered. Verification operates on the **decoded IR**, not a flat opcode stream, and is deterministic: it must reject malformed packages before any package code runs. Because the Binary Representation is structured (nested regions with explicit ends), structure is explicit — there is no control-flow graph to reconstruct and no "jump into a trap or cleanup region" to reject — and most invariants reuse the compiler's existing IR-level passes.

The verifier must check:

- Package metadata is well-formed, uses a supported binary representation/package (Binary Representation) version, satisfies the resolved manifest and lockfile entries, and matches the Binary Representation body.
- The package signature, hash, or trust record is valid when the build mode requires signed or locked dependencies.
- Public API metadata is consistent with the IR definitions, including exported names, type shapes, function signatures, ownership properties, and native-link declarations.
- Every IR node is type-correct. Operand types, result types, call signatures, record fields, union member types, collection element types, and `Result` handling (`CallResult`, `ResultIsOk`/`ResultValue`/`ResultError`) must match the typed metadata.
- Every binding, local, and result value is definitely defined before read and is not read after move.
- Resource ownership is linear. A resource handle has one owner, is not copied, is not stored in ordinary collections, is sent to threads only when its concrete type is thread-sendable, and is closed or moved exactly once on every control-flow path.
- Drop and cleanup paths are valid. The verifier rejects double-drop, missing-drop, and use-after-drop paths. Because resource regions are nested in the IR, every exit path is bounded by the region's end.
- Every `MATCH` is exhaustive (covers every value or has an `ELSE`).
- All normal and error paths satisfy the function's declared return type and `Result` behavior (declared return/effect agreement).
- There is at most one function-level bottom `TRAP`; error routing uses the structured `Result`/`TRAP`/`FAIL`/`PROPAGATE` form, never exception-like unwinding.
- Native-link manifests are valid: every linked library and symbol referenced by the IR is declared in metadata, every resource close function exists and has the correct resource-consuming signature, and every ABI mapping uses supported native types.

Verification failure rejects the package with a toolchain diagnostic. It is not recoverable by program `TRAP` code because no package code has started running.

A future VM is not foreclosed: it would either interpret the structured, typed Binary Representation directly or lower it through the same `IR -> NIR -> native` path. The artifact contract remains: packages are portable `.mfp` Binary Representation packages; executables are native platform binaries.
