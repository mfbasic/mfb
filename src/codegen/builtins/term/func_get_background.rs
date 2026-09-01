//! `term::getBackground` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_get_background`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Read the current background colour as a `term::TermColor`"#;

const DESC: &str = r#"`term::getBackground` returns the colour drawn behind subsequently written text,
as a `term::TermColor` record with three `Byte` fields `r`, `g`, and
`b` holding the red, green, and blue channels. It takes no arguments.

The value is the module's current background attribute, unpacked from the 24-bit
value that `term::setBackground` stored. Immediately after `term::on` — which
resets the background to black — it is (0, 0, 0); after a `term::setBackground`
call it is exactly the triple that was set, until the next `term::setBackground`
or the next `term::on`.

This is the *current attribute*, not the colour of anything on screen. Each cell
of the grid carries the attributes that were current when its glyph was written,
so this call says what the next drawing will use. Note in particular that
`term::clear` blanks the surface to black rather than painting it with this colour, so a
cleared surface is black whatever `term::getBackground` reports.

Unlike most of the module, `term::getBackground` does not simply do nothing while
TUI mode is off: it returns the **inert default**, black (0, 0, 0). A program
cannot distinguish "off" from "on and set to black" by this call alone — use
`term::isOn` for that.

The call reads state only: it changes no `term::` state, moves no cursor, and
draws nothing. It can still fail, because building the returned record can
fail when memory is exhausted."#;

const EX: &str = r#"Set a background colour and read it back:

```
IMPORT term

SUB main()
  term::on()
  term::setBackground(0, 128, 255)
  LET c AS term::TermColor = term::getBackground()
  term::off()
END SUB
```

Inspect the individual channels:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET c AS term::TermColor = term::getBackground()
  term::off()
  io::print(toString(c.r) & "," & toString(c.g) & "," & toString(c.b))
END SUB
```"#;
/// `abi_function` body for `term::get_background` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_get_background(
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
        name: "getBackground",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("TermColor"),
            errors: vec![],
            body: Body::abi_function(lower_get_background),
        }],
    });
}
