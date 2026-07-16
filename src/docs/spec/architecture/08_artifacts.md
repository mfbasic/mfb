# Build Artifacts

MFBASIC uses three artifact kinds: source files for authoring, portable binary
representation packages, and native binaries for executables.

| Artifact | Extension | Purpose |
|----------|-----------|---------|
| Source file | `.mfb` | Human-authored source code. The `.mfb` files selected by a project's `project.json` together form that project's source package (see `./mfb spec language modules-and-packages`). |
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

Package compilation emits `.mfp` packages containing portable Binary Representation
plus the embedded package manifest, dependency metadata, native-link metadata, and
public API metadata needed for import, source-syntax checking, IR merging, and
semantic verification. This metadata includes each exported type and function's
ownership properties: copyability, movability, resource-handle status,
closure-capture requirements, thread-sendability, drop requirements, and collection
element constraints. A package containing `LINK` declarations emits a reusable
native binding `.mfp`: importers consume the package API and do not repeat the
`LINK` declarations.

## Debug-dump artifacts

Every intermediate stage can be dumped from the build with a flag. Each row is the
artifact, the flag that produces it, and the pipeline stage it captures.[[src/cli/build.rs:BuildOutput]]

| Artifact | Command | Contents |
| --- | --- | --- |
| `<name>.ast` | `mfb build --ast` | Parsed source tree, before monomorphization. |
| `<name>.ir` | `mfb build --ir` | Typed, architecture-independent compiler IR. |
| `<name>.hex` | `mfb build --br` | Hex dump of the MFPC binary representation. |
| `<name>.mir` | `mfb build --mir` | Target-neutral MIR dump. |
| `<name>.nir` | `mfb build --nir` | Native IR for the selected target. |
| `<name>.nplan` | `mfb build --nplan` | Native function/storage/call plan. |
| `<name>.nobj` | `mfb build --nobj` | OS object/container layout plan. |
| `<name>.ncode` | `mfb build --ncode` | Target code-generation plan. |
| `<name>.out` | `mfb build` executable (macOS) | Native executable (Mach-O). |
| `<name>-glibc.out` | `mfb build` executable (Linux) | Native executable (ELF, glibc). |
| `<name>-musl.out` | `mfb build` executable (Linux) | Native executable (ELF, musl). |
| `<name>.mfp` | `mfb build` package | Compiled MFB package. |

## `.mfp` verification

Every `.mfp` package is verified before its Binary Representation can be decoded,
merged into the project IR, or lowered. Verification operates on the **decoded IR**,
not a flat opcode stream, and is deterministic: it must reject malformed packages
before any package code runs. Because the Binary Representation is structured
(nested regions with explicit ends), structure is explicit — there is no
control-flow graph to reconstruct and no "jump into a trap or cleanup region" to
reject — and most invariants reuse the compiler's existing IR-level passes.

The decoder verifies the container before decoding: the outer `MFP` package magic
and container major version, then the inner `MFPC` payload magic, MFPC major
version, and header-vs-manifest identity match — malformed input is rejected before
any merge. After the package IR is merged into the application IR, the same complete
semantic checker used for the project's own source-lowered IR runs over the merged
IR before native lowering, re-checking type-correctness, definite-assignment, linear
resource ownership, drop/cleanup validity, `MATCH`-exhaustiveness, return/effect
agreement, single-bottom-`TRAP`, and native-link-manifest validity. Verification
failure rejects the package with a toolchain diagnostic; it is not recoverable by
program `TRAP` code because no package code has started running. The complete
invariant catalogue and which invariants are re-checked at import time are owned by
`./mfb spec package verifier-rules`. [[src/manifest/package.rs:read_mfp_header]] [[src/binary_repr/mod.rs:MFPC_MAJOR_VERSION]] [[src/ir/binary.rs:verify_package]]

A future VM is not foreclosed: it would either interpret the structured, typed
Binary Representation directly or lower it through the same `IR -> NIR -> native`
path. The artifact contract remains: packages are portable `.mfp` Binary
Representation packages; executables are native platform binaries.

## See Also

* ./mfb spec architecture flows — end-to-end IR → NIR → native build and package decode/merge
* ./mfb spec package verifier-rules — full `.mfp` verifier-invariant catalogue
* ./mfb spec package container-format — `.mfp`/`MFPC` container byte layout
* ./mfb spec architecture commands — the build flags that select each dump
