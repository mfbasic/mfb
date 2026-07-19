//! x86-64 native-plan platform (plan-00-H). The x86 backend uses raw Linux
//! syscalls for the primitives (write/exit/mmap/getrandom) and libc for
//! everything with no practical syscall form (pthread, dlopen, the
//! fs/net/term surface), emitted via `emit_libc_call`. The plan is
//! flavor-parameterized: each import binds to `libc.so.6` (glibc) or
//! `libc.musl-x86_64.so.1` (musl), and the console build emits one executable
//! per flavor, exactly like AArch64. A build importing nothing stays a static
//! ELF; one that imports links libc dynamically (PLT/GOT + interpreter).
//!
//! The import rules themselves are Linux-invariant and live in
//! [`crate::target::linux_common::plan`] (bug-321). The raw-syscall policy that
//! makes x86-64 differ is declared once, in [`ABI`] — every arm that would
//! otherwise import a libc wrapper for a raw-syscalled primitive consults it, so
//! the policy can no longer drift arm by arm. Each flag must match the
//! corresponding override in [`super::code`], or the plan declares a dead
//! dynamic symbol (bug-71, bug-79.4).

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common::plan::{LinuxAbi, LinuxPlan};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, NativePlanPlatform, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule, flavor: LinuxFlavor) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform { flavor })
}

/// x86-64 raw-syscalls `write` (nr 1), `exit_group` (nr 231), and `getrandom`
/// (nr 318), so none of their libc wrappers may be imported.
static ABI: LinuxAbi = LinuxAbi {
    target: "linux-x86_64",
    musl_libc: "libc.musl-x86_64.so.1",
    // On musl and modern glibc, pthread lives in libc.
    glibc_libpthread: "libc.so.6",
    raw_write: true,
    raw_exit: true,
    raw_getrandom: true,
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

impl NativePlanPlatform for Platform {
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

    fn platform() -> Platform {
        Platform {
            flavor: LinuxFlavor::Glibc,
        }
    }

    /// bug-71: x86 exits via the raw `exit_group` syscall in `emit_program_exit`,
    /// so `program_exit_imports` must declare no libc `_exit` (a dead symbol
    /// copied from AArch64, which does call libc `_exit`).
    #[test]
    fn program_exit_imports_nothing() {
        assert!(platform().program_exit_imports("_main").is_empty());
    }

    /// bug-71: x86 `emit_random_bytes` is the raw `getrandom` syscall, so
    /// `fs.createTempFile` must not import libc `getentropy` (dead on x86).
    #[test]
    fn create_temp_file_does_not_import_getentropy() {
        let spec = crate::target::shared::runtime::spec_for_call("fs.createTempFile")
            .expect("fs.createTempFile spec");
        assert!(
            platform()
                .runtime_imports(spec)
                .iter()
                .all(|imp| imp.symbol != "getentropy"),
            "fs.createTempFile must not import getentropy on x86"
        );
    }

    /// bug-71: `crypto.randomBytes` calls libc `getentropy` directly
    /// (`lower_crypto_random_bytes_helper`), so this import stays live on x86.
    #[test]
    fn crypto_random_bytes_imports_getentropy() {
        let spec = crate::target::shared::runtime::spec_for_call("crypto.randomBytes")
            .expect("crypto.randomBytes spec");
        assert!(
            platform()
                .runtime_imports(spec)
                .iter()
                .any(|imp| imp.symbol == "getentropy"),
            "crypto.randomBytes must import getentropy on x86"
        );
    }

    /// bug-71: `io.flush` is drain-only, so its runtime import arm is empty — no
    /// dead `fsync`/`__errno_location`.
    #[test]
    fn io_flush_imports_nothing() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.flush").expect("io.flush spec");
        assert!(platform().runtime_imports(spec).is_empty());
    }

    /// bug-79.4: x86 emits `write` as a raw syscall (`emit_write`, nr 1), never a
    /// libc PLT call, so every runtime helper that writes must not import the
    /// `write` wrapper (a dead unreferenced dynsym copied from AArch64).
    #[test]
    fn write_is_never_imported() {
        for call in [
            "io.print",
            "io.write",
            "io.printError",
            "io.writeError",
            "io.input",
            "term.on",
            "term.clear",
            "term.moveTo",
            "fs.writeText",
            "fs.open",
            "fs.writeTextAtomic",
            // bug-300 E10: these two were omitted, which is why the dead net
            // `write` import survived this guard.
            "net.write",
            "net.writeText",
        ] {
            let spec = crate::target::shared::runtime::spec_for_call(call)
                .unwrap_or_else(|| panic!("{call} spec"));
            assert!(
                platform()
                    .runtime_imports(spec)
                    .iter()
                    .all(|imp| imp.symbol != "write"),
                "{call} must not import libc write on x86 (raw syscall)"
            );
        }
    }

    /// The io.print family raw-syscalls `write`, so its import arm is empty.
    #[test]
    fn io_print_imports_nothing() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.print").expect("io.print spec");
        assert!(platform().runtime_imports(spec).is_empty());
    }

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

    /// bug-321: unlike aarch64/riscv64, the x86-64 backend binds its glibc
    /// pthread imports to libc rather than `libpthread.so.0`. That difference is
    /// a `LinuxAbi` field, not an accident of the copy it was forked from.
    #[test]
    fn glibc_threads_bind_to_libc() {
        let spec =
            crate::target::shared::runtime::spec_for_call("thread.start").expect("thread.start");
        assert!(
            platform()
                .runtime_imports(spec)
                .iter()
                .all(|imp| imp.library == "libc.so.6"),
            "glibc x86-64 thread imports bind to libc.so.6"
        );
    }
}
