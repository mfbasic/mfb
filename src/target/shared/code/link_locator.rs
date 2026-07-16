//! Consumer-side native-library locator resolution (plan-46-C §4.1).
//!
//! At **executable** build time, for the target being emitted `(os, arch, libc)`,
//! pick the most-specific locator a binding declared for each logical library.
//! Pure: no I/O, no diagnostics — the caller renders the error and does the
//! vendor hash verify.
//!
//! This replaces `link_thunk`'s old `library_filename()` soname guess
//! (`lib{logical}.so.0` / `lib{logical}.dylib`), which never consulted the
//! manifest and missed every unversioned `.so`, `.so.3`, non-`lib`-prefixed, or
//! per-arch/libc variant.

use crate::binary_repr::{NativeLibraryLocator, NativeLibraryTable};
use crate::manifest::libraries::{LibType, Libc};

/// Why a logical library could not be resolved for a target.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResolveErr {
    /// The table carries no locator matching this target.
    NoMatch {
        logical: String,
        target: String,
        /// What the binding *did* declare, for the message.
        declared: Vec<String>,
    },
    /// Two equally-specific locators match — only reachable via genuinely
    /// duplicate entries (same `os`, same specified axes).
    Ambiguous {
        logical: String,
        target: String,
        first: String,
        second: String,
    },
    /// The imported binding carries no section-10 table for this library at all.
    NotDeclared { logical: String },
}

impl ResolveErr {
    /// The diagnostic rule this error raises.
    pub(crate) fn rule(&self) -> &'static str {
        match self {
            Self::NoMatch { .. } | Self::NotDeclared { .. } => "NATIVE_LIBRARY_NO_MATCH",
            Self::Ambiguous { .. } => "NATIVE_LIBRARY_AMBIGUOUS",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::NotDeclared { logical } => format!(
                "the binding declaring `LINK \"{logical}\"` carries no native library locators, \
                 so there is no way to know which shared object to load. Rebuild that binding \
                 with a `libraries` section in its project.json."
            ),
            Self::NoMatch {
                logical,
                target,
                declared,
            } => format!(
                "no `libraries` locator for `{logical}` matches this build's target ({target}). \
                 The binding declares: {}. Add a locator covering {target}.",
                if declared.is_empty() {
                    "nothing".to_string()
                } else {
                    declared.join("; ")
                }
            ),
            Self::Ambiguous {
                logical,
                target,
                first,
                second,
            } => format!(
                "two equally-specific `libraries` locators for `{logical}` both match this \
                 build's target ({target}): {first} and {second}. Remove the duplicate, or make \
                 one of them more specific."
            ),
        }
    }
}

/// The concrete target an executable is being emitted for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinkTarget {
    pub(crate) os: String,
    pub(crate) arch: String,
    /// The flavor being emitted. `None` on macOS, which has no libc axis — a
    /// single Linux `mfb build` emits both flavors, each its own codegen pass
    /// with its own data image, so the resolved `source` lands in the right
    /// binary for free.
    pub(crate) libc: Option<Libc>,
}

impl std::fmt::Display for LinkTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.libc {
            Some(libc) => write!(f, "{}/{}/{}", self.os, self.arch, libc.as_str()),
            None => write!(f, "{}/{}", self.os, self.arch),
        }
    }
}

/// Whether `locator` matches `target`.
///
/// `arch` and `libc` are symmetric optional wildcards — `None` means *any*
/// (plan-46-A). On macOS the libc axis is ignored; validation rejects `libc`
/// there, so it cannot be `Some`.
fn matches(locator: &NativeLibraryLocator, target: &LinkTarget) -> bool {
    if locator.os != target.os {
        return false;
    }
    if let Some(arch) = &locator.arch {
        if arch != &target.arch {
            return false;
        }
    }
    if let Some(libc) = locator.libc {
        if target.libc != Some(libc) {
            return false;
        }
    }
    true
}

/// How specific a locator is: the number of axes it pins.
///
/// A Linux `vendor` locator always scores 2 — plan-46-A §3.2 requires both axes
/// on it — so it always outranks a wildcarding `system` entry for its exact slot.
/// That makes "vendor on one slot, system everywhere else" fall out of the rule
/// with no special case.
fn specificity(locator: &NativeLibraryLocator) -> u32 {
    u32::from(locator.arch.is_some()) + u32::from(locator.libc.is_some())
}

