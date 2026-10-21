//! What a build's options put into the Linux ELF it writes.
//!
//! `cross_executables` holds the plain build: one ELF per libc world, with the
//! right `e_machine`. These are the options on top of it -- a build that vendors
//! a native library must tell the loader where to look, and one that does not
//! must not.

use super::*;
use crate::target::NativeBuildMode;

const SRC: &str = "\
FUNC main() AS Integer
  RETURN 0
END FUNC
";

/// A scratch project directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mfb_modes_{tag}_{nonce}"));
        std::fs::create_dir_all(&dir).expect("create scratch");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build(arch: &str, vendors: bool) -> Vec<Vec<u8>> {
    let scratch = Scratch::new(arch);
    let ir = crate::testutil::named_ir_for_src(SRC, "modeprog");
    let backend = backend_for(&BuildTarget {
        os: "linux".to_string(),
        arch: arch.to_string(),
    })
    .unwrap_or_else(|err| panic!("linux-{arch}: {err}"));
    let written = backend
        .write_executable(
            &scratch.0,
            &ir,
            &[],
            None,
            NativeBuildMode::Console,
            None,
            None,
            vendors,
            None,
            crate::codegen::debug::DebugOptions::OFF,
            &|_| {},
        )
        .unwrap_or_else(|err| panic!("linux-{arch}: write_executable: {err}"));
    written
        .iter()
        .map(|path| std::fs::read(path).expect("read the artifact"))
        .collect()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// A build that resolved a `vendor` native library points the loader at the
/// `vendor/` directory beside the executable (`$ORIGIN/vendor`), in every libc
/// world and on every Linux architecture; a build that vendored nothing carries
/// no runpath, so every other binary stays byte-identical.
#[test]
fn a_build_that_vendors_a_library_records_the_vendor_runpath_and_one_that_does_not_has_none() {
    for arch in ["x86_64", "aarch64", "riscv64"] {
        for image in build(arch, true) {
            assert!(
                contains(&image, crate::os::ELF_VENDOR_RPATH),
                "linux-{arch}: a vendoring build must name {}",
                crate::os::ELF_VENDOR_RPATH
            );
        }
        for image in build(arch, false) {
            assert!(
                !contains(&image, crate::os::ELF_VENDOR_RPATH),
                "linux-{arch}: a build that vendors nothing must carry no runpath"
            );
        }
    }
}

/// An AppDir's executable sits at `usr/bin/<name>`, so a vendoring app build points
/// the loader at `usr/lib/` (its own runpath, not the console `vendor/` one), and
/// `--app-debug` keeps the intermediate AppDir the seal would otherwise remove.
#[test]
fn an_app_build_that_vendors_a_library_uses_the_appdir_runpath_and_app_debug_keeps_the_appdir() {
    for arch in ["x86_64", "aarch64"] {
        let scratch = Scratch::new(&format!("app_{arch}"));
        let ir = crate::testutil::named_ir_for_src(SRC, "modeapp");
        let backend = backend_for(&BuildTarget {
            os: "linux".to_string(),
            arch: arch.to_string(),
        })
        .unwrap_or_else(|err| panic!("linux-{arch}: {err}"));
        let appdirs = backend
            .write_executable(
                &scratch.0,
                &ir,
                &[],
                None,
                NativeBuildMode::LinuxApp,
                None,
                Some("1.0.0"),
                true,
                None,
                crate::codegen::debug::DebugOptions::OFF,
                &|_| {},
            )
            .unwrap_or_else(|err| panic!("linux-{arch}: app write_executable: {err}"));
        for appdir in &appdirs {
            let executable = std::fs::read(appdir.join("usr/bin/modeapp"))
                .unwrap_or_else(|err| panic!("linux-{arch}: the AppDir executable: {err}"));
            assert!(
                contains(&executable, crate::os::ELF_APPDIR_VENDOR_RPATH),
                "linux-{arch}: an app build must name {}",
                crate::os::ELF_APPDIR_VENDOR_RPATH
            );
            assert!(!contains(&executable, crate::os::ELF_VENDOR_RPATH));
        }
        let sealed = backend
            .finalize_app_bundle(&scratch.0, "modeapp", true)
            .unwrap_or_else(|err| panic!("linux-{arch}: finalize_app_bundle: {err}"));
        assert_eq!(sealed.len(), appdirs.len());
        for appdir in &appdirs {
            assert!(
                appdir.is_dir(),
                "linux-{arch}: --app-debug keeps {}",
                appdir.display()
            );
        }
    }
}

/// A build mode that belongs to another OS's toolkit is a misroute, refused with
/// the backend's own name for what it does support.
#[test]
fn a_build_mode_from_another_operating_system_is_refused() {
    for (os, arch, mode, mode_name) in [
        ("linux", "x86_64", NativeBuildMode::MacApp, "macos-app"),
        ("linux", "aarch64", NativeBuildMode::MacApp, "macos-app"),
        ("windows", "x86_64", NativeBuildMode::LinuxApp, "linux-app"),
    ] {
        let scratch = Scratch::new(&format!("misroute_{os}_{arch}"));
        let ir = crate::testutil::named_ir_for_src(SRC, "misroute");
        let backend = backend_for(&BuildTarget {
            os: os.to_string(),
            arch: arch.to_string(),
        })
        .unwrap_or_else(|err| panic!("{os}-{arch}: {err}"));
        let error = backend
            .write_executable(
                &scratch.0,
                &ir,
                &[],
                None,
                mode,
                None,
                None,
                false,
                None,
                crate::codegen::debug::DebugOptions::OFF,
                &|_| {},
            )
            .err()
            .unwrap_or_else(|| panic!("{os}-{arch}: a {mode_name} build must be refused"));
        assert!(
            error.contains(&format!("do not support the {mode_name} build mode")),
            "{os}-{arch}: {error}"
        );
    }
}
