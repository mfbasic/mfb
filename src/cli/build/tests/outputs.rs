//! plan-176-K: `cli::build::tests` — the outputs tests. The shared project builders live in\n//! `build/tests/mod.rs`.

use super::*;

fn vendor_locator(source: &str) -> NativeLibraryLocator {
    NativeLibraryLocator {
        os: "linux".to_string(),
        arch: Some("x86_64".to_string()),
        libc: Some(Libc::Glibc),
        lib_type: LibType::Vendor,
        source: source.to_string(),
        hash: Some([1u8; 32]),
    }
}

fn resolved(unit: &str, source: &str) -> crate::codegen::link::locator::ResolvedLibrary {
    let locator = vendor_locator(source);
    crate::codegen::link::locator::ResolvedLibrary {
        dlopen_name: crate::codegen::link::locator::dlopen_name(&locator, unit),
        declaring_unit: unit.to_string(),
        locator,
    }
}

/// The consumer project's own name in these tests. Every `resolved(unit, …)`
/// below uses a `unit` different from this, so the library reads from the
/// imported-package location `packages/<unit>.vendor/` (plan-48-B §4.3).
const OWN_UNIT: &str = "app";

/// Write a resolved library's source bytes where `vendor_source_path` will
/// look for them given `OWN_UNIT` — the imported `packages/<unit>.vendor/`
/// directory for a unit other than `OWN_UNIT`.
fn write_vendor_source(
    root: &Path,
    library: &crate::codegen::link::locator::ResolvedLibrary,
    bytes: &[u8],
) {
    let path = vendor_source_path(root, OWN_UNIT, library);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
}

/// The file written and the string emitted must be the SAME string: a
/// divergence is a `dlopen` miss at runtime and invisible at build time. Both
/// sides build it through `dlopen_name`, so pin that they agree.
#[test]
fn the_copied_filename_is_the_emitted_dlopen_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let library = resolved("sqlite3", "libfoo.so");
    write_vendor_source(root, &library, b"bytes");
    let out = root.join("out");
    copy_vendor_libraries(
        std::slice::from_ref(&library),
        root,
        OWN_UNIT,
        std::slice::from_ref(&out),
    )
    .expect("copy succeeds");

    // The file on disk is named exactly what plan-46-C emits into the binary.
    assert!(out.join(&library.dlopen_name).is_file());
    assert_eq!(library.dlopen_name, "sqlite3-libfoo.so");
}

/// An imported binding's vendor file is read from its per-package
/// `packages/<unit>.vendor/` directory, never the consumer's own `vendor/`
/// (plan-48-B §4.3).
#[test]
fn imported_vendor_file_is_read_from_the_per_package_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let library = resolved("sqlite3", "libfoo.so");
    // Placed in the imported location; a same-named file in the consumer's own
    // `vendor/` must NOT be picked up in its place.
    std::fs::create_dir_all(root.join("vendor")).unwrap();
    std::fs::write(
        root.join("vendor").join("libfoo.so"),
        b"WRONG own-vendor bytes",
    )
    .unwrap();
    write_vendor_source(root, &library, b"right imported bytes");

    let out = root.join("out");
    copy_vendor_libraries(
        std::slice::from_ref(&library),
        root,
        OWN_UNIT,
        std::slice::from_ref(&out),
    )
    .expect("copy succeeds");
    assert_eq!(
        std::fs::read(out.join(&library.dlopen_name)).unwrap(),
        b"right imported bytes"
    );
}

/// The collision this prefix exists to prevent: two packages each vendoring a
/// `libfoo.so`. Both must land as distinct files — without the prefix one
/// would silently overwrite the other and both bindings would `dlopen`
/// whichever won.
#[test]
fn two_packages_vendoring_the_same_filename_land_as_two_distinct_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let a = resolved("sqlite3", "libfoo.so");
    let b = resolved("imaging", "libfoo.so");
    write_vendor_source(root, &a, b"bytes");
    write_vendor_source(root, &b, b"bytes");
    assert_ne!(a.dlopen_name, b.dlopen_name);

    let out = root.join("out");
    copy_vendor_libraries(
        &[a.clone(), b.clone()],
        root,
        OWN_UNIT,
        std::slice::from_ref(&out),
    )
    .expect("copy");
    assert!(out.join("sqlite3-libfoo.so").is_file());
    assert!(out.join("imaging-libfoo.so").is_file());
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2);
}

