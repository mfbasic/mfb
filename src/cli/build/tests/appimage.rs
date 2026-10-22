//! A Linux `-app` build of a project that vendors a native library: each libc
//! world's AppImage carries that world's copy of the library and not the
//! other's (plan-56-B §4.3).

use super::*;

/// The libsnd fixture's sources and manifest, with a stub file per vendor
/// locator (the build hashes and copies them; nothing loads them).
fn vendoring_project(dir: &Path) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rt-behavior/native/libsnd-open-file-info-rt");
    std::fs::copy(fixture.join("project.json"), dir.join("project.json")).unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::copy(fixture.join("src/main.mfb"), dir.join("src/main.mfb")).unwrap();
    std::fs::create_dir_all(dir.join("vendor")).unwrap();
    for entry in std::fs::read_dir(fixture.join("vendor")).unwrap() {
        let name = entry.unwrap().file_name();
        let stub = format!("stub {}", name.to_string_lossy());
        std::fs::write(dir.join("vendor").join(&name), stub).unwrap();
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn each_libc_world_s_appimage_carries_only_its_own_vendored_library() {
    let dir = tempfile::tempdir().unwrap();
    vendoring_project(dir.path());
    let project = dir.path().to_str().unwrap();
    let options =
        parse_build_options(s(&["-app", "-target", "linux-x86_64", project])).expect("options");
    build_project(&options).expect("the app builds");
    let build = dir.path().join(crate::os::BUILD_DIR);
    let image = |flavor: &str| {
        std::fs::read(build.join(format!("libsnd_open_file_info_rt-{flavor}.AppImage"))).unwrap()
    };
    let (glibc, musl) = (image("glibc"), image("musl"));
    let glibc_lib = "stub libsndfile.so.1.0.37-x86_64-glibc";
    let musl_lib = "stub libsndfile.so.1.0.37-x86_64-musl";
    assert!(contains(&glibc, glibc_lib) && !contains(&glibc, musl_lib));
    assert!(contains(&musl, musl_lib) && !contains(&musl, glibc_lib));
}

/// A `libraries` entry that covers the target's OS and architecture but not
/// one of the libc worlds a Linux build emits: the manifest check only warns
/// (the entry exists and its files hash), and resolving the glibc world's
/// vendor library is what refuses the build, before anything is written.
#[test]
fn a_vendor_library_with_no_locator_for_one_libc_world_stops_the_build() {
    let dir = tempfile::tempdir().unwrap();
    vendoring_project(dir.path());
    let manifest = std::fs::read_to_string(dir.path().join("project.json")).unwrap();
    let glibc = "      {\n        \"os\": \"linux\",\n        \"arch\": \"x86_64\",\n        \"libc\": \"glibc\",\n        \"type\": \"vendor\",\n        \"source\": \"libsndfile.so.1.0.37-x86_64-glibc\"\n      },\n";
    assert!(manifest.contains(glibc), "fixture manifest layout changed");
    std::fs::write(dir.path().join("project.json"), manifest.replace(glibc, "")).unwrap();
    let project = dir.path().to_str().unwrap();
    let options = parse_build_options(s(&["-target", "linux-x86_64", project])).expect("options");
    assert_eq!(build_project(&options), Err(()));
    let build = dir.path().join(crate::os::BUILD_DIR);
    for flavor in ["glibc", "musl"] {
        assert!(!build
            .join(format!("libsnd_open_file_info_rt-{flavor}.out"))
            .exists());
    }
}

/// A `LINK` block with no `libraries` entry to locate it: the native dump
/// refuses it as the build does.
#[test]
fn a_native_dump_of_a_link_block_with_no_locator_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    vendoring_project(dir.path());
    let manifest = std::fs::read_to_string(dir.path().join("project.json")).unwrap();
    let start = manifest.find("\"libraries\"").unwrap();
    let end = manifest.find("\"sources\"").unwrap();
    let manifest = format!("{}{}", &manifest[..start], &manifest[end..]);
    std::fs::write(dir.path().join("project.json"), manifest).unwrap();
    let project = dir.path().to_str().unwrap();
    let options =
        parse_build_options(s(&["-ncode", "-target", "linux-x86_64", project])).expect("options");
    assert_eq!(build_project(&options), Err(()));
    assert!(!dir.path().join("libsnd_open_file_info_rt.ncode").exists());
}
