//! x86-64 native-plan platform (plan-00-H). The x86 backend uses raw Linux
//! syscalls for the primitives (write/read/exit/mmap/getrandom), so it declares
//! only the libc imports for things with no practical syscall form, emitted via
//! `emit_libc_call` — currently `snprintf` for `toString(Float)`. A build needing
//! none stays a static ELF; one that imports links libc dynamically (PLT/GOT +
//! interpreter), exactly like AArch64.

use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{self, NativePlan, NativePlanPlatform, PlatformImport};
use crate::target::shared::runtime::RuntimeHelperSpec;

pub(crate) fn lower_module(module: &NirModule, flavor: LinuxFlavor) -> Result<NativePlan, String> {
    plan::lower_module_for_platform(module, &Platform { flavor })
}

struct Platform {
    flavor: LinuxFlavor,
}

impl Platform {
    fn libc(&self) -> &'static str {
        match self.flavor {
            LinuxFlavor::Glibc => "libc.so.6",
            LinuxFlavor::Musl => "libc.musl-x86_64.so.1",
        }
    }

    fn libc_import(&self, symbol: &str, required_by: &str) -> PlatformImport {
        PlatformImport {
            library: self.libc().to_string(),
            symbol: symbol.to_string(),
            required_by: required_by.to_string(),
        }
    }
}

impl NativePlanPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-x86_64"
    }

    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
        if module.entry.is_none() {
            return Vec::new();
        }
        // The shared program entry (`lower_program_entry`) always mixes the wall
        // clock into the memory-fill RNG seed via `clock_gettime` (the entropy
        // bytes themselves come from the `getrandom` syscall, so no `getentropy`
        // import). `_exit`/`write` are raw syscalls on x86, so only these libc
        // calls need importing. `signal` installs the console SIGINT/SIGTERM
        // handlers; app mode has no console handlers (mirrors AArch64).
        let mut imports = vec![self.libc_import("clock_gettime", "_main")];
        if !module.build_mode.is_app() {
            imports.push(self.libc_import("signal", "_main"));
        }
        imports
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

    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport> {
        // toString(Float) formats via libc snprintf (no reasonable syscall form).
        // The Float math kernels are all in-tree, so nothing else imports here.
        if target == "toString" {
            return vec![self.libc_import("snprintf", required_by)];
        }
        Vec::new()
    }

    fn link_imports(&self, _required_by: &str) -> Vec<PlatformImport> {
        Vec::new()
    }
}
