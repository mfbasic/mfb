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
