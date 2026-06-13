use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform)
}

struct Platform;

impl plan::NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-aarch64"
    }

    fn entry_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn entry_error_imports(&self, _module: &NirModule) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn runtime_imports(&self, _spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        Vec::new()
    }
}
