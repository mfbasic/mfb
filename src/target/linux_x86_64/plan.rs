//! x86-64 native-plan platform (plan-00-H). The x86 backend uses raw Linux
//! syscalls for the primitives (write/exit/mmap/getrandom) and libc for
//! everything with no practical syscall form (pthread, dlopen, the
//! fs/net/term surface), emitted via `emit_external_call`. The plan is
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
    /// (`crypto::func_random_bytes::lower_random_bytes`), so this import stays live on x86.
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

    /// bug-71: `io.flush` is drain-only, so it declares no `fsync` and no write of
    /// its own.
    ///
    /// bug-467 replaced the original `is_empty()` assertion. The arm is no longer
    /// empty and correctly so: `io.flush` runs the shared stdout drain, and the
    /// drain now classifies its own `EPIPE` — restoring `SIG_DFL` and re-raising
    /// SIGPIPE — so that `prog | head` still ends a CLI the way it always has,
    /// despite the process-wide `SIG_IGN` the entry installs to stop a socket peer
    /// from killing the process.
    ///
    /// `__errno_location` is deliberately ABSENT here where the libc-write
    /// backends carry it: x86-64's `write` is a raw `svc` returning `-errno`, so
    /// the classification reads the return value and the accessor would be a dead
    /// dynsym — the same split `term_sync_does_not_import_errno_accessor` pins
    /// (bug-410).
    #[test]
    fn io_flush_imports_only_the_sigpipe_classification() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.flush").expect("io.flush spec");
        let symbols: Vec<String> = platform()
            .runtime_imports(spec)
            .into_iter()
            .map(|imp| imp.symbol)
            .collect();
        assert_eq!(symbols, vec!["signal", "raise"]);
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
            // `write` import survived this guard. plan-110-E: the stream members
            // are tcp's now.
            "tcp.write",
            "tcp.writeText",
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

    /// bug-410: the `term::sync` present-write loop retries EINTR, but x86-64's
    /// `write` is a raw `svc` returning `-errno`, so the retry classifies EINTR
    /// without the accessor. The `__errno_location` import must therefore stay off
    /// this arm (a dead dynsym on x86, the mirror of `write_is_never_imported`); the
    /// libc-write backends (aarch64/riscv64/macOS) add it under `!raw_write`.
    #[test]
    fn term_sync_does_not_import_errno_accessor() {
        let spec =
            crate::target::shared::runtime::spec_for_call("term.sync").expect("term.sync spec");
        assert!(
            platform()
                .runtime_imports(spec)
                .iter()
                .all(|imp| imp.symbol != "__errno_location"),
            "term.sync must not import __errno_location on x86 (raw write returns -errno)"
        );
    }

    /// The io.print family raw-syscalls `write`, so it imports no libc `write`.
    ///
    /// bug-467 replaced the original `is_empty()` assertion. `io.print` writes to
    /// the process's own stdout, so it now classifies its own `EPIPE` and restores
    /// `SIG_DFL` before re-raising SIGPIPE, keeping `prog | head` working despite
    /// the process-wide `SIG_IGN` the entry installs. That block references
    /// `signal` and `raise`, which are therefore declared.
    ///
    /// Still no `write` and still no `__errno_location`: the raw `svc` returns
    /// `-errno`, so both would be dead dynsyms. Those two remain covered by
    /// `write_is_never_imported` and `term_sync_does_not_import_errno_accessor`,
    /// and the exact-set assertion here re-pins them for this call.
    #[test]
    fn io_print_imports_only_the_sigpipe_classification() {
        let spec =
            crate::target::shared::runtime::spec_for_call("io.print").expect("io.print spec");
        let symbols: Vec<String> = platform()
            .runtime_imports(spec)
            .into_iter()
            .map(|imp| imp.symbol)
            .collect();
        assert_eq!(symbols, vec!["signal", "raise"]);
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
