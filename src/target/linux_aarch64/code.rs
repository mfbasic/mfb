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

const LINUX_PROT_READ_WRITE: &str = "3";
const LINUX_MAP_PRIVATE_ANON: &str = "34";
const LINUX_SYSCALL_MMAP: &str = "222";
const LINUX_SYSCALL_MUNMAP: &str = "215";

impl code::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-aarch64"
    }

    fn arch(&self) -> &'static str {
        "aarch64"
    }

    fn preserves_link_register_in_runtime_helpers(&self) -> bool {
        false
    }

    fn emit_program_exit(
        &self,
        _from: &str,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", "93"),
            abi::syscall(),
            abi::branch_self(),
            abi::return_(),
        ]);
        Ok(())
    }

    fn emit_write(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", "64"),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_arena_map(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::move_register("x1", "x23"),
            abi::move_immediate("x2", "Integer", LINUX_PROT_READ_WRITE),
            abi::move_immediate("x3", "Integer", LINUX_MAP_PRIVATE_ANON),
            abi::move_immediate("x4", "Integer", &u64::MAX.to_string()),
            abi::move_immediate("x5", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MUNMAP),
            abi::syscall(),
        ]);
        Ok(())
    }
}
