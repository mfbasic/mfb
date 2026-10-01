# End-to-End Build Flows

Executable and package builds share source preparation and diagnostics, then
select their output paths. [[src/cli/build/mod.rs:build_project]]

## Shared source preparation

1. Parse build options, set the optimization level, and validate `project.json`.
   The manifest must supply `kind`; it is not defaulted by the build.
2. Build source-directory dependencies into compiled package artifacts, then
   verify declared dependencies and their signatures/trust policy. Refuse an
   inconsistent declared dependency closure.
3. Select console or app mode from the CLI and manifest, check target support,
   and resolve any app icon.
4. Collect and parse selected source files, including the compiler prelude and
   parser-injected collection sources. Reject app-only imports in console mode
   and misplaced testing assertions.
5. Scope private names. Capture the `--ast` snapshot at this point, then drop
   `TESTING` blocks for a normal build or synthesize a test driver for `mfb test`.
6. Run initial resolution, which augments the AST and elaborates it to HIR.
   Separately augment the build AST with the selected debug mode and elaborate
   the generic HIR used by monomorphization.
7. Monomorphize to concrete HIR and resolve it again, without repeating DOC
   validation. Validate the executable entry, or select the synthesized test entry.
8. Read imported signatures, types, globals, and resource-close metadata. Run
   the HIR shape pass with the complete external signature map.
9. Lower diagnostic IR with external signatures restricted to imported resource
   producers, plus imported types/globals, and run source IR verification. Collect
   export-surface and private-scope diagnostics, render them, and stop on errors.

Both diagnostic passes complete even if shape checking finds errors; lowering
is total. Shape diagnostics precede verifier diagnostics in the combined stream.
The diagnostic IR is not reused as the final emission IR: each output path lowers
concrete HIR with the complete imported signature/type/global metadata it needs.
[[src/cli/build/mod.rs:build_project]]
[[src/cli/build/source_packages.rs:build_source_dependencies]]
[[src/cli/build/packages.rs:verify_and_report_packages]]

## Source to executable

With no output flags, an executable build lowers the concrete HIR with all
external signatures, assembles native-library metadata, and applies runtime
configuration. It clears `build/` for a normal build, resolves and verifies
vendored libraries, then dispatches to the selected native backend.
[[src/cli/build/mod.rs:build_project]]

The backend:

1. Decodes and merges declared packages into application IR, verifies the merged
   project, detects required helpers, lowers to NIR, and runs NIR optimization.
2. Validates NIR and runtime-call capabilities.
3. Produces and validates the native function/storage plan and OS object layout.
4. Generates shared instructions, runs MIR optimization, selects the target ISA,
   allocates registers, finalizes frames, and validates the native code output.
5. Applies required branch relaxation, encodes text/data/symbols/relocations,
   and writes the OS executable container or app artifact.

[[src/target/shared/lower.rs:lower_project]]
[[src/target/shared/nir/lower.rs:merge_packages]]
[[src/target/macos_aarch64/mod.rs:write_executable]]
[[src/target/linux_common/mod.rs]]
[[src/target/win_x86_64/mod.rs:write_executable]]
[[src/codegen/engine/builder/mod.rs:lower_module_for_platform]]

The CLI copies vendored libraries and declared resources, finalizes app bundles
(including Linux AppImages), and reports the output paths. Console outputs are
`build/<name>.out` on macOS, `build/<name>-glibc.out` and
`build/<name>-musl.out` on Linux, and `build/<name>.exe` on Windows. App artifacts
are specified by `./mfb spec architecture artifacts`.
[[src/cli/build/mod.rs:build_project]]

A host `mfb test` links into a temporary output directory, runs the driver, and
removes that directory; a cross-target test build emits artifacts for the target.
[[src/cli/build/mod.rs:build_project]]

## Source to package

With no output flags, a package build lowers concrete HIR using the complete
external metadata, assembles its native-library table, and derives package
metadata from the manifest. It serializes the structured IR into an MFBR section
inside the MFPC payload, validates it, and wraps that payload in an MFP container.
The container is signed when `--sign` is supplied and unsigned otherwise. The
normal output is `<project>/<name>.mfp`.
[[src/cli/build/mod.rs:build_project]]
[[src/target/package_mfp/mod.rs:build_package_bytes]]

Packages support `--ast`, `--ir`, and `--br` inspection. Native intermediate
outputs are rejected. Distinct output flags can be combined; any output flag
selects inspection output instead of the default executable/package artifact.
[[src/cli/build/options.rs:parse_build_options]]
[[src/cli/build/mod.rs:BuildOutput]]

## See Also

* ./mfb spec architecture commands — build modes and output selection
* ./mfb spec architecture frontend — source preparation and diagnostics
* ./mfb spec linker pipeline — native output stages
* ./mfb spec package binary-representation — portable package payload
