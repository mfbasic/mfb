use super::*;

/// Assemble the package's `NATIVE_LIBRARY_TABLE` into `metadata` and render every
/// finding (plan-46-B §4.3). Returns whether the build may continue — false when
/// any finding is `Error` severity (a `LINK` with no `libraries` entry, or an
/// unreadable `vendor` file).
///
/// A package with no `LINK` block produces an empty table, emits no section 10,
/// and leaves container flag bit 0 clear — its `.mfp` stays byte-identical.
pub(super) fn assemble_native_library_table(
    manifest: &HashMap<String, JsonValue>,
    ir: &ir::IrProject,
    project_root: &Path,
) -> Option<binary_repr::NativeLibraryTable> {
    let linked = ir.link_library_names();
    let manifest_path = project_root.join("project.json");
    let (table, findings) =
        crate::manifest::libraries::build_native_library_table(manifest, &linked, project_root);

    let mut ok = true;
    for finding in &findings {
        rules::show_diagnostic(finding.rule, &finding.message, &manifest_path, 1, 1, 1);
        if rules::is_error(finding.rule) {
            ok = false;
        }
    }
    ok.then_some(table)
}

/// Every `(os, arch, libc)` this build will actually emit a binary for.
///
/// A Linux **console** `mfb build` emits both libc flavors from one invocation,
/// each its own codegen pass, so a locator that differs by libc must be resolved
/// (and its vendor file verified) once per flavor.
///
/// A Linux **app-mode** build emits a single glibc binary (plan-05-linux-app.md
/// §5.2), so it must be checked for glibc only: demanding a musl locator — and a
/// musl blob in `vendor/` — for a flavor the build never emits would fail a
/// correct project. macOS has no libc axis and yields one target either way.
///
/// This must stay in lockstep with what the backends actually emit; it is the
/// caller-side mirror of their per-flavor loop.
pub(super) fn emitted_link_targets(
    target: &target::BuildTarget,
    _build_mode: target::NativeBuildMode,
) -> Vec<link_locator::LinkTarget> {
    if target.os == "linux" {
        // plan-56-B §4.1: every Linux build — console AND app — emits both libc
        // worlds, so vendor resolution must cover both. Leaving app mode on
        // glibc-only here would put the glibc blob inside the musl AppImage.
        let flavors: &[Libc] = &[Libc::Glibc, Libc::Musl];
        flavors
            .iter()
            .map(|libc| link_locator::LinkTarget {
                os: target.os.clone(),
                arch: target.arch.clone(),
                libc: Some(*libc),
            })
            .collect()
    } else {
        vec![link_locator::LinkTarget {
            os: target.os.clone(),
            arch: target.arch.clone(),
            libc: None,
        }]
    }
}

/// Resolve every `vendor` library this build emits, in a deterministic order.
///
/// Shared by the hash verify (plan-46-C §4.4) and the output copy (plan-46-D
/// §4.5) so both act on exactly the same resolved set. Deduplicated by the
/// emitted `dlopen_name`, since both Linux flavors commonly resolve to the same
/// `system` locator and may share a `vendor` one.
pub(super) fn resolved_vendor_libraries(
    ir: &ir::IrProject,
    packages: &[PathBuf],
    target: &target::BuildTarget,
    build_mode: target::NativeBuildMode,
) -> Result<Vec<link_locator::ResolvedLibrary>, String> {
    let tables =
        link_locator::LibraryTables::collect(packages, &ir.name, ir.native_libraries.clone())?;
    // Derived from the tables, not `ir.link_library_names()`: `ir` here is the
    // project's own IR, not yet merged with its imported packages, so its
    // `link_functions` would miss every library an imported binding links — which
    // is the whole case this verify exists for.
    let linked = tables.logical_names();
    if linked.is_empty() {
        return Ok(Vec::new());
    }

    let mut vendored: Vec<link_locator::ResolvedLibrary> = Vec::new();
    for link_target in emitted_link_targets(target, build_mode) {
        let resolved = link_locator::LinkLibraries::resolve_all(&tables, &linked, &link_target)?;
        for library in resolved.vendored() {
            if !vendored
                .iter()
                .any(|existing| existing.dlopen_name == library.dlopen_name)
            {
                vendored.push(library.clone());
            }
        }
    }
    vendored.sort_by(|a, b| a.dlopen_name.cmp(&b.dlopen_name));
    Ok(vendored)
}

