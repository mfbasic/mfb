//! The built-in `os` package (plan-31 / plan-55-B).
//!
//! `os` reaches the host process: it reads, tests, sets, unsets, and enumerates
//! environment variables, and reports read-only facts about the running process
//! and platform (command-line arguments, process id, executable path, OS family,
//! CPU architecture, host and user names, and CPU count).
//!
//! Migrated onto the clean-room registry (`crate::codegen::registry`) and onto the
//! `Body::abi_function` clean-room shape (crypto/io/fs/net). Each member owns its
//! `abi_function` body in its own `func_*.rs` (`lower_<name>`, branching on
//! `platform.family()` internally); genuinely-shared emitters live in `gen_env`,
//! `gen_introspect`, `gen_paths`, and `gen_shared` and are called by those bodies.
//! `os` owns **no** resource handle — `os::resourcePath` merely *returns* a
//! `String`; the substring "resource" names no `RegistryResource`.
//!
//! `os::resourcePath` is the one member that consumes per-compilation build
//! context: [`func_resource_path::lower_resource_path`] reads the real
//! `build_mode`/`module_name` off the `AbiCtx` (the strip/suffix selection baked
//! into the resource-base offset). Every other member accepts and ignores them.
//!
//! `os` is a fully data-only package: each call's return type is fixed per name,
//! no overload uses an argument union, and it contributes no builtin value type or
//! source companion — the registry's generic overload/return resolution answers
//! arity/return/validation with no custom resolver.

use crate::codegen::registry::{Registry, RegistryPackage};

mod gen_env;
mod gen_introspect;
mod gen_paths;
mod gen_shared;
pub(crate) use gen_env::{module_uses_env_lock, os_env_lock_init_hex};
pub(crate) use gen_shared::{
    OS_ARGC_GLOBAL_SYMBOL, OS_ARGV_GLOBAL_SYMBOL, OS_ENV_LOCK_SIZE, OS_ENV_LOCK_SYMBOL,
};

mod func_arch;
mod func_args;
mod func_cpu_count;
mod func_environ;
mod func_executable_path;
mod func_get_env;
mod func_get_env_or;
mod func_has_env;
mod func_host_name;
mod func_name;
mod func_pid;
mod func_resource_path;
mod func_set_env;
mod func_unset_env;
mod func_user_name;

const MODULE_INTRO: &str = r#"Process environment and platform introspection"#;
const MODULE_DESC: &str = r#"The `os` package reaches the host process: it reads, tests, sets, unsets, and
enumerates environment variables, and reports read-only facts about the running
process and platform (command-line arguments, process id, executable path, OS
family, CPU architecture, host and user names, and CPU count). `os` is a
built-in package, so `IMPORT os` needs no manifest dependency.

The introspection calls are all nullary and read-only. `os::name` and `os::arch`
are compile-time constants selected by the build target (`"macos"`/`"linux"`;
`"aarch64"`/`"x86_64"`/`"riscv64"`). `os::args` returns the command-line
arguments **after** the program name (element 0 is the first real argument, not
the executable — the program name is available through `os::executablePath`).
`os::pid` and `os::cpuCount` return an `Integer`; `os::hostName`, `os::userName`,
and `os::executablePath` return a `String` and raise `ErrUnsupported` if the host
lookup fails. `os::resourcePath(relative)` is the one call taking an argument: it
maps a build-relative resource path to its absolute on-disk location for the
running build shape (console → beside the executable; macOS `--app` →
`Contents/Resources`; Linux `--app` → `usr/share/<name>`), raising
`ErrInvalidPath` on a `.`/`..` component and `ErrUnsupported` if the executable
path cannot be found.

Variable names and values are UTF-8 `String` values passed to and from the host
C library (`getenv`, `setenv`, `unsetenv`, and the platform environ accessor).
A name must be non-empty and, like a value, may not contain an embedded NUL byte
or, for a name, an `=` — the host requires NUL-terminated strings and uses `=`
to separate a name from its value.