/// Render a locator for a diagnostic message.
fn describe(locator: &NativeLibraryLocator) -> String {
    let mut axes = vec![format!("os: {}", locator.os)];
    match &locator.arch {
        Some(arch) => axes.push(format!("arch: {arch}")),
        None => axes.push("arch: any".to_string()),
    }
    if locator.os != "macos" {
        match locator.libc {
            Some(libc) => axes.push(format!("libc: {}", libc.as_str())),
            None => axes.push("libc: any".to_string()),
        }
    }
    format!(
        "{{ {}, type: {}, source: \"{}\" }}",
        axes.join(", "),
        locator.lib_type.as_str(),
        locator.source
    )
}

/// Resolve the most-specific locator for `logical` on `target` (plan-46-C §4.1).
///
/// Among matching locators the highest specificity wins; a tie is
/// [`ResolveErr::Ambiguous`] and no match is [`ResolveErr::NoMatch`]. Both are
/// hard build errors — emitting a wrong soname silently is exactly what this
/// plan exists to eliminate.
pub(crate) fn resolve<'a>(
    table: &'a NativeLibraryTable,
    logical: &str,
    target: &LinkTarget,
) -> Result<&'a NativeLibraryLocator, ResolveErr> {
    let Some(locators) = table.locators(logical) else {
        return Err(ResolveErr::NotDeclared {
            logical: logical.to_string(),
        });
    };

    let mut best: Option<&NativeLibraryLocator> = None;
    let mut tied: Option<&NativeLibraryLocator> = None;
    for locator in locators {
        if !matches(locator, target) {
            continue;
        }
        match best {
            None => best = Some(locator),
            Some(current) => match specificity(locator).cmp(&specificity(current)) {
                std::cmp::Ordering::Greater => {
                    best = Some(locator);
                    tied = None;
                }
                std::cmp::Ordering::Equal => tied = Some(locator),
                std::cmp::Ordering::Less => {}
            },
        }
    }

    match (best, tied) {
        (Some(first), Some(second)) => Err(ResolveErr::Ambiguous {
            logical: logical.to_string(),
            target: target.to_string(),
            first: describe(first),
            second: describe(second),
        }),
        (Some(locator), None) => Ok(locator),
        (None, _) => Err(ResolveErr::NoMatch {
            logical: logical.to_string(),
            target: target.to_string(),
            declared: locators.iter().map(describe).collect(),
        }),
    }
}

/// The filename a resolved locator is `dlopen`ed by (plan-46-C §3.1/§4.2).
///
/// **This is shared with plan-46-D's vendor copy step, deliberately.** The file
/// written into the output vendor directory and the string emitted into the
/// binary must be byte-identical or the `dlopen` misses — a divergence would be
/// invisible at build time and a runtime failure. Building the name in two places
/// is exactly how that happens, so there is one helper.
///
/// - a **`system`** locator emits `source` verbatim: the exact soname, which the
///   platform's dynamic loader resolves and which knows nothing of our
///   conventions;
/// - a **`vendor`** locator emits `<declaring-unit>-<source>`: vendor `source`
///   filenames are unique only *within one manifest* (plan-46-A §4.3), the output
///   flattens every vendor file into one directory, and the emitted filename *is*
///   the library's identity — so two packages each vendoring a `libfoo.so` would
///   otherwise collide and both `dlopen("libfoo.so")` would get whichever won.
pub(crate) fn dlopen_name(locator: &NativeLibraryLocator, declaring_unit: &str) -> String {
    match locator.lib_type {
        LibType::System => locator.source.clone(),
        LibType::Vendor => format!("{declaring_unit}-{}", locator.source),
    }
}

/// A logical library's resolved locator, plus the unit that declared it.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedLibrary {
    /// The `dlopen` filename to emit (and, for a vendor locator, the filename
    /// plan-46-D copies the file under). Always built by [`dlopen_name`], never
    /// independently.
    pub(crate) dlopen_name: String,
    /// The declaring unit: the imported package's name, or the project's own name
    /// for a locator from its own `libraries` section.
    pub(crate) declaring_unit: String,
    pub(crate) locator: NativeLibraryLocator,
}

/// Every native library table reachable from this build, keyed by logical name,
/// carrying the unit that declared it (plan-46-C).
pub(crate) struct LibraryTables {
    /// `(declaring unit, table)` in import order, then the project's own.
    units: Vec<(String, NativeLibraryTable)>,
}

