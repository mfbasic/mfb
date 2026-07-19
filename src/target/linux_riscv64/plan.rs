//! The linux-riscv64 native-plan delta.
//!
//! The import rules themselves are Linux-invariant and live in
//! [`crate::target::linux_common::plan`] (bug-321); this file supplies the
//! per-arch [`LinuxAbi`] and the app-mode hard-stop.

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common::plan::{LinuxAbi, LinuxPlan};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule, flavor: LinuxFlavor) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform { flavor })
}

/// Nothing on riscv64 is a raw syscall — every primitive routes through libc.
static ABI: LinuxAbi = LinuxAbi {
    target: "linux-riscv64",
    musl_libc: "libc.musl-riscv64.so.1",
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
        // bug-117.1: app mode was never ported to rv64, and plan-51-A §3.3
        // records why it is now permanently out — AppImage/type2-runtime
        // publishes no riscv64 runtime, so an AppDir could never be sealed.
        // `supports_app_mode()` is false and the backend rejects an app build
        // before lowering, so reaching here is a bug; say so loudly rather than
        // returning an empty import list that would yield a silently broken
        // binary. Mirrors the `AppSupport::Unsupported` hard-stops in `code.rs`.
        unimplemented!("{}", super::code::APP_MODE_UNPORTED);
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

    /// bug-71: `io.flush` is drain-only (`lower_io_flush_helper` never fsyncs /
    /// reads errno), so its runtime import arm must be empty on both flavors — no
    /// dead `fsync`/`__errno_location` symbols.
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

    /// bug-71: `fs.createTempFile` draws entropy via `emit_random_bytes`, which on
    /// riscv is the libc `getentropy` call — so unlike x86 the import is live.
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
            "fs.createTempFile should import getentropy on riscv"
        );
    }

    /// bug-321: riscv64 raw-syscalls nothing, so `write` stays a libc import.
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
            "io.print must import libc write on riscv64"
        );
    }

    /// bug-223 defense layer: the plan's app-mode import hook hard-stops rather
    /// than returning an empty list that would yield a silently broken binary.
    #[test]
    #[should_panic(expected = "rv64 app mode not ported")]
    fn app_mode_imports_hard_stops() {
        let platform = Platform {
            flavor: LinuxFlavor::Glibc,
        };
        let _ = platform.app_mode_imports();
    }
}
