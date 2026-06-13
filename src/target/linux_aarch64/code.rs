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
}