Reads observe the live environment: `os::getEnv`, `os::getEnvOr`, `os::hasEnv`,
and `os::environ` all reflect both variables inherited from the host and any
changes a prior `os::setEnv`/`os::unsetEnv` made earlier in the same process. A
missing variable is a first-class outcome: `os::getEnv` raises `ErrNotFound`,
while `os::getEnvOr` returns a caller-supplied fallback and `os::hasEnv` reports
presence as a `Boolean`, so a program can choose whether absence is an error.

`os::environ` returns a `Map OF String TO String` snapshot built by walking the
process environment array and splitting each `NAME=VALUE` entry at its first `=`;
an `=` inside a value is preserved as part of the value. The map is an ordinary
owned value taken at the moment of the call and does not track later mutations.

`os::setEnv` and `os::unsetEnv` mutate process-global state. They are **not**
synchronized against a concurrent `os::getEnv`/`os::environ` running in another
`thread::` worker — this is the classic `getenv`/`setenv` data race and is the
caller's responsibility to avoid. All returned `String`, `Boolean`, and
`Map OF String TO String` values follow the ordinary owned-value rules; the
package holds no resource handles."#;

/// Register the `os` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("os", MODULE_INTRO, MODULE_DESC);

    func_get_env::register(&mut pkg);
    func_get_env_or::register(&mut pkg);
    func_has_env::register(&mut pkg);
    func_set_env::register(&mut pkg);
    func_unset_env::register(&mut pkg);
    func_environ::register(&mut pkg);
    func_args::register(&mut pkg);
    func_pid::register(&mut pkg);
    func_executable_path::register(&mut pkg);
    func_resource_path::register(&mut pkg);
    func_name::register(&mut pkg);
    func_arch::register(&mut pkg);
    func_host_name::register(&mut pkg);
    func_user_name::register(&mut pkg);
    func_cpu_count::register(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn os_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("os").expect("os package");
        assert_eq!(pkg.functions().len(), 15);
        // os contributes no builtin value type and owns no resource.
        assert!(!registry().is_builtin_type("os"));
    }

    #[test]
    fn generic_dispatch_reaches_os() {
        assert!(registry().is_member("os.getEnv"));
        assert!(registry().is_member("os.resourcePath"));
        assert!(!registry().is_member("os.nope"));
        // Native members carry no rewrite target (they lower through Body::abi_function).
        assert_eq!(registry::rewrite_target("os.getEnv", &[]), None);
        // Fixed per-name return types.
        assert_eq!(
            registry::call_return_type("os.getEnv").as_deref(),
            Some("String")
        );
        assert_eq!(
            registry::call_return_type("os.hasEnv").as_deref(),
            Some("Boolean")
        );
        assert_eq!(
            registry::call_return_type("os.setEnv").as_deref(),
            Some("Nothing")
        );
        assert_eq!(
            registry::call_return_type("os.pid").as_deref(),
            Some("Integer")
        );
        assert_eq!(
            registry::call_return_type("os.cpuCount").as_deref(),
            Some("Integer")
        );
        assert_eq!(
            registry::call_return_type("os.environ").as_deref(),
            Some("Map OF String TO String")
        );
        assert_eq!(
            registry::call_return_type("os.args").as_deref(),
            Some("List OF String")
        );
        assert_eq!(
            registry::call_return_type("os.resourcePath").as_deref(),
            Some("String")
        );
    }

    #[test]
    fn niladic_calls_render_no_arguments() {
        // The bespoke "no arguments" phrasing rides on each niladic member's
        // descriptor `expected_arguments` field (the per-position render would
        // otherwise decline for a zero-parameter member).
        assert_eq!(
            crate::codegen::registry::expected_arguments("os.pid"),
            Some("no arguments")
        );
        assert_eq!(
            crate::codegen::registry::expected_arguments("os.environ"),
            Some("no arguments")
        );
        // The argument-taking calls render per-position from the registry.
        assert_eq!(
            crate::codegen::registry::expected_arguments("os.getEnv"),
            Some("String")
        );
        assert_eq!(
            crate::codegen::registry::expected_arguments("os.setEnv"),
            Some("String, String")
        );
    }

    #[test]
    fn os_owns_no_resource() {
        assert_eq!(crate::codegen::builtins::resource_close_function("os"), None);
        assert!(registry()
            .resolve_package("os")
            .expect("os")
            .resources()
            .is_empty());
    }
}
