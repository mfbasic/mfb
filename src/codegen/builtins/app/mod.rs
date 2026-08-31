//! The built-in `app` package (plan-62 / clean-room registry migration).
//!
//! `app` makes an `--app` program's **presentation mode** — what its window
//! surface currently *is* — a first-class, explicit choice: `app::getMode` reads
//! it and `app::setMode` writes it, choosing from the three `Mode` enum members
//! (`Console` = 0, the default; `None` = 1, windowless; `Canvas` = 2, a 2D
//! graphics surface). The mode is
//! per-execution-context state held in a reserved arena-state slot; the two
//! members lower to a load / store of that slot, exactly like
//! `money::getRounding` / `money::setRounding`.
//!
//! Unlike `money`, the two members lower to **runtime helpers** (`_mfb_rt_app_*`),
//! not inline call-site sequences, so each carries its own per-member
//! `Body::abi_function` lowering in its `func_*.rs` (`func_get_mode::lower_get_mode`
//! loads the slot; `func_set_mode::lower_set_mode` stores it then runs the
//! per-backend surface reconcile). The `abi_function` wrapper threads the per-arena
//! `presentation_mode_offset` through the [`AbiCtx`](crate::codegen::registry::AbiCtx);
//! the reconcile is a `CodegenPlatform::emit_app_mode_reconcile` seam. Their runtime
//! specs are DERIVED from the registry (`registry::runtime_specs`), so there is no
//! hand-written `app_specs.rs`.
//!
//! The `Mode` enum is modeled on the registry via `add_enum` (rendered into the
//! injected source by `get_mfb`, like `money`'s `Rounding` and `datetime`'s
//! enums); there is no source companion. The cross-package `ErrWrongMode`
//! presentation-mode gate that fences `term::` / console-read `io::` helpers stays
//! in the shared code layer (`src/codegen/app/hook/app.rs`) — it is not part of
//! this package.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{EnumVariant, Registry, RegistryEnum, RegistryPackage};

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

The mode is one of the `app::Mode` enum members: `Console` — the terminal-in-a-window
surface (a transcript view, optionally a full-screen `term::` grid), the default —
`None` — windowless, where no surface is presented and `io::print` degrades to
standard output — or `Canvas` — a 2D graphics surface drawn by the `canvas`
package, where `term::` is unavailable but `io::` still works (output degrades to
standard output, input comes from the window's key events).
A program's **initial** mode is decided statically: `Console`
unless the program references `app::setMode` anywhere, in which case it starts in
`None`. This lets a program that intends to manage its own surface start windowless
and bring a window up deliberately, while a program that never touches the mode
keeps the terminal-in-a-window behavior unchanged.

`app::getMode` and `app::setMode` raise no errors from the mode machinery itself:
the argument to `setMode` is an `app::Mode` the type checker has already constrained, and
reading the current mode cannot fail. The mode model is designed to grow: a new
presentation surface is a new `app::Mode` variant entered through `app::setMode`, with
no change to this surface — which is exactly how `Canvas` was added.

The `app::Mode` enum is referenced bare, like every other builtin type: write
`app::Mode.None`, not `app::Mode.None`."#;

/// Register the `app` package on the clean-room registry.
///
/// The `Mode` enum is modeled on the registry (`get_mfb` renders it into the
/// injected source in place of the former hand-written `EXPORT ENUM Mode` in
/// `app_package.mfb`, exactly as `money`/`datetime` do). Its variants' declaration
/// order fixes the discriminants (`Console` = 0, `None` = 1, `Canvas` = 2), which
/// are the values `setMode`/`getMode` store and load — so appending a variant is
/// slot-safe (no existing discriminant moves) but reordering one is not. Each of
/// the two members registers itself from its `func_*.rs`.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("app", MODULE_INTRO, MODULE_DESC);

    pkg.add_enum(RegistryEnum {
        name: "Mode",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Console",
                description: "The terminal-in-a-window surface (the default).",
                advisory: None,
            },
            EnumVariant {
                name: "None",
                description: "Windowless — no surface is presented.",
                advisory: None,
            },
            EnumVariant {
                name: "Canvas",
                description: "A 2D graphics surface drawn by the `canvas` package.",
                advisory: None,
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
        assert!(source.contains("Canvas"));
    }

    /// Variant declaration order fixes the discriminants, and those are the values
    /// `setMode`/`getMode` store into and load from the presentation slot. `Canvas`
    /// must therefore stay appended LAST: reordering would silently repoint every
    /// already-stored slot word (plan-98-A Phase 1).
    #[test]
    fn mode_variant_order_pins_the_discriminants() {
        let pkg = registry().resolve_package("app").expect("app package");
        let mode = pkg
            .enums()
            .iter()
            .find(|e| e.name == "Mode")
            .expect("Mode enum");
        let names: Vec<&str> = mode.variants.iter().map(|v| v.name).collect();
        assert_eq!(names, vec!["Console", "None", "Canvas"]);
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
        assert_eq!(
            registry::call_return_type_typed(GET_MODE)
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("Mode")
        );
        assert_eq!(
            registry::call_return_type_typed(SET_MODE)
                .map(|t| t.name().into_owned())
                .as_deref(),
            Some("Nothing")
        );
    }

    #[test]
    fn members_are_os_seam_runtime_helpers() {
        // Both members lower to a runtime helper (`Body::abi_function`), NOT an
        // inline lowering, so they carry a derived runtime spec and the inline-`TRAP`
        // fallibility census (`native_member_declares_error`, which reports only for
        // `abi_inline` inline natives) declines them.
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
