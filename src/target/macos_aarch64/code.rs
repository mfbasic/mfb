use std::collections::HashMap;

use crate::arch::aarch64::abi;
use crate::target::shared::code::{self, CodeInstruction, CodeRelocation, NativeCodePlan};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
) -> Result<NativeCodePlan, String> {
    code::lower_module_for_platform(module, native_plan, &Platform)
}

struct Platform;

const DARWIN_PROT_READ_WRITE: &str = "3";
const DARWIN_MAP_PRIVATE_ANON: &str = "4098";
const DARWIN_SYSCALL_MMAP: &str = "33554629";
const DARWIN_SYSCALL_MUNMAP: &str = "33554505";

impl code::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "macos-aarch64"
    }

    fn arch(&self) -> &'static str {
        "aarch64"
    }

    fn preserves_link_register_in_runtime_helpers(&self) -> bool {
        true
    }

    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::branch_link("_exit"),
            abi::branch_self(),
            abi::return_(),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_exit".to_string(),
            kind: "branch26".to_string(),
            binding: "external".to_string(),
            library: Some("libSystem".to_string()),
        });
        Ok(())
    }

    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("_write")
            .ok_or_else(|| "io.print runtime helper requires _write import".to_string())?
            .clone();
        instructions.push(abi::branch_link("_write"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_write".to_string(),
            kind: "branch26".to_string(),
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let library = platform_imports
            .get("_poll")
            .ok_or_else(|| "io.pollInput runtime helper requires _poll import".to_string())?
            .clone();
        instructions.push(abi::branch_link("_poll"));
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: "_poll".to_string(),
            kind: "branch26".to_string(),
            binding: "external".to_string(),
            library: Some(library),
        });
        Ok(())
    }

    fn emit_arena_map(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::move_register("x1", "x23"),
            abi::move_immediate("x2", "Integer", DARWIN_PROT_READ_WRITE),
            abi::move_immediate("x3", "Integer", DARWIN_MAP_PRIVATE_ANON),
            abi::move_immediate("x4", "Integer", &u64::MAX.to_string()),
            abi::move_immediate("x5", "Integer", "0"),
            abi::move_immediate("x16", "Integer", DARWIN_SYSCALL_MMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate("x16", "Integer", DARWIN_SYSCALL_MUNMAP),
            abi::syscall(),
        ]);
        Ok(())
    }
}