/// The on-disk source of a resolved vendor library (plan-48-B §4.3): the
/// consumer's own `libraries` locators read from `<project>/vendor/`, an imported
/// binding's from `<project>/packages/<declaring-unit>.vendor/` — where `pkg add`
/// placed the downloaded blob. `own_unit` is the project's own name (`ir.name`),
/// which `declaring_unit` equals exactly for a locator from the project's own
/// `libraries` section.
pub(super) fn vendor_source_path(
    project_root: &Path,
    own_unit: &str,
    library: &link_locator::ResolvedLibrary,
) -> PathBuf {
    if library.declaring_unit == own_unit {
        crate::manifest::libraries::vendor_path(project_root, &library.locator.source)
    } else {
        crate::manifest::libraries::imported_vendor_path(
            project_root,
            &library.declaring_unit,
            &library.locator.source,
        )
    }
}

/// Hash-verify each resolved `vendor` library against the sha256 the declaring
/// binding recorded in its section-10 table (plan-46-C §4.4, plan-48-B §4.3).
///
/// The consumer's own `libraries` locators read the file the author placed at
/// `<project>/vendor/<source>` by hand; an imported binding's read the blob
/// `pkg add` downloaded to `<project>/packages/<declaring-unit>.vendor/<source>`.
/// Either way the `.mfp` carries the hash, never the blob.
pub(super) fn verify_vendor_libraries(
    vendored: &[link_locator::ResolvedLibrary],
    project_root: &Path,
    own_unit: &str,
) -> bool {
    let mut ok = true;
    for library in vendored {
        let path = vendor_source_path(project_root, own_unit, library);
        let actual = match crate::manifest::libraries::sha256_file(&path) {
            Ok(hash) => hash,
            Err(reason) => {
                rules::show_general_diagnostic(
                    "NATIVE_LIBRARY_FILE_MISSING",
                    &format!(
                        "`{}` vendors native library \"{}\", but {} could not be read: {reason}. \
                         Place that file there — the package carries its hash, not its bytes.",
                        library.declaring_unit,
                        library.locator.source,
                        path.display()
                    ),
                );
                ok = false;
                continue;
            }
        };
        // A vendor locator always carries a hash (the encoder enforces
        // `hash` present iff vendor, and decode re-checks it).
        let Some(expected) = library.locator.hash else {
            rules::show_general_diagnostic(
                "NATIVE_LIBRARY_HASH_MISMATCH",
                &format!(
                    "`{}` vendors native library \"{}\" but records no hash for it; the package \
                     is malformed and must be rebuilt.",
                    library.declaring_unit, library.locator.source
                ),
            );
            ok = false;
            continue;
        };
        if actual != expected {
            rules::show_general_diagnostic(
                "NATIVE_LIBRARY_HASH_MISMATCH",
                &format!(
                    "{} does not match the sha256 `{}` recorded for \"{}\" — this is the wrong \
                     version of the library.\n               expected {}\n               actual   {}",
                    path.display(),
                    library.declaring_unit,
                    library.locator.source,
                    hex(&expected),
                    hex(&actual),
                ),
            );
            ok = false;
        }
    }
    ok
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where this build's output shape wants its vendored native libraries — the
/// directory (or directories) the emitted RPATH resolves to (plan-46-D §4.4).
///
/// Must stay in lockstep with the RPATH each backend emits: the loader looks
/// exactly here and nowhere else.
///
/// | build | rpath | vendor files |
/// | --- | --- | --- |
/// | linux console | `$ORIGIN/vendor` | `build/vendor/` |
/// | linux `--app` | `$ORIGIN/../lib` | `build/<name>.AppDir/usr/lib/` |
/// | macos console | `@loader_path/vendor` | `build/vendor/` |
/// | macos `--app` | `@executable_path/../Frameworks` | `build/<name>.app/Contents/Frameworks/` |
pub(super) fn vendor_output_dirs(
    output_dir: &Path,
    project_name: &str,
    build_mode: target::NativeBuildMode,
) -> Vec<PathBuf> {
    let build_dir = output_dir.join(crate::os::BUILD_DIR);
    match build_mode {
        // The `.app` bundle puts its dylibs in the platform-standard
        // `Contents/Frameworks/`, where Apple specifies private shared libraries
        // live and where bundle-inspecting tools expect them. Created only when the
        // build vendors something — an empty `Frameworks/` in every bundle would be
        // noise.
        target::NativeBuildMode::MacApp => vec![build_dir
            .join(format!("{project_name}.app"))
            .join("Contents")
            .join(crate::os::MACOS_APP_FRAMEWORKS_DIR)],
        // The AppDir puts its libraries at `usr/lib/`, one directory up from the
        // executable at `usr/bin/<name>` — the layout every AppDir-consuming tool
        // expects, and what `ELF_APPDIR_VENDOR_RPATH` (`$ORIGIN/../lib`) resolves
        // to. Created only when the build vendors something, so a non-vendoring
        // AppDir carries no empty `usr/lib/` (plan-51-A §4.4).
        // One `usr/lib` per flavor's AppDir (plan-56-B §4.3). Both are returned
        // so the caller can route each flavor's blob into its own image; see
        // `vendor_output_dirs_for_flavor` for the per-flavor selection.
        target::NativeBuildMode::LinuxApp => crate::os::linux::flavor::LinuxFlavor::ALL
            .iter()
            .map(|flavor| {
                build_dir
                    .join(crate::os::linux::appdir::appdir_name(
                        project_name,
                        flavor.suffix(),
                    ))
                    .join("usr")
                    .join("lib")
            })
            .collect(),
        // Both Linux libc flavors live in the one `build/` directory and share the
        // one `vendor/`. That is sound only because vendor `source` filenames are
        // unique project-wide (plan-46-A §4.3), so a glibc blob and a musl blob
        // never collide.
        target::NativeBuildMode::Console => vec![build_dir.join(crate::os::VENDOR_DIR)],
    }
}

/// The directory declared resources are copied into for a given build shape
/// (plan-55-A §4.3). Each entry's `<dst>` is joined *under* this directory.
///
/// Kept in lockstep with plan-55-B's `os::resourcePath` base offset
/// (`resource_base_offset`): the runtime locator resolves to exactly this
/// directory, so a change here without the matching change there makes resources
/// unfindable at runtime.
///
/// | build            | resource dir                                   |
/// | ---              | ---                                            |
/// | console          | `build/`                                       |
/// | macos `--app`    | `build/<name>.app/Contents/Resources/`         |
/// | linux `--app`    | `build/<name>.AppDir/usr/share/<name>/`        |
///
/// The `LinuxApp` arm depends on plan-51-A's AppDir existing; until then a Linux
/// `--app` build never reaches this path (Linux app mode is unimplemented
/// pre-51). It is written now so 51-A needs no change here.
pub(super) fn resource_output_dirs(
    output_dir: &Path,
    project_name: &str,
    build_mode: target::NativeBuildMode,
) -> Vec<PathBuf> {
    let build_dir = output_dir.join(crate::os::BUILD_DIR);
    match build_mode {
        target::NativeBuildMode::MacApp => vec![build_dir
            .join(format!("{project_name}.app"))
            .join("Contents")
            .join(crate::os::MACOS_APP_RESOURCES_DIR)],
        // Resources are flavor-independent, so BOTH AppDirs get the same copy
        // (plan-56-B §4.3). Broadcasting is correct here and wrong for vendored
        // libraries, which is why the two do not share a helper.
        target::NativeBuildMode::LinuxApp => crate::os::linux::flavor::LinuxFlavor::ALL
            .iter()
            .map(|flavor| {
                build_dir
                    .join(crate::os::linux::appdir::appdir_name(
                        project_name,
                        flavor.suffix(),
                    ))
                    .join("usr")
                    .join("share")
                    .join(project_name)
            })
            .collect(),
        target::NativeBuildMode::Console => vec![build_dir],
    }
}

/// Copy each resolved `vendor` library into the output directory the executable's
/// RPATH points at (plan-46-D §4.5), preserving the executable bit.
///
/// Runs **after** the hash verify on the same files, so the bytes landing in the
/// output are the bytes that were verified — and they are not re-hashed.
///
/// Only *resolved* locators are copied, never the whole `vendor/` directory: a
/// project vendoring blobs for six targets ships one per build.
///
/// The destination filename is `dlopen_name` — the same helper plan-46-C emits the
/// `dlopen` cstring from, never a second copy of the format string. If the file
/// written here and the string emitted there ever disagreed, the `dlopen` would
/// miss at runtime and nothing at build time would notice.
pub(super) fn copy_vendor_libraries(
    vendored: &[link_locator::ResolvedLibrary],
    project_root: &Path,
    own_unit: &str,
    output_dirs: &[PathBuf],
) -> Result<(), String> {
    if vendored.is_empty() {
        return Ok(());
    }

    // §4.5.2 residual check: the `<declaring-unit>-` prefix makes a collision
    // essentially unrepresentable, but not provably so — a project named `sqlite3`
    // that also imports a package named `sqlite3` could reach the same output name.
    // Absurd, but the cost is a silent wrong-library load, so assert it. Identical
    // hashes are fine: the same bytes, legitimately shared, and the copy is
    // idempotent. This check should never fire; it is the guard rail that lets the
    // prefix be trusted, not the mechanism.
    for (index, library) in vendored.iter().enumerate() {
        for other in &vendored[index + 1..] {
            if library.dlopen_name == other.dlopen_name
                && library.locator.hash != other.locator.hash
            {
                rules::show_general_diagnostic(
                    "NATIVE_LIBRARY_VENDOR_COLLISION",
                    &format!(
                        "`{}` and `{}` both vendor a native library that copies to \"{}\", with \
                         different contents. One would silently overwrite the other and both \
                         bindings would load whichever won. Rename one of the vendored files.",
                        library.declaring_unit, other.declaring_unit, library.dlopen_name
                    ),
                );
                return Err(format!(
                    "vendored native library name collision on \"{}\"",
                    library.dlopen_name
                ));
            }
        }
    }

    for output_dir in output_dirs {
        std::fs::create_dir_all(output_dir)
            .map_err(|err| format!("failed to create '{}': {err}", output_dir.display()))?;
        for library in vendored {
            let from = vendor_source_path(project_root, own_unit, library);
            let to = output_dir.join(&library.dlopen_name);
            std::fs::copy(&from, &to).map_err(|err| {
                format!(
                    "failed to copy vendored library '{}' to '{}': {err}",
                    from.display(),
                    to.display()
                )
            })?;
            // Preserve the executable bit: a shared object is loadable without it
            // on Linux, but macOS `dlopen` of a non-executable dylib can be
            // refused, and `fs::copy` already carries the source mode on Unix.
            // Set it explicitly so a source blob checked out without +x still works.
            let mut permissions = std::fs::metadata(&to)
                .map_err(|err| format!("failed to read '{}': {err}", to.display()))?
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&to, permissions)
                .map_err(|err| format!("failed to mark '{}' executable: {err}", to.display()))?;
        }
    }
    Ok(())
}

/// Package build: the table becomes `.mfp` section 10 and sets container flag
/// bit 0, so it rides on the metadata the package writer encodes.
pub(super) fn assemble_native_libraries(
    metadata: &mut binary_repr::BinaryReprMetadata,
    manifest: &HashMap<String, JsonValue>,
    ir: &ir::IrProject,
    project_root: &Path,
) -> bool {
    match assemble_native_library_table(manifest, ir, project_root) {
        Some(table) => {
            metadata.native_libraries = table;
            true
        }
        None => false,
    }
}

/// Executable build: nothing is encoded, but a project declaring its own `LINK`
/// block must still resolve against its own locators at codegen, so the table
/// rides on the IR into the NIR module (plan-46-C).
pub(super) fn assemble_native_libraries_for_ir(
    ir: &mut ir::IrProject,
    manifest: &HashMap<String, JsonValue>,
    project_root: &Path,
) -> bool {
    match assemble_native_library_table(manifest, ir, project_root) {
        Some(table) => {
            ir.native_libraries = table;
            true
        }
        None => false,
    }
}