/// §4.5.2 residual check: two declaring units mapping to the same output name
/// with *different* bytes. This should never fire — it is the guard rail that
/// lets the prefix be trusted, not the mechanism.
#[test]
fn colliding_output_names_with_differing_hashes_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let mut a = resolved("same", "libfoo.so");
    let mut b = resolved("same", "libfoo.so");
    a.locator.hash = Some([1u8; 32]);
    b.locator.hash = Some([2u8; 32]);
    write_vendor_source(root, &a, b"bytes");
    let error = copy_vendor_libraries(&[a, b], root, OWN_UNIT, &[root.join("out")])
        .expect_err("differing hashes on one output name must be rejected");
    assert!(error.contains("collision"), "error: {error}");
}

/// Identical hashes are fine: the same bytes, legitimately shared, and the
/// copy is idempotent.
#[test]
fn colliding_output_names_with_identical_hashes_are_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let a = resolved("same", "libfoo.so");
    let b = resolved("same", "libfoo.so");
    write_vendor_source(root, &a, b"bytes");
    copy_vendor_libraries(&[a, b], root, OWN_UNIT, &[root.join("out")])
        .expect("identical bytes may share an output name");
}

/// A build with no vendor locators writes no vendor directory at all.
#[test]
fn no_vendor_locators_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    copy_vendor_libraries(&[], dir.path(), OWN_UNIT, std::slice::from_ref(&out)).expect("no-op");
    assert!(
        !out.exists(),
        "an empty vendor set must not create the directory"
    );
}

/// The RPATH each backend emits and the directory the copy targets must agree
/// — the loader looks exactly there and nowhere else.
#[test]
fn vendor_output_dirs_match_the_emitted_rpath_per_shape() {
    let root = Path::new("/proj");
    assert_eq!(
        vendor_output_dirs(root, "app", target::NativeBuildMode::Console),
        vec![PathBuf::from("/proj/build/vendor")],
        "console: $ORIGIN/vendor | @loader_path/vendor -> build/vendor"
    );
    assert_eq!(
        vendor_output_dirs(root, "app", target::NativeBuildMode::LinuxApp),
        vec![
            PathBuf::from("/proj/build/app-glibc.AppDir/usr/lib"),
            PathBuf::from("/proj/build/app-musl.AppDir/usr/lib"),
        ],
        "linux --app: $ORIGIN/../lib -> each flavor's AppDir usr/lib"
    );
    // The two Linux shapes must differ: the console `.out` and the AppDir's
    // `usr/bin/<name>` sit at different depths, so a shared directory would
    // mean one of the two RUNPATHs points at nothing.
    assert_ne!(
        vendor_output_dirs(root, "app", target::NativeBuildMode::Console),
        vendor_output_dirs(root, "app", target::NativeBuildMode::LinuxApp),
    );
    assert_eq!(
        vendor_output_dirs(root, "app", target::NativeBuildMode::MacApp),
        vec![PathBuf::from("/proj/build/app.app/Contents/Frameworks")],
        "macos -app: @executable_path/../Frameworks -> the bundle's Frameworks"
    );
}

/// plan-55-A §4.3: the resource directory each build shape writes into, kept
/// in lockstep with plan-55-B's `resource_base_offset`.
#[test]
fn resource_output_dir_per_build_shape() {
    let root = Path::new("/proj");
    assert_eq!(
        resource_output_dirs(root, "app", target::NativeBuildMode::Console),
        vec![PathBuf::from("/proj/build")],
        "console: resources beside the executable in build/"
    );
    assert_eq!(
        resource_output_dirs(root, "app", target::NativeBuildMode::MacApp),
        vec![PathBuf::from("/proj/build/app.app/Contents/Resources")],
        "macos -app: the bundle's Contents/Resources"
    );
    assert_eq!(
        resource_output_dirs(root, "app", target::NativeBuildMode::LinuxApp),
        vec![
            PathBuf::from("/proj/build/app-glibc.AppDir/usr/share/app"),
            PathBuf::from("/proj/build/app-musl.AppDir/usr/share/app"),
        ],
        "linux --app: usr/share/<name> inside BOTH flavors' AppDirs"
    );
}

#[test]
fn resource_src_fixed_prefix_splits_at_first_glob() {
    assert_eq!(resource_src_fixed_prefix("data/**/*.ogg"), "data");
    assert_eq!(resource_src_fixed_prefix("data/*.ogg"), "data");
    assert_eq!(resource_src_fixed_prefix("assets/logo.png"), "assets");
    assert_eq!(resource_src_fixed_prefix("*.ogg"), "");
    assert_eq!(resource_src_fixed_prefix("logo.png"), "");
    assert_eq!(resource_src_fixed_prefix("a/b/c/*.txt"), "a/b/c");
}

