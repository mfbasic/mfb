//! The built-in `app` package (plan-62 / clean-room registry migration).
//!
//! `app` makes an `--app` program's **presentation mode** — what its window
//! surface currently *is* — a first-class, explicit choice: `app::getMode` reads
//! it and `app::setMode` writes it, choosing from the two `Mode` enum members
//! (`Console` = 0, the default; `None` = 1, windowless). The mode is
//! per-execution-context state held in a reserved arena-state slot; the two
//! members lower to a load / store of that slot, exactly like
//! `money::getRounding` / `money::setRounding`.
//!
//! Unlike `money`, the two members lower to **runtime helpers** (`_mfb_rt_app_*`),
//! not inline call-site sequences, so each carries a `Body::native` OS-seam
//! lowering ([`native::lower_app_helper`]) in both the posix and win slots; the
//! generic runtime-call dispatch (`crate::codegen::os::dispatch_runtime_helper` →
//! `registry::os_helper`) routes each member to it and threads the per-arena
//! `presentation_mode_offset` through the [`OsLowerCtx`](crate::codegen::registry::OsLowerCtx)
//! (app is not OS-family-specific — the per-backend surface reconcile is a
//! `CodegenPlatform` seam invoked from the shared body). Their runtime specs are
//! DERIVED from the registry (`registry::runtime_specs`), so there is no
//! hand-written `app_specs.rs`.
//!
//! The `Mode` enum is modeled on the registry via `add_enum` (rendered into the
//! injected source by `get_mfb`, like `money`'s `Rounding` and `datetime`'s
//! enums); there is no source companion. The cross-package `ErrWrongMode`
//! presentation-mode gate that fences `term::` / console-read `io::` helpers stays
//! in the shared code layer (`src/target/shared/code/app.rs`) — it is not part of
//! this package.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{EnumVariant, Registry, RegistryEnum, RegistryPackage};
pub(crate) mod native;

mod func_get_mode;
mod func_set_mode;

const MODULE_INTRO: &str = r#"Presentation-mode control for `--app` builds"#;
const MODULE_DESC: &str = r#"The `app` package makes an `--app` program's **presentation mode** — what its
window surface currently *is* — a first-class, explicit choice. A running program
reads its mode with `app::getMode` and changes it with `app::setMode`. `app` is a
built-in package, but it is importable **only** in `--app` builds: `IMPORT app` in
a plain console build is a compile-time error, because the package controls an app
window's presentation surface, which a console binary does not have. Enable app
mode with the `-app` build flag or `"mode": "app"` in `project.json`.

The mode is one of the `Mode` enum members: `Console` — the terminal-in-a-window
surface (a transcript view, optionally a full-screen `term::` grid), the default —
or `None` — windowless, where no surface is presented and `io::print` degrades to
standard output. A program's **initial** mode is decided statically: `Console`
unless the program references `app::setMode` anywhere, in which case it starts in
`None`. This lets a program that intends to manage its own surface start windowless
and bring a window up deliberately, while a program that never touches the mode
keeps the terminal-in-a-window behavior unchanged.

`app::getMode` and `app::setMode` raise no errors from the mode machinery itself:
the argument to `setMode` is a `Mode` the type checker has already constrained, and
reading the current mode cannot fail. The mode model is designed to grow: a future
graphical mode is a new `Mode` variant entered through `app::setMode`, with no
change to this surface.

The `Mode` enum is referenced bare, like every other builtin type: write
`Mode.None`, not `app::Mode.None`."#;

/// Register the `app` package on the clean-room registry.
///
/// The `Mode` enum is modeled on the registry (`get_mfb` renders it into the
/// injected source in place of the former hand-written `EXPORT ENUM Mode` in
/// `app_package.mfb`, exactly as `money`/`datetime` do). Its variants' declaration
/// order fixes the discriminants (`Console` = 0, `None` = 1), which are the values
/// `setMode`/`getMode` store and load. Each of the two members registers itself
/// from its `func_*.rs`.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("app", MODULE_INTRO, MODULE_DESC);

    pkg.add_enum(RegistryEnum {
        name: "Mode",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Console",
                description: "The terminal-in-a-window surface (the default).",
            },
            EnumVariant {
                name: "None",
                description: "Windowless — no surface is presented.",
            },
        ],
    });

    func_get_mode::register(&mut pkg);
    func_set_mode::register(&mut pkg);

    r.add_package(pkg);
}

// Man/spec citation anchor: `APP`. The `app/*` man pages and the app §5 spec ground
// their package-level and `Mode`-enum facts here with `[[…/app/mod.rs:APP]]`.

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    const GET_MODE: &str = "app.getMode";
    const SET_MODE: &str = "app.setMode";

    #[test]
    fn app_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("app").expect("app package");
        assert_eq!(pkg.functions().len(), 2);
        // The `Mode` enum is rendered into the injected companion source.
        let source = pkg.get_mfb();
        assert!(source.contains("EXPORT ENUM Mode"));
        assert!(source.contains("Console"));
        assert!(source.contains("None"));
    }

    #[test]
    fn mode_is_a_builtin_type() {
        assert!(registry().is_builtin_type("Mode"));
        assert_eq!(
            registry().qualified_builtin_type("app.Mode"),
            Some("Mode".to_string())
        );
    }

    #[test]
    fn membership_and_return_types() {
        for name in [GET_MODE, SET_MODE] {
            assert_eq!(registry().owning_package(name), Some("app"), "{name}");
        }
        assert_eq!(registry::call_return_type(GET_MODE), Some("Mode"));
        assert_eq!(registry::call_return_type(SET_MODE), Some("Nothing"));
    }

    #[test]
    fn members_are_os_seam_runtime_helpers() {
        // Both members lower to a runtime helper (posix/win OS-seam slots), NOT an
        // inline `common` lowering, so they carry a derived runtime spec and the
        // inline-`TRAP` fallibility census (`native_member_declares_error`, which
        // reports only for `common`-slot inline natives) declines them.
        assert_eq!(registry::native_member_declares_error(GET_MODE), None);
        assert_eq!(registry::native_member_declares_error(SET_MODE), None);
        // Neither member declares an error — the mode machinery is total.
        assert!(!registry().declares_error(GET_MODE, "ErrWrongMode"));
        assert!(!registry().declares_error(SET_MODE, "ErrWrongMode"));
    }

    #[test]
    fn machine_argument_types() {
        // `setMode` takes a single `Mode`; `getMode` takes no arguments.
        assert_eq!(
            registry::argument_types(SET_MODE),
            Some(vec!["Mode".to_string()])
        );
        assert_eq!(registry::argument_types(GET_MODE), Some(vec![]));
    }
}
