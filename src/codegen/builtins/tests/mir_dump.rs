//! `mfb build --mir` (`engine/builder/lower_module.rs:lower_module_mir_for_platform`)
//! lowers the same program to the same set of functions as the build, and shows each
//! one's neutral instructions: a dump that dropped or invented a function would
//! describe a program that is not the one compiled.

use crate::target::shared::nir::NirModule;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, native_plan_for_src, nir_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io

FUNC twice(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC main() AS Integer
  io::print(toString(twice(21)))
  RETURN 0
END FUNC
";

fn mir_function_names(module: &NirModule, target: CodeTarget) -> Vec<String> {
    use crate::os::linux::flavor::LinuxFlavor::Glibc;
    let plan = native_plan_for_src(SOURCE, target, Console).expect("the program plans");
    let mir = match target {
        CodeTarget::MacosAarch64 => {
            crate::target::macos_aarch64::code::lower_module_mir(module, &plan, &[])
        }
        CodeTarget::LinuxAarch64 => {
            crate::target::linux_aarch64::code::lower_module_mir(module, &plan, &[], Glibc)
        }
        CodeTarget::LinuxX86_64 => {
            crate::target::linux_x86_64::code::lower_module_mir(module, &plan, &[], Glibc)
        }
        CodeTarget::LinuxRiscv64 => {
            crate::target::linux_riscv64::code::lower_module_mir(module, &plan, &[], Glibc)
        }
        CodeTarget::WindowsX86_64 => {
            crate::target::win_x86_64::code::lower_module_mir(module, &plan, &[])
        }
    }
    .unwrap_or_else(|error| panic!("{}: the MIR lowers: {error}", target.name()));
    for function in &mir.functions {
        assert!(
            !function.instructions.is_empty(),
            "{}: `{}` has no MIR",
            target.name(),
            function.name
        );
    }
    mir.functions.iter().map(|f| f.name.clone()).collect()
}

#[test]
fn the_mir_dump_describes_the_functions_the_build_emits() {
    for target in CodeTarget::ALL {
        let module = nir_for_src(SOURCE, target, Console).expect("the program lowers to NIR");
        let mut mir = mir_function_names(&module, target);
        let mut built: Vec<String> = code_for_src_cached(SOURCE, target, Console)
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect();
        mir.sort();
        built.sort();
        assert_eq!(mir, built, "{}", target.name());
        assert!(mir.iter().any(|name| name == "twice"), "{}", target.name());
    }
}