/// bug-298 defense in depth: manifest validation rejects an escaping `src`
/// textually, but `copy_resources` is the step that actually reads files, and
/// a symlink *inside* the project pointing outside it passes every textual
/// check. Canonicalized containment is what catches that.
#[test]
#[cfg(unix)]
fn copy_resources_refuses_a_source_that_resolves_outside_the_project() {
    let project = tempfile::tempdir().expect("project dir");
    let outside = tempfile::tempdir().expect("outside dir");
    std::fs::write(outside.path().join("secret.conf"), b"secret").unwrap();
    // An in-tree name that textually looks contained, but resolves out.
    std::os::unix::fs::symlink(outside.path(), project.path().join("assets")).unwrap();

    let out = project.path().join("build");
    std::fs::create_dir_all(&out).unwrap();
    let entries = vec![crate::manifest::ResourceEntry {
        src: "assets/*.conf".to_string(),
        dst: "cfg/".to_string(),
    }];
    let err = copy_resources(project.path(), &entries, &out)
        .expect_err("a source resolving outside the project must be refused");
    assert!(
        err.contains("outside the project root"),
        "unexpected error: {err}"
    );
    // Nothing was copied.
    assert!(!out.join("cfg/secret.conf").exists());
}

/// plan-55-A §4.3: the three worked examples — flat glob, `**` subtree
/// preservation, and a single literal file — plus the empty-match no-op.
#[test]
fn copy_resources_maps_the_worked_examples() {
    let project = tempfile::tempdir().expect("project dir");
    let root = project.path();
    // data/Mozart1.ogg, data/loops/kick.ogg, assets/logo.png.
    std::fs::create_dir_all(root.join("data/loops")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("data/Mozart1.ogg"), b"a").unwrap();
    std::fs::write(root.join("data/loops/kick.ogg"), b"b").unwrap();
    std::fs::write(root.join("assets/logo.png"), b"c").unwrap();

    let out = tempfile::tempdir().expect("out dir");
    let resource_dir = out.path();
    let entries = vec![
        crate::manifest::ResourceEntry {
            src: "data/*.ogg".to_string(),
            dst: "music/".to_string(),
        },
        crate::manifest::ResourceEntry {
            src: "data/**/*.ogg".to_string(),
            dst: "all/".to_string(),
        },
        crate::manifest::ResourceEntry {
            src: "assets/logo.png".to_string(),
            dst: "img/".to_string(),
        },
        // Matches nothing — must be a silent no-op.
        crate::manifest::ResourceEntry {
            src: "nowhere/*.dat".to_string(),
            dst: "x/".to_string(),
        },
    ];
    copy_resources(root, &entries, resource_dir).expect("copy");

    // data/*.ogg -> music/ : only the top-level file, not the subtree one.
    assert!(resource_dir.join("music/Mozart1.ogg").is_file());
    assert!(!resource_dir.join("music/loops").exists());
    // data/**/*.ogg -> all/ : subtree structure preserved below the prefix.
    assert!(resource_dir.join("all/Mozart1.ogg").is_file());
    assert!(resource_dir.join("all/loops/kick.ogg").is_file());
    // assets/logo.png -> img/logo.png.
    assert!(resource_dir.join("img/logo.png").is_file());
    assert_eq!(
        std::fs::read(resource_dir.join("img/logo.png")).unwrap(),
        b"c"
    );
    // The empty-match entry created nothing.
    assert!(!resource_dir.join("x").exists());
}

/// A Linux **console** build emits both libc flavors, so both must be checked;
/// a Linux **app** build emits a single glibc binary, so demanding a musl
/// locator (and a musl blob in `vendor/`) for a flavor it never emits would
/// fail a correct project.
#[test]
fn emitted_link_targets_track_what_each_build_mode_actually_emits() {
    let linux = target::BuildTarget {
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
    };
    let console: Vec<String> = emitted_link_targets(&linux, target::NativeBuildMode::Console)
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert_eq!(console, vec!["linux/x86_64/glibc", "linux/x86_64/musl"]);

    // plan-56-B §4.1: app mode is no longer glibc-only — it emits one
    // AppImage per libc, so vendor resolution must cover both. Resolving
    // only glibc here would put the glibc blob inside the musl image.
    let app: Vec<String> = emitted_link_targets(&linux, target::NativeBuildMode::LinuxApp)
        .iter()
        .map(|t| t.to_string())
        .collect();
    assert_eq!(
        app, console,
        "app mode resolves the same libc set as console"
    );

    // macOS has no libc axis in either mode.
    let macos = target::BuildTarget {
        os: "macos".to_string(),
        arch: "aarch64".to_string(),
    };
    for mode in [
        target::NativeBuildMode::Console,
        target::NativeBuildMode::MacApp,
    ] {
        let slots = emitted_link_targets(&macos, mode);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].libc, None);
        assert_eq!(slots[0].to_string(), "macos/aarch64");
    }
}

