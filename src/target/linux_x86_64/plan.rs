//! x86-64 native-plan platform (plan-00-H). The x86 backend uses raw Linux
//! syscalls for every OS interaction, so — unlike the AArch64 plan platform,
//! which imports libc symbols — it declares **no** dynamic imports: every
//! `*_imports` hook is empty, which is what makes the linked executable static.

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, NativePlanPlatform, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule, _flavor: LinuxFlavor) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform)
}

struct Platform;

impl NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-x86_64"
    }

    fn entry_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn entry_error_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn program_exit_imports(&self, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn runtime_imports(&self, _spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn native_call_imports(&self, _target: &str, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn link_imports(&self, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }
}
