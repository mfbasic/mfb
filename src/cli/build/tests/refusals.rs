//! Builds that stop on the way: a `build/` that cannot be cleared, a config
//! that does not exist or a `project-local.json` that does not parse, and a
//! declared package that is the wrong version or cannot be decoded — for an
//! executable, a package and a native dump alike. Each refusal writes no
//! artifact.

use super::*;
use crate::binary_repr::BinaryReprMetadata;

fn build(dir: &Path, flags: &[&str]) -> Result<(), ()> {
    let mut args = s(flags);
    args.push(dir.to_str().unwrap().to_string());
    build_project(&parse_build_options(args).expect("options"))
}

fn artifacts(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".ncode") || name.ends_with(".mfp"))
        .collect();
    names.sort();
    names
}

#[test]
fn a_build_directory_that_is_a_file_cannot_be_cleared() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    let build_dir = dir.path().join(crate::os::BUILD_DIR);
    std::fs::write(&build_dir, b"not a directory").unwrap();
    assert_eq!(build(dir.path(), &[]), Err(()));
    assert_eq!(std::fs::read(&build_dir).unwrap(), b"not a directory");
}

/// A stale entry in `build/` that cannot be removed (its directory is not
/// writable) stops the build before anything is written.
#[test]
#[cfg(unix)]
fn a_build_directory_entry_that_cannot_be_removed_stops_the_build() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    let locked = dir.path().join(crate::os::BUILD_DIR).join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("stale"), b"x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = build(dir.path(), &[]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(result, Err(()));
    assert!(locked.join("stale").exists());
    assert!(!dir
        .path()
        .join(crate::os::BUILD_DIR)
        .join("app.out")
        .exists());
}

#[test]
fn an_unknown_config_or_an_unreadable_local_config_stops_the_build() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    assert_eq!(build(dir.path(), &["--config", "nosuch"]), Err(()));
    std::fs::write(dir.path().join("project-local.json"), b"{ not json").unwrap();
    assert_eq!(build(dir.path(), &[]), Err(()));
    assert!(!dir.path().join(crate::os::BUILD_DIR).exists());
}

#[test]
fn a_config_value_that_is_no_scalar_stops_the_build() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    let manifest = std::fs::read_to_string(dir.path().join("project.json")).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        manifest.replace(
            "\"targets\"",
            "\"config\": {\"release\": {\"x\": [1]}},\n  \"targets\"",
        ),
    )
    .unwrap();
    assert_eq!(build(dir.path(), &["-ncode"]), Err(()));
    assert!(artifacts(dir.path()).is_empty());
}

/// Declare package `base` (pinned to `=1.0.0`) in `dir`'s manifest, and
/// install `bytes` as its `.mfp`.
fn declare_base(dir: &Path, kind: &str, bytes: &[u8]) {
    let manifest = std::fs::read_to_string(dir.join("project.json")).unwrap();
    let mut manifest = manifest
        .replace("\"kind\": \"executable\"", &format!("\"kind\": \"{kind}\""))
        .replace(
            "\"targets\"",
            "\"packages\": [{\"name\": \"base\", \"version\": \"1.0.0\", \"pin\": true, \
             \"source\": \"file:packages/base.mfp\", \"direct\": true, \"requiredBy\": []}],\n  \
             \"targets\"",
        );
    if kind == "package" {
        // A package has no entry, and must say what it is.
        manifest = manifest.replace("\"entry\": \"main\",", "\"description\": \"A package.\",");
    }
    std::fs::write(dir.join("project.json"), manifest).unwrap();
    std::fs::create_dir_all(dir.join("packages")).unwrap();
    std::fs::write(dir.join("packages/base.mfp"), bytes).unwrap();
}

/// A `.mfp` whose header is sound: version `version`, payload `payload`.
fn package_bytes(version: &str, payload: &[u8]) -> Vec<u8> {
    crate::target::package_mfp::build_package_bytes(
        &BinaryReprMetadata::new("base".to_string(), version.to_string()),
        payload,
        None,
    )
    .unwrap()
}

/// A sound header over a payload that is no binary representation.
fn undecodable() -> Vec<u8> {
    package_bytes("1.0.0", b"MFPC but nothing after it")
}

#[test]
fn a_declared_package_of_another_version_stops_every_build_kind() {
    for flags in [&[][..], &["-ncode"][..]] {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        declare_base(dir.path(), "executable", &package_bytes("2.0.0", b"MFPC"));
        assert_eq!(build(dir.path(), flags), Err(()), "{flags:?}");
        assert!(artifacts(dir.path()).is_empty());
    }
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    declare_base(dir.path(), "package", &package_bytes("2.0.0", b"MFPC"));
    assert_eq!(build(dir.path(), &[]), Err(()));
    assert!(artifacts(dir.path()).is_empty());
}

#[test]
fn a_declared_package_that_cannot_be_decoded_stops_every_build_kind() {
    for flags in [&[][..], &["-ncode"][..]] {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        declare_base(dir.path(), "executable", &undecodable());
        assert_eq!(build(dir.path(), flags), Err(()), "{flags:?}");
        assert!(artifacts(dir.path()).is_empty());
    }
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    declare_base(dir.path(), "package", &undecodable());
    assert_eq!(build(dir.path(), &[]), Err(()));
    assert!(artifacts(dir.path()).is_empty());
}

/// A dependency the manifest declares but nothing imports, and that was never
/// installed: no import names it, so the resolver has nothing to refuse, and
/// the signature gate skips a missing file (it leaves the message to the
/// install check). The install check is what stops the build — for an
/// executable, a package, and a native dump alike — and nothing is written.
#[test]
fn a_declared_but_uninstalled_package_stops_every_build_kind() {
    for (kind, flags) in [
        ("executable", &[][..]),
        ("package", &[][..]),
        ("executable", &["-ncode"][..]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        declare_base(dir.path(), kind, b"");
        std::fs::remove_file(dir.path().join("packages/base.mfp")).unwrap();
        assert_eq!(build(dir.path(), flags), Err(()), "{kind} {flags:?}");
        assert!(artifacts(dir.path()).is_empty(), "{kind} {flags:?}");
        assert!(
            !dir.path()
                .join(crate::os::BUILD_DIR)
                .join("app.out")
                .exists(),
            "{kind} {flags:?}"
        );
    }
}

/// A native dump whose output path is already taken by a directory is refused
/// (the write fails, and the directory is left as it was) on a Linux target,
/// for each of the dumps the Linux backends write themselves.
#[test]
fn a_linux_dump_whose_path_is_a_directory_is_refused() {
    for (flag, extension) in [
        ("-nir", "nir"),
        ("-nplan", "nplan"),
        ("-ncode", "ncode"),
        ("-mir", "mir"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        let taken = dir.path().join(format!("app.{extension}"));
        std::fs::create_dir(&taken).unwrap();
        std::fs::write(taken.join("keep"), b"x").unwrap();
        assert_eq!(
            build(dir.path(), &[flag, "-target", "linux-x86_64"]),
            Err(()),
            "{flag}"
        );
        assert!(taken.join("keep").is_file(), "{flag}");
    }
}