// ---- test_mode.rs: run_test_binary + generate_coverage_report (plan-68-B B1) ----

/// `run_test_binary` maps the child's exit status onto `mfb test`'s result:
/// success passes, a non-zero exit fails, and a failure to even spawn fails.
#[test]
#[cfg(unix)]
fn run_test_binary_maps_every_exit_status() {
    // Exit 0 -> Ok (the success arm).
    assert!(run_test_binary(Path::new("/usr/bin/true"), None).is_ok());
    // Non-zero exit -> Err (the `Ok(_) => Err(())` arm).
    assert!(run_test_binary(Path::new("/usr/bin/false"), None).is_err());
    // Spawn failure on a nonexistent path -> Err (the `Err(err)` arm).
    assert!(run_test_binary(Path::new("/no/such/binary-xyzzy-42"), None).is_err());
}

/// An empty project directory has no `coverage.covmap.json`, so `read_covmap`
/// is `None`: the report warns and returns without writing `coverage.html`.
#[test]
fn generate_coverage_report_warns_when_the_covmap_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    generate_coverage_report(dir.path());
    assert!(!dir.path().join(crate::testing::COVERAGE_HTML).exists());
}

/// With a real covmap seeded (as a `--coverage` build writes), `read_covmap`
/// is `Some`, so `generate_html` and the write arm run and produce the HTML.
#[test]
fn generate_coverage_report_writes_html_from_a_seeded_covmap() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.mfb"), "PRINT 1\n").unwrap();
    crate::testing::coverage::write_covmap(
        &dir.path().join(crate::testing::COVMAP_FILE),
        &[crate::testing::coverage::CovSlot {
            file: "main.mfb".to_string(),
            line: 1,
        }],
    )
    .unwrap();
    generate_coverage_report(dir.path());
    assert!(dir.path().join(crate::testing::COVERAGE_HTML).is_file());
}

// ---- resources.rs: copy_resources remaining branches (plan-68-B B3) ----

/// A root-level glob (`*.png`) has an empty fixed prefix, so `copy_resources`
/// walks the project root itself and strips no prefix from the destination.
#[test]
fn copy_resources_handles_a_root_level_glob() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    std::fs::write(root.join("banner.png"), b"img").unwrap();
    std::fs::write(root.join("notes.txt"), b"skip").unwrap();
    let out = tempfile::tempdir().unwrap();
    let entries = vec![crate::manifest::ResourceEntry {
        src: "*.png".to_string(),
        dst: String::new(),
    }];
    copy_resources(root, &entries, out.path()).expect("root glob copies");
    assert!(out.path().join("banner.png").is_file());
    // A non-matching root file is left behind.
    assert!(!out.path().join("notes.txt").exists());
}

/// An unreadable resource source directory is a hard scan error, not a silent
/// skip (the direct `read_dir` failure inside `collect_files_recursive`).
#[test]
#[cfg(unix)]
fn copy_resources_reports_an_unreadable_source_directory() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("x.ogg"), b"a").unwrap();
    let mut perm = std::fs::metadata(&data).unwrap().permissions();
    perm.set_mode(0o000);
    std::fs::set_permissions(&data, perm).unwrap();
    let out = tempfile::tempdir().unwrap();
    let entries = vec![crate::manifest::ResourceEntry {
        src: "data/*.ogg".to_string(),
        dst: "m/".to_string(),
    }];
    let err = copy_resources(root, &entries, out.path())
        .expect_err("an unreadable source directory must error");
    // Restore so the tempdir can be cleaned up.
    let mut perm = std::fs::metadata(&data).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&data, perm).unwrap();
    assert!(err.contains("failed to scan resources"), "{err}");
}

/// An unreadable subdirectory encountered while walking a resource root is a
/// hard scan error (`collect_files_recursive`'s propagated `read_dir` error).
#[test]
#[cfg(unix)]
fn copy_resources_reports_an_unreadable_subdirectory() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let data = root.join("data");
    let sub = data.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(data.join("top.ogg"), b"a").unwrap();
    std::fs::write(sub.join("deep.ogg"), b"b").unwrap();
    let mut perm = std::fs::metadata(&sub).unwrap().permissions();
    perm.set_mode(0o000);
    std::fs::set_permissions(&sub, perm).unwrap();
    let out = tempfile::tempdir().unwrap();
    let entries = vec![crate::manifest::ResourceEntry {
        src: "data/**/*.ogg".to_string(),
        dst: "all/".to_string(),
    }];
    let err = copy_resources(root, &entries, out.path())
        .expect_err("an unreadable subdirectory must error");
    let mut perm = std::fs::metadata(&sub).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(&sub, perm).unwrap();
    assert!(err.contains("failed to scan resources"), "{err}");
}

