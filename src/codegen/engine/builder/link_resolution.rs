//! plan-176-U: LINK resolution — the closer symbol, the link libraries and deferred relocation
//! libraries. The builder types and shared imports live in `engine/builder/mod.rs`.

use super::*;

/// The thunk symbol a resource's registered `CLOSE BY` op resolves to, or `None`
/// when the name routes to nothing in this module.
///
/// bug-377: two spellings reach here and both must resolve, or a resource is
/// silently never closed.
///
/// * `<id>.<package>.<alias>.<op>` — a `RESOURCE T CLOSE BY link::op` whose
///   internal dotted target `merge_packages` identity-prefixed with the link
///   function it names.
/// * `<package>.<alias-name>` — a re-exported `EXPORT FUNC close AS link::op`.
///   `ir::package::merge_package` qualifies the bare alias with the package name
///   but does NOT identity-prefix it, so the identity-prefixed spelling that
///   `code::validation` built for it misses.
///
/// Try the name as given, then again with a leading identity segment stripped.
/// The identity is a 16-hex-digit content hash, which no package or alias name
/// can be, so the retry cannot capture a legitimately-dotted first segment.
pub(crate) fn resolve_closer_symbol(
    close: &str,
    function_symbols: &HashMap<String, String>,
) -> Option<String> {
    if let Some(symbol) = function_symbols.get(close) {
        return Some(symbol.clone());
    }
    let (identity, rest) = close.split_once('.')?;
    if identity.len() != 16 || !identity.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    function_symbols.get(rest).cloned()
}

/// Resolve every logical `LINK` library this module names to the concrete
/// `source` the declaring binding declared for this build's `(os, arch, libc)`
/// (plan-46-C §4.2).
///
/// Locators come from two places: an imported binding's `.mfp` section 10 (read
/// here, since resolution is per-target and so cannot happen upstream of this
/// per-flavor pass), and the project's own `libraries` section for a project
/// declaring its own `LINK` block. A no-match or ambiguity is a hard build error.
pub(crate) fn resolve_link_libraries(
    module: &nir::NirModule,
    packages: &[PathBuf],
    platform: &dyn CodegenPlatform,
) -> Result<link_locator::LinkLibraries, String> {
    if module.link_functions.is_empty() {
        return Ok(link_locator::LinkLibraries::default());
    }

    let tables = link_locator::LibraryTables::collect(
        packages,
        &module.project,
        module.native_libraries.clone(),
    )?;

    // `platform.target()` is `<os>-<arch>`; the libc comes from the platform,
    // which on Linux is one per flavor (§4.3).
    let target = platform.target();
    let (os, arch) = target.split_once('-').ok_or_else(|| {
        format!("codegen platform target '{target}' is not in `<os>-<arch>` form")
    })?;
    let target = link_locator::LinkTarget {
        os: os.to_string(),
        arch: arch.to_string(),
        libc: platform.libc(),
    };

    let mut linked: Vec<String> = Vec::new();
    for function in &module.link_functions {
        if !linked.contains(&function.library) {
            linked.push(function.library.clone());
        }
    }
    link_locator::LinkLibraries::resolve_all(&tables, &linked, &target)
}

/// Fill in every relocation whose `library` is the deferred placeholder (an
/// empty string) from `platform_imports`, and reject a symbol the platform never
/// declared.
///
/// A backend may leave `library` empty when the correct value depends on
/// something the emitter does not know. The Linux/GTK app-mode emitters do this
/// because the C-library soname depends on the libc flavor, and threading that
/// through ~30 emitter signatures and 33 builder construction sites would
/// duplicate a mapping the native plan already owns (plan-56-A §4.2).
/// Relocations that already carry a library, or carry `None`, are left exactly
/// as they are, so no other backend is affected.
///
/// The `library` field is cosmetic — the linker binds by symbol name — but a
/// wrong or absent one makes artifact debugging lie, and on musl it is the
/// *only* place the libc flavor is observable at all: musl's loader absorbs the
/// glibc compat sonames, so the program runs either way.
pub(crate) fn bind_deferred_relocation_libraries(
    functions: &mut [CodeFunction],
    platform_imports: &HashMap<String, String>,
) -> Result<(), String> {
    for function in functions.iter_mut() {
        for relocation in function.relocations.iter_mut() {
            if relocation.library.as_deref() != Some("") {
                continue;
            }
            match platform_imports.get(&relocation.to) {
                Some(library) => relocation.library = Some(library.clone()),
                None => {
                    return Err(format!(
                        "codegen calls '{}' from '{}', which the platform import list \
                         does not declare",
                        relocation.to, relocation.from
                    ))
                }
            }
        }
    }
    Ok(())
}
