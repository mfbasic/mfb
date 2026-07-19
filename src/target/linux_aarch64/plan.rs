//! The linux-aarch64 native-plan delta.
//!
//! The import rules themselves are Linux-invariant and live in
//! [`crate::target::linux_common::plan`] (bug-321); this file supplies the
//! per-arch [`LinuxAbi`] and the one plan method that is genuinely per-backend,
//! `app_mode_imports`.

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common::plan::{LinuxAbi, LinuxPlan};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule, flavor: LinuxFlavor) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform { flavor })
}

/// Nothing on aarch64 is a raw syscall — every primitive routes through libc.
static ABI: LinuxAbi = LinuxAbi {
    target: "linux-aarch64",
    musl_libc: "libc.musl-aarch64.so.1",
    glibc_libpthread: "libpthread.so.0",
    raw_write: false,
    raw_exit: false,
    raw_getrandom: false,
};

struct Platform {
    flavor: LinuxFlavor,
}

impl Platform {
    fn common(&self) -> LinuxPlan<'static> {
        LinuxPlan {
            abi: &ABI,
            flavor: self.flavor,
        }
    }
}

impl plan::NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        self.common().target()
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        self.common().entry_imports(module)
    }

    fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        self.common().entry_error_imports(module)
    }

    fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        self.common().program_exit_imports(required_by)
    }

    fn link_imports(&self, required_by: &str) -> Vec<PlatformImport> {
        self.common().link_imports(required_by)
    }

    fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
        self.common().runtime_imports(spec)
    }

    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport> {
        self.common().native_call_imports(target, required_by)
    }

    fn app_mode_imports(&self) -> Vec<PlatformImport> {
        // Shared with the sibling Linux backend
        // (src/target/linux_gtk/mod.rs::app_mode_imports). The C-library
        // sonames are this Platform's, so a musl app build declares musl
        // libraries (plan-56-A §4.1).
        let common = self.common();
        crate::target::linux_gtk::app_mode_imports(crate::target::linux_gtk::AppLibcNames {
            libc: common.libc(),
            libpthread: common.libpthread(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::plan::NativePlanPlatform;

    /// plan-01-libm-kernels Phase 5: no `math.*` target resolves to a `libm.so`
    /// import on either flavor (the kernels are all in-tree).
    #[test]
    fn no_libm_math_imports() {
        for flavor in [LinuxFlavor::Glibc, LinuxFlavor::Musl] {
            let platform = Platform { flavor };
            for target in [
                "math.pow",
                "math.exp",
                "math.log",
                "math.log10",
                "math.fmod",
                "math.sin",
                "math.cos",
                "math.tan",
                "math.asin",
                "math.acos",
                "math.atan",
                "math.atan2",
            ] {
                assert!(
                    platform.native_call_imports(target, "_main").is_empty(),
                    "{target} still resolves to a libm import ({flavor:?})"
                );
            }
        }
    }

    /// bug-321: aarch64 raw-syscalls nothing, so `write` stays a libc import
    /// everywhere it is used — the mirror image of the x86-64 guard.
    #[test]
    fn write_is_imported() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.print").expect("io.print spec");
        let platform = Platform {
            flavor: LinuxFlavor::Glibc,
        };
        assert!(
            platform
                .runtime_imports(spec)
                .iter()
                .any(|imp| imp.symbol == "write"),
            "io.print must import libc write on aarch64"
        );
    }

    /// bug-71: `fs.createTempFile` draws entropy via `emit_random_bytes`, which
    /// on aarch64 is the libc `getentropy` call — so the import is live.
    #[test]
    fn create_temp_file_imports_getentropy() {
        let spec = crate::target::shared::runtime::spec_for_call("fs.createTempFile")
            .expect("fs.createTempFile spec");
        let platform = Platform {
            flavor: LinuxFlavor::Glibc,
        };
        assert!(
            platform
                .runtime_imports(spec)
                .iter()
                .any(|imp| imp.symbol == "getentropy"),
            "fs.createTempFile should import getentropy on aarch64"
        );
    }

    /// bug-71: `io.flush` is drain-only (`lower_io_flush_helper` never fsyncs /
    /// reads errno), so its runtime import arm must be empty on both flavors.
    #[test]
    fn io_flush_imports_nothing() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.flush").expect("io.flush spec");
        for flavor in [LinuxFlavor::Glibc, LinuxFlavor::Musl] {
            let platform = Platform { flavor };
            assert!(
                platform.runtime_imports(spec).is_empty(),
                "io.flush should import nothing ({flavor:?})"
            );
        }
    }

    /// The glibc pthread soname is per-backend (x86-64 declares libc instead),
    /// so it is a `LinuxAbi` field rather than a shared constant.
    #[test]
    fn glibc_threads_bind_to_libpthread() {
        let spec =
            crate::target::shared::runtime::spec_for_call("thread.start").expect("thread.start");
        let platform = Platform {
            flavor: LinuxFlavor::Glibc,
        };
        assert!(
            platform
                .runtime_imports(spec)
                .iter()
                .all(|imp| imp.library == "libpthread.so.0"),
            "glibc aarch64 thread imports bind to libpthread.so.0"
        );
    }
}