/// A destination directory that cannot be created (unwritable resource dir)
/// is a hard error (the `create_dir_all` arm).
#[test]
#[cfg(unix)]
fn copy_resources_reports_an_uncreatable_destination() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("x.ogg"), b"a").unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut perm = std::fs::metadata(out.path()).unwrap().permissions();
    perm.set_mode(0o500);
    std::fs::set_permissions(out.path(), perm).unwrap();
    let entries = vec![crate::manifest::ResourceEntry {
        src: "data/*.ogg".to_string(),
        dst: "sub/".to_string(),
    }];
    let err = copy_resources(root, &entries, out.path())
        .expect_err("an uncreatable destination must error");
    let mut perm = std::fs::metadata(out.path()).unwrap().permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(out.path(), perm).unwrap();
    assert!(err.contains("failed to create"), "{err}");
}

/// A matched source file that cannot be read is a hard copy error (the
/// `fs::copy` arm), distinct from the create-directory arm above.
#[test]
#[cfg(unix)]
fn copy_resources_reports_an_unreadable_source_file() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let data = root.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let file = data.join("x.ogg");
    std::fs::write(&file, b"a").unwrap();
    let mut perm = std::fs::metadata(&file).unwrap().permissions();
    perm.set_mode(0o000);
    std::fs::set_permissions(&file, perm).unwrap();
    let out = tempfile::tempdir().unwrap();
    let entries = vec![crate::manifest::ResourceEntry {
        src: "data/*.ogg".to_string(),
        dst: "m/".to_string(),
    }];
    let err = copy_resources(root, &entries, out.path())
        .expect_err("an unreadable source file must error");
    let mut perm = std::fs::metadata(&file).unwrap().permissions();
    perm.set_mode(0o644);
    std::fs::set_permissions(&file, perm).unwrap();
    assert!(err.contains("failed to copy resource"), "{err}");
}

// ---- native_libs.rs: verify / vendor-source / copy helpers (plan-68-B B4) ----

/// `verify_vendor_libraries` fails, naming the missing file, when a resolved
/// vendor blob is absent from disk.
#[test]
fn verify_vendor_libraries_reports_a_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let library = resolved("sqlite3", "libfoo.so");
    // No source written -> sha256_file errs -> NATIVE_LIBRARY_FILE_MISSING.
    assert!(!verify_vendor_libraries(
        std::slice::from_ref(&library),
        dir.path(),
        OWN_UNIT
    ));
}

/// A vendor locator that records no hash is a malformed package: the verify
/// fails on the "records no hash" arm even though the file is present.
#[test]
fn verify_vendor_libraries_rejects_a_locator_without_a_hash() {
    let dir = tempfile::tempdir().unwrap();
    let mut library = resolved("sqlite3", "libfoo.so");
    library.locator.hash = None;
    write_vendor_source(dir.path(), &library, b"bytes");
    assert!(!verify_vendor_libraries(
        std::slice::from_ref(&library),
        dir.path(),
        OWN_UNIT
    ));
}

/// A present file whose bytes hash to something other than the recorded
/// sha256 is the wrong version of the library and is rejected.
#[test]
fn verify_vendor_libraries_rejects_a_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    // `resolved` records hash [1u8; 32]; real bytes hash to something else.
    let library = resolved("sqlite3", "libfoo.so");
    write_vendor_source(dir.path(), &library, b"totally different bytes");
    assert!(!verify_vendor_libraries(
        std::slice::from_ref(&library),
        dir.path(),
        OWN_UNIT
    ));
}

/// A file whose bytes hash exactly to the recorded sha256 verifies.
#[test]
fn verify_vendor_libraries_accepts_a_matching_hash() {
    let dir = tempfile::tempdir().unwrap();
    let mut library = resolved("sqlite3", "libfoo.so");
    write_vendor_source(dir.path(), &library, b"the real bytes");
    let path = vendor_source_path(dir.path(), OWN_UNIT, &library);
    library.locator.hash = Some(crate::manifest::libraries::sha256_file(&path).unwrap());
    assert!(verify_vendor_libraries(
        std::slice::from_ref(&library),
        dir.path(),
        OWN_UNIT
    ));
}