impl LibraryTables {
    /// Collect the section-10 tables of every imported package, plus the
    /// project's own `libraries` section if it has one.
    pub(crate) fn collect(
        packages: &[std::path::PathBuf],
        own_unit: &str,
        own: NativeLibraryTable,
    ) -> Result<Self, String> {
        let mut units = Vec::new();
        for path in packages {
            let (name, table) = crate::binary_repr::read_package_native_libraries(path)?;
            if !table.is_empty() {
                units.push((name, table));
            }
        }
        if !own.is_empty() {
            units.push((own_unit.to_string(), own));
        }
        Ok(Self { units })
    }

    /// Every logical library any reachable table declares, deduplicated and
    /// sorted.
    ///
    /// This is exactly the set the build will `dlopen`, and it is derived from the
    /// tables rather than from the project's own IR: at the point the build path
    /// runs its vendor verify, the project IR has **not** yet been merged with its
    /// imported packages, so its `link_functions` carry only the project's own
    /// `LINK` blocks. The tables are already the right set — a package's section
    /// 10 lists only the libraries that package links (plan-46-B §4.3 encodes no
    /// unused entry), and the merge pulls in every declared package's link
    /// functions whether or not the consumer calls them.
    pub(crate) fn logical_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for (_, table) in &self.units {
            for entry in &table.entries {
                if !names.contains(&entry.logical) {
                    names.push(entry.logical.clone());
                }
            }
        }
        names.sort();
        names
    }

    /// Resolve `logical` for `target`, returning the locator and the unit that
    /// declared it.
    ///
    /// Searches each declaring unit's table in turn. A logical name is expected to
    /// be declared by exactly one unit (the binding that owns the `LINK` block),
    /// so the first table carrying it wins.
    pub(crate) fn resolve(
        &self,
        logical: &str,
        target: &LinkTarget,
    ) -> Result<ResolvedLibrary, ResolveErr> {
        let mut last: Option<ResolveErr> = None;
        for (unit, table) in &self.units {
            if table.locators(logical).is_none() {
                continue;
            }
            match resolve(table, logical, target) {
                Ok(locator) => {
                    return Ok(ResolvedLibrary {
                        dlopen_name: dlopen_name(locator, unit),
                        declaring_unit: unit.clone(),
                        locator: locator.clone(),
                    })
                }
                // Keep the real error (no-match / ambiguous) from the unit that
                // declares this library rather than reporting "not declared".
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or(ResolveErr::NotDeclared {
            logical: logical.to_string(),
        }))
    }
}

/// The resolved locator for every logical library this build links, keyed by
/// logical name (plan-46-C).
///
/// Built once per codegen pass — which on Linux means once per libc flavor, each
/// with its own data image — so a locator that differs per libc lands in the
/// correct binary automatically.
#[derive(Debug, Default)]
pub(crate) struct LinkLibraries {
    resolved: std::collections::HashMap<String, ResolvedLibrary>,
}

impl LinkLibraries {
    /// Resolve every logical name in `linked` against `tables` for `target`.
    ///
    /// A no-match or ambiguous locator is a **hard build error** — emitting a
    /// wrong soname and failing at runtime is exactly what plan-46 exists to
    /// eliminate. The diagnostic is rendered here and the error returned, so the
    /// build aborts before any cstring is emitted.
    pub(crate) fn resolve_all(
        tables: &LibraryTables,
        linked: &[String],
        target: &LinkTarget,
    ) -> Result<Self, String> {
        let mut resolved = std::collections::HashMap::new();
        for logical in linked {
            if resolved.contains_key(logical) {
                continue;
            }
            match tables.resolve(logical, target) {
                Ok(library) => {
                    resolved.insert(logical.clone(), library);
                }
                Err(error) => {
                    crate::rules::show_general_diagnostic(error.rule(), &error.message());
                    return Err(format!(
                        "cannot resolve native library `{logical}` for {target}"
                    ));
                }
            }
        }
        Ok(Self { resolved })
    }

    pub(crate) fn get(&self, logical: &str) -> Result<&ResolvedLibrary, String> {
        self.resolved.get(logical).ok_or_else(|| {
            format!("native library `{logical}` was not resolved before code emission")
        })
    }

