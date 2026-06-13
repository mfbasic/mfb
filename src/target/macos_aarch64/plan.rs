use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform)
}

struct Platform;

impl plan::NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "macos-aarch64"
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_exit".to_string(),
            required_by: "_main".to_string(),
        }]
    }

    fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
            required_by: "_main".to_string(),
        }]
    }

    fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        match spec.call {
            "io.print" => vec![PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_write".to_string(),
                required_by: spec.symbol.to_string(),
            }],
            _ => Vec::new(),
        }
    }
}