/// A locator declared by the project's own unit reads from `<root>/vendor/`;
/// an imported unit's reads from `<root>/packages/<unit>.vendor/`.
#[test]
fn vendor_source_path_distinguishes_own_and_imported_units() {
    let root = Path::new("/proj");
    let own = resolved(OWN_UNIT, "libbar.so");
    assert_eq!(
        vendor_source_path(root, OWN_UNIT, &own),
        crate::manifest::libraries::vendor_path(root, "libbar.so"),
    );
    let imported = resolved("other", "libbar.so");
    assert_eq!(
        vendor_source_path(root, OWN_UNIT, &imported),
        crate::manifest::libraries::imported_vendor_path(root, "other", "libbar.so"),
    );
}

/// plan-158-D: a dependency compiled from source (`"source": "file:…"`) has its
/// `.mfp` in the build's package cache, and its vendored libraries beside it in
/// `build/packages/<name>.vendor/` — not in `packages/<name>.vendor/`, which only
/// `mfb pkg` fills. Reading the latter made every such dependency that vendors a
/// library fail with `NATIVE_LIBRARY_FILE_MISSING`.
#[test]
fn vendor_source_path_finds_a_source_dependency_s_staged_vendor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let cache = crate::manifest::package::source_package_cache_dir(root);
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("dep.mfp"), b"mfp").unwrap();
    let library = resolved("dep", "libdep.so");
    assert_eq!(
        vendor_source_path(root, OWN_UNIT, &library),
        cache.join("dep.vendor").join("libdep.so"),
    );
}

/// plan-158-D: building a source dependency stages its flat `vendor/` files into
/// the cache's `<name>.vendor/`, where `vendor_source_path` reads them.
#[test]
fn stage_source_dependency_vendor_copies_the_flat_vendor_files() {
    let dir = tempfile::tempdir().unwrap();
    let dep = dir.path().join("dep");
    std::fs::create_dir_all(dep.join("vendor").join("nested")).unwrap();
    std::fs::write(dep.join("vendor").join("libdep.so"), b"lib").unwrap();
    std::fs::write(dep.join("vendor").join("nested").join("ignored"), b"x").unwrap();
    let cache = dir.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    super::source_packages::stage_source_dependency_vendor(&dep, &cache, "dep").unwrap();
    assert_eq!(
        std::fs::read(cache.join("dep.vendor").join("libdep.so")).unwrap(),
        b"lib"
    );
    assert!(!cache.join("dep.vendor").join("nested").exists());
    // A dependency with no vendor directory stages nothing and succeeds.
    let bare = dir.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    super::source_packages::stage_source_dependency_vendor(&bare, &cache, "bare").unwrap();
    assert!(!cache.join("bare.vendor").exists());
}

/// `copy_vendor_libraries` surfaces the copy error when a resolved vendor
/// source file does not exist on disk (the `fs::copy` failure arm).
#[test]
fn copy_vendor_libraries_errors_when_the_source_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let library = resolved("sqlite3", "libfoo.so");
    // Deliberately do NOT write the source file.
    let out = dir.path().join("out");
    let err = copy_vendor_libraries(
        std::slice::from_ref(&library),
        dir.path(),
        OWN_UNIT,
        std::slice::from_ref(&out),
    )
    .expect_err("a missing source must fail the copy");
    assert!(err.contains("failed to copy vendored library"), "{err}");
}

// ---- build_project: reachable validation/error branches (plan-68-B B6) ----

/// Every `BuildOutput` label is the documented noun (drives the `Wrote …` and
/// package-unsupported lines).
#[test]
fn build_output_label_names_every_variant() {
    assert_eq!(BuildOutput::Ast.label(), "AST");
    assert_eq!(BuildOutput::Ir.label(), "IR");
    assert_eq!(BuildOutput::BinaryRepr.label(), "binary representation");
    assert_eq!(BuildOutput::NativeIr.label(), "native IR");
    assert_eq!(BuildOutput::NativePlan.label(), "native plan");
    assert_eq!(BuildOutput::NativeObjectPlan.label(), "native object plan");
    assert_eq!(BuildOutput::NativeCodePlan.label(), "native code plan");
    assert_eq!(BuildOutput::Mir.label(), "MIR");
}

/// A verbose build runs the `phase …` reporter arms (level-gated stderr; no
/// effect on the emitted bytes).
#[test]
fn build_project_verbose_runs_the_phase_reporter() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    let options = parse_build_options(s(&["-v", dir.path().to_str().unwrap()])).expect("options");
    assert_eq!(options.verbosity, Verbosity::Verbose);
    build_project(&options).expect("verbose build succeeds");
}

/// A `-vv` build runs the whole compile profiler — every span opened deep in
/// codegen, the leaderboards, the counters, and the render — and still
/// produces the same artifact. The span stack is thread-local and each span
/// records against its depth, so an unbalanced open/close would show up here
/// as a panic or a hang rather than silently mis-filing later builds.
#[test]
fn build_project_trace_runs_the_compile_profiler() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    let options = parse_build_options(s(&["-vv", dir.path().to_str().unwrap()])).expect("options");
    assert_eq!(options.verbosity, Verbosity::Trace);
    build_project(&options).expect("trace build succeeds");
}

