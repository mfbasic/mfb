//! `LINK` library resolution's two refusals (`engine/builder/link_resolution.rs`):
//! a package whose library table cannot be read, and a codegen platform whose
//! target is not in `<os>-<arch>` form (the libc-specific locator is chosen by
//! that pair, so there is nothing to resolve against). Both are hard build
//! errors, named, rather than a module linked against no library.

use crate::codegen::engine::builder::resolve_link_libraries;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::testutil::{nir_for_linking_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io

LINK \"c\" AS libc
  FUNC pid() AS Integer
    SYMBOL \"getpid\"
    ABI () AS status CInt32
    RETURN status
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::pid()))
  RETURN 0
END FUNC
";

#[test]
fn an_unreadable_package_table_is_refused() {
    let module = nir_for_linking_src(SOURCE, CodeTarget::LinuxX86_64, &["c"])
        .expect("the probe lowers to NIR");
    let missing = std::env::temp_dir().join("mfb-a2-no-such-package.mfp");
    let Err(error) = resolve_link_libraries(&module, &[missing], &TestPlatform) else {
        panic!("a missing package's library table resolved");
    };
    assert!(!error.is_empty());
}

#[test]
fn a_platform_target_not_in_os_arch_form_is_refused() {
    let module = nir_for_linking_src(SOURCE, CodeTarget::LinuxX86_64, &["c"])
        .expect("the probe lowers to NIR");
    // The test platform names itself `linux_aarch64`, with no `-`.
    let Err(error) = resolve_link_libraries(&module, &[], &TestPlatform) else {
        panic!("a platform target with no `-` resolved");
    };
    assert!(
        error.contains("codegen platform target 'linux_aarch64' is not in `<os>-<arch>` form"),
        "{error}"
    );
}
