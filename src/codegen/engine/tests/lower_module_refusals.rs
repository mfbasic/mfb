//! `lower_module_for_platform` refuses a module lowered for another target
//! (`engine/builder/lower_module.rs`): every layout and calling-convention
//! decision downstream is the platform's, so a Linux x86-64 module handed to the
//! Linux AArch64 code layer is a named error, not a mis-assembled program.

use crate::os::linux::flavor::LinuxFlavor::Glibc;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io

FUNC main() AS Integer
  io::print(\"x\")
  RETURN 0
END FUNC
";

#[test]
fn a_module_for_another_target_is_refused() {
    let module = nir_for_src(SOURCE, CodeTarget::LinuxX86_64, Console).expect("lowers to NIR");
    let plan = crate::target::linux_x86_64::plan::lower_module(&module, Glibc)
        .expect("the x86-64 plan builds");
    let Err(error) = crate::target::linux_aarch64::code::lower_module(&module, &plan, &[], Glibc)
    else {
        panic!("the AArch64 code layer lowered an x86-64 module");
    };
    assert!(
        error.contains("cannot lower module target 'linux-x86_64'"),
        "{error}"
    );
}
