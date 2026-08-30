//! `term::getBold` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_get_bold`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether the bold attribute is currently set"#;

const DESC: &str = r#"`term::getBold` returns `TRUE` when subsequently drawn text will be bold and
`FALSE` when it will not. It takes no arguments.

The value is the module's current bold attribute read directly. Immediately after
`term::on` — which resets bold to off — it is `FALSE`; afterwards it is whatever
the most recent `term::setBold` passed, until the next `term::setBold` or the
next `term::on`.

This is the *current attribute*, not a property of anything on screen. Each cell
of the grid carries the attributes that were current when its glyph was written,
so this call describes what the next drawing will use.

Unlike most of the module, `term::getBold` does not simply do nothing while TUI
mode is off: it returns the **inert default**, `FALSE`. A program cannot
distinguish "off" from "on with bold disabled" by this call alone — use
`term::isOn` for that.

The call reads state only. It allocates nothing, changes no `term::` state, draws
nothing, and cannot fail."#;

const EX: &str = r#"Set bold and read it back:

```
IMPORT term

SUB main()
  term::on()
  term::setBold(TRUE)
  LET b AS Boolean = term::getBold()
  term::off()
END SUB
```

Toggle the attribute from its current value:

```
IMPORT term

SUB main()
  term::on()
  term::setBold(NOT term::getBold())
  term::off()
END SUB
```"#;
/// `abi_function` body for `term::get_bold` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_get_bold(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_shared::lower_term_helper(
        ctx.call,
        &symbol,
        ctx.term_state_offset,
        ctx.presentation_mode_offset,
        ctx.build_mode,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getBold",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_get_bold),
        }],
    });
}