    /// Every resolved `vendor` locator, for plan-46-D's copy step. `system`
    /// locators name a file the loader finds; there is nothing to copy.
    pub(crate) fn vendored(&self) -> Vec<&ResolvedLibrary> {
        let mut vendored: Vec<&ResolvedLibrary> = self
            .resolved
            .values()
            .filter(|library| library.locator.lib_type == LibType::Vendor)
            .collect();
        // Deterministic order for the copy step and its diagnostics.
        vendored.sort_by(|a, b| a.dlopen_name.cmp(&b.dlopen_name));
        vendored
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_repr::NativeLibraryEntry;

    fn locator(
        os: &str,
        arch: Option<&str>,
        libc: Option<Libc>,
        lib_type: LibType,
        source: &str,
    ) -> NativeLibraryLocator {
        NativeLibraryLocator {
            os: os.to_string(),
            arch: arch.map(str::to_string),
            libc,
            lib_type,
            source: source.to_string(),
            hash: match lib_type {
                LibType::Vendor => Some([1u8; 32]),
                LibType::System => None,
            },
        }
    }

    fn table(locators: Vec<NativeLibraryLocator>) -> NativeLibraryTable {
        NativeLibraryTable {
            entries: vec![NativeLibraryEntry {
                logical: "sqlite3".to_string(),
                locators,
            }],
        }
    }

    fn linux(arch: &str, libc: Libc) -> LinkTarget {
        LinkTarget {
            os: "linux".to_string(),
            arch: arch.to_string(),
            libc: Some(libc),
        }
    }

    fn macos() -> LinkTarget {
        LinkTarget {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
            libc: None,
        }
    }

    /// The plan-46-A §1 manifest: a wildcard Linux system entry plus one concrete
    /// riscv64/musl vendor entry.
    fn worked_example() -> NativeLibraryTable {
        table(vec![
            locator("macos", None, None, LibType::System, "libsqlite3.dylib"),
            locator("linux", None, None, LibType::System, "libsqlite3.so.0"),
            locator(
                "linux",
                Some("riscv64"),
                Some(Libc::Musl),
                LibType::Vendor,
                "libsqlite3-riscv64-musl.so",
            ),
        ])
    }

    #[test]
    fn a_one_line_system_entry_covers_all_of_linux() {
        let table = table(vec![locator(
            "linux",
            None,
            None,
            LibType::System,
            "libsqlite3.so.0",
        )]);
        for arch in ["aarch64", "x86_64", "riscv64"] {
            for libc in [Libc::Glibc, Libc::Musl] {
                let resolved = resolve(&table, "sqlite3", &linux(arch, libc))
                    .unwrap_or_else(|e| panic!("{arch}/{libc:?}: {}", e.message()));
                assert_eq!(resolved.source, "libsqlite3.so.0");
            }
        }
    }

    #[test]
    fn a_concrete_vendor_entry_outranks_a_wildcard_system_entry_for_its_slot() {
        // The §4.1 worked example: building linux/riscv64/musl, the vendor entry
        // wins on specificity (2 axes vs 0).
        let table = worked_example();
        let resolved = resolve(&table, "sqlite3", &linux("riscv64", Libc::Musl)).expect("resolves");
        assert_eq!(resolved.source, "libsqlite3-riscv64-musl.so");
        assert_eq!(resolved.lib_type, LibType::Vendor);
    }

    #[test]
    fn every_other_linux_slot_still_gets_the_system_soname() {
        // "vendor on one slot, system everywhere else" with no special case.
        let table = worked_example();
        for (arch, libc) in [
            ("x86_64", Libc::Glibc),
            ("x86_64", Libc::Musl),
            ("aarch64", Libc::Glibc),
            ("aarch64", Libc::Musl),
            ("riscv64", Libc::Glibc),
        ] {
            let resolved = resolve(&table, "sqlite3", &linux(arch, libc)).expect("resolves");
            assert_eq!(
                resolved.source, "libsqlite3.so.0",
                "{arch}/{libc:?} should take the system soname"
            );
        }
    }

    #[test]
    fn macos_resolves_its_own_entry_and_ignores_the_libc_axis() {
        let table = worked_example();
        let resolved = resolve(&table, "sqlite3", &macos()).expect("resolves");
        assert_eq!(resolved.source, "libsqlite3.dylib");
    }

    #[test]
    fn a_wildcard_arch_locator_pinned_to_one_libc_matches_only_that_libc() {
        let table = table(vec![locator(
            "linux",
            None,
            Some(Libc::Musl),
            LibType::System,
            "libsqlite3-musl.so",
        )]);
        assert!(resolve(&table, "sqlite3", &linux("x86_64", Libc::Musl)).is_ok());
        let error =
            resolve(&table, "sqlite3", &linux("x86_64", Libc::Glibc)).expect_err("no match");
        assert_eq!(error.rule(), "NATIVE_LIBRARY_NO_MATCH");
    }

    #[test]
    fn a_pinned_arch_locator_matches_only_that_arch() {
        let table = table(vec![locator(
            "linux",
            Some("x86_64"),
            None,
            LibType::System,
            "libsqlite3-x86.so",
        )]);
        assert!(resolve(&table, "sqlite3", &linux("x86_64", Libc::Glibc)).is_ok());
        assert!(resolve(&table, "sqlite3", &linux("aarch64", Libc::Glibc)).is_err());
    }

    #[test]
    fn no_matching_locator_is_a_no_match_error_listing_what_was_declared() {
        // A macOS-only binding built for Linux.
        let table = table(vec![locator(
            "macos",
            None,
            None,
            LibType::System,
            "libsqlite3.dylib",
        )]);
        let error =
            resolve(&table, "sqlite3", &linux("x86_64", Libc::Glibc)).expect_err("no match");
        assert_eq!(error.rule(), "NATIVE_LIBRARY_NO_MATCH");
        let message = error.message();
        assert!(message.contains("linux/x86_64/glibc"), "message: {message}");
        // The message must show what the binding *did* declare, or the author
        // cannot tell what to add.
        assert!(message.contains("libsqlite3.dylib"), "message: {message}");
    }

    #[test]
    fn two_equally_specific_locators_are_ambiguous() {
        let table = table(vec![
            locator(
                "linux",
                Some("x86_64"),
                Some(Libc::Glibc),
                LibType::System,
                "a.so",
            ),
            locator(
                "linux",
                Some("x86_64"),
                Some(Libc::Glibc),
                LibType::System,
                "b.so",
            ),
        ]);
        let error =
            resolve(&table, "sqlite3", &linux("x86_64", Libc::Glibc)).expect_err("ambiguous");
        assert_eq!(error.rule(), "NATIVE_LIBRARY_AMBIGUOUS");
        let message = error.message();
        assert!(
            message.contains("a.so") && message.contains("b.so"),
            "message: {message}"
        );
    }

    #[test]
    fn duplicate_wildcards_are_ambiguous_too() {
        let table = table(vec![
            locator("linux", None, None, LibType::System, "a.so"),
            locator("linux", None, None, LibType::System, "b.so"),
        ]);
        assert_eq!(
            resolve(&table, "sqlite3", &linux("x86_64", Libc::Glibc))
                .expect_err("ambiguous")
                .rule(),
            "NATIVE_LIBRARY_AMBIGUOUS"
        );
    }

    #[test]
    fn a_library_absent_from_the_table_is_not_declared() {
        let table = worked_example();
        let error = resolve(&table, "zlib", &macos()).expect_err("not declared");
        assert_eq!(error.rule(), "NATIVE_LIBRARY_NO_MATCH");
        assert!(error.message().contains("no native library locators"));
    }

    #[test]
    fn a_more_specific_locator_wins_regardless_of_declaration_order() {
        // The specific entry declared *first*, the wildcard second.
        let table = table(vec![
            locator(
                "linux",
                Some("riscv64"),
                Some(Libc::Musl),
                LibType::Vendor,
                "specific.so",
            ),
            locator("linux", None, None, LibType::System, "wildcard.so"),
        ]);
        let resolved = resolve(&table, "sqlite3", &linux("riscv64", Libc::Musl)).expect("resolves");
        assert_eq!(resolved.source, "specific.so");
    }

    // ---- dlopen_name (§3.1) ----

    #[test]
    fn a_system_locator_emits_its_soname_verbatim() {
        let locator = locator("linux", None, None, LibType::System, "libsqlite3.so.0");
        assert_eq!(dlopen_name(&locator, "sqlite3"), "libsqlite3.so.0");
    }

    #[test]
    fn a_vendor_locator_emits_the_declaring_unit_prefixed_name() {
        // Must match plan-46-D §4.5's copied filename exactly.
        let locator = locator(
            "linux",
            Some("x86_64"),
            Some(Libc::Glibc),
            LibType::Vendor,
            "libfoo.so",
        );
        assert_eq!(dlopen_name(&locator, "imaging"), "imaging-libfoo.so");
        assert_eq!(dlopen_name(&locator, "myapp"), "myapp-libfoo.so");
    }

    #[test]
    fn two_packages_vendoring_the_same_filename_get_distinct_names() {
        // The collision this prefix exists to prevent: without it both bindings
        // would dlopen("libfoo.so") and get whichever file won the copy.
        let locator = locator(
            "linux",
            Some("x86_64"),
            Some(Libc::Glibc),
            LibType::Vendor,
            "libfoo.so",
        );
        assert_ne!(
            dlopen_name(&locator, "sqlite3"),
            dlopen_name(&locator, "imaging")
        );
    }
}