/// App mode requires an executable project; a package with `--app` is rejected
/// before any lowering.
#[test]
fn build_project_rejects_app_mode_for_a_package() {
    let dir = tempfile::tempdir().unwrap();
    write_package_project(dir.path());
    let options = parse_build_options(s(&["-app", dir.path().to_str().unwrap()])).expect("options");
    assert!(build_project(&options).is_err());
}

/// Write an app-mode executable manifest whose `icon` points at a file that
/// does not exist, so the icon existence check fails.
fn write_app_project_missing_icon(dir: &Path) {
    std::fs::write(
        dir.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"mode\": \"app\",\n",
            "  \"icon\": \"assets/missing.png\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.join("src")).expect("src dir");
    std::fs::write(
        dir.join("src").join("main.mfb"),
        "FUNC main() AS Nothing\nEND FUNC\n",
    )
    .expect("source");
}

/// A missing `icon` is a hard error in app mode, on each app-capable target's
/// build-mode arm (macOS host, and the cross Linux/Windows selectors).
#[test]
fn build_project_app_mode_missing_icon_is_rejected_per_target() {
    for target in [None, Some("linux-aarch64"), Some("windows-x86_64")] {
        let dir = tempfile::tempdir().unwrap();
        write_app_project_missing_icon(dir.path());
        let mut args = vec!["-app".to_string()];
        if let Some(t) = target {
            args.push("-target".to_string());
            args.push(t.to_string());
        }
        args.push(dir.path().to_str().unwrap().to_string());
        let options = parse_build_options(args).expect("options");
        assert!(
            build_project(&options).is_err(),
            "target {target:?}: a missing icon must be rejected"
        );
    }
}

/// The `app` package is importable only in app mode; a console build that
/// imports it is a compile error.
#[test]
fn build_project_rejects_importing_app_without_app_mode() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "IMPORT app\n\nFUNC main() AS Nothing\nEND FUNC\n",
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// A host executable that vendors a native library resolves the vendor
/// locator, hash-verifies the blob, and copies it beside the binary — driving
/// `resolved_vendor_libraries`' resolution loop and the vendor copy on a real
/// build. The `.so` need not be a real library: codegen emits a `dlopen` stub
/// and only the recorded hash is checked at build time.
#[test]
fn build_project_builds_an_executable_that_vendors_a_library() {
    let host = target::BuildTarget::host();
    let dir = tempfile::tempdir().unwrap();
    // A Linux native build emits BOTH libc flavors from one invocation
    // (plan-56-B), and a Linux `vendor` locator must name its exact `arch` and
    // `libc` (§3.2) — a glibc `.so` cannot double as musl, so `libc` may not be
    // omitted. A logical library therefore needs one vendored blob per flavor,
    // each with its own filename (they share the one flat `build/vendor/`), and
    // both are copied. macOS has no libc axis and yields a single target, hence
    // one blob and one copy.
    let (locators, blobs): (String, Vec<&str>) = if host.os == "linux" {
        (
            format!(
                "[ {{ \"os\": \"linux\", \"arch\": \"{arch}\", \"libc\": \"glibc\", \
                       \"type\": \"vendor\", \"source\": \"libfoo-glibc.so\" }}, \
                       {{ \"os\": \"linux\", \"arch\": \"{arch}\", \"libc\": \"musl\", \
                       \"type\": \"vendor\", \"source\": \"libfoo-musl.so\" }} ]",
                arch = host.arch,
            ),
            vec!["libfoo-glibc.so", "libfoo-musl.so"],
        )
    } else {
        (
            format!(
                "[ {{ \"os\": \"{os}\", \"arch\": \"{arch}\", \"type\": \"vendor\", \
                       \"source\": \"libfoo.so\" }} ]",
                os = host.os,
                arch = host.arch,
            ),
            vec!["libfoo.so"],
        )
    };
    std::fs::write(
        dir.path().join("project.json"),
        format!(
            concat!(
                "{{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"libraries\": {{ \"foo\": {locators} }},\n",
                "  \"sources\": [{{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }}]\n",
                "}}\n"
            ),
            locators = locators,
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        concat!(
            "LINK \"foo\" AS fooLib\n",
            "  FUNC ping() AS Nothing\n",
            "    SYMBOL \"foo_ping\"\n",
            "    ABI () AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n\n",
            "FUNC main() AS Nothing\n",
            "  fooLib::ping()\n",
            "END FUNC\n"
        ),
    )
    .unwrap();
    // The vendored blobs (dummy bytes; the build records each one's sha256).
    // The `.so` need not be a real library, so both flavors may share bytes.
    std::fs::create_dir_all(dir.path().join("vendor")).unwrap();
    for blob in &blobs {
        std::fs::write(
            dir.path().join("vendor").join(blob),
            b"\x7fELF dummy vendor blob",
        )
        .unwrap();
    }
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    build_project(&options).expect("vendored build should succeed");
    // Every vendored blob the host's emitted flavors resolve was copied into
    // build/vendor/ beside the executable (one per libc flavor on Linux, one
    // on macOS).
    let vendor_out = dir
        .path()
        .join(crate::os::BUILD_DIR)
        .join(crate::os::VENDOR_DIR);
    let copied: Vec<PathBuf> = std::fs::read_dir(&vendor_out)
        .expect("build/vendor exists")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.is_file())
        .collect();
    assert_eq!(
        copied.len(),
        blobs.len(),
        "every vendored library the host resolves was copied"
    );
}

/// A resources entry whose source resolves outside the project (an in-tree
/// symlink escaping the root) fails the build at the resource-copy step, after
/// codegen — exercising the `copy_resources` error arm in `build_project`.
#[test]
#[cfg(unix)]
fn build_project_reports_a_resource_copy_failure() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.conf"), b"secret").unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"resources\": [{ \"src\": \"assets/*.conf\", \"dst\": \"cfg/\" }],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n",
    )
    .unwrap();
    // An in-tree name that textually looks contained but resolves outside.
    std::os::unix::fs::symlink(outside.path(), dir.path().join("assets")).unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// The same missing-`libraries` error aborts a package build (the package-path
/// `assemble_native_libraries` gate).
#[test]
fn build_project_rejects_a_package_link_without_a_libraries_entry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"lib\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"package\",\n",
            "  \"description\": \"Test fixture package with an unmatched LINK.\",\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"package\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.mfb"),
        concat!(
            "LINK \"foo\" AS fooLib\n",
            "  FUNC ping() AS Nothing\n",
            "    SYMBOL \"foo_ping\"\n",
            "    ABI () AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n\n",
            "EXPORT FUNC go() AS Nothing\n",
            "  fooLib::ping()\n",
            "END FUNC\n"
        ),
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// Write an app-mode executable manifest whose `icon` points at an existing
/// file (dummy bytes). The icon existence check resolves (the `Some(resolved)`
/// arm), then the macOS backend rejects the non-1024×1024 image.
fn write_app_project_present_icon(dir: &Path) {
    std::fs::write(
        dir.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"mode\": \"app\",\n",
            "  \"icon\": \"assets/app.png\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.join("src")).expect("src dir");
    std::fs::write(
        dir.join("src").join("main.mfb"),
        "FUNC main() AS Nothing\nEND FUNC\n",
    )
    .expect("source");
    std::fs::create_dir_all(dir.join("assets")).expect("assets dir");
    // A file that exists (so the existence check passes) but is not a valid
    // 1024×1024 image (so the backend's deep icon validation fails).
    std::fs::write(dir.join("assets").join("app.png"), b"not really a png").expect("icon");
}

/// A present `icon` resolves (the `Some(resolved)` icon arm), then the macOS
/// app backend rejects the dummy image inside `write_executable` — driving the
/// icon-resolves arm and the `write_executable` error arm on the host.
#[test]
fn build_project_app_mode_present_icon_resolves_then_backend_rejects() {
    // Only meaningful when the host is app-capable (macOS): the `write_executable`
    // MacApp arm is what rejects the dummy icon. On a non-app host this build
    // routes through a cross target instead, so guard on the host target.
    if !target::target_supports_app_mode(&target::BuildTarget::host()) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write_app_project_present_icon(dir.path());
    let options = parse_build_options(s(&["-app", dir.path().to_str().unwrap()])).expect("options");
    assert!(
        build_project(&options).is_err(),
        "a present-but-invalid icon must be rejected by the backend"
    );
}

/// App mode with no `icon` field takes the `None => None` icon arm. Routed to a
/// Windows cross target (mfb's internal PE linker builds it without a host
/// toolchain), exercising the no-icon arm and the WindowsApp build mode.
#[test]
fn build_project_app_mode_without_an_icon_field_takes_the_none_arm() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"mode\": \"app\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "FUNC main() AS Nothing\nEND FUNC\n",
    )
    .unwrap();
    let options = parse_build_options(s(&[
        "-app",
        "-target",
        "windows-x86_64",
        dir.path().to_str().unwrap(),
    ]))
    .expect("options");
    build_project(&options).expect("windows app build with no icon succeeds");
}
