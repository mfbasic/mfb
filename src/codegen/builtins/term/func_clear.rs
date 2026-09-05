//! `term::clear` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_clear`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Blank the whole surface and home the cursor"#;

const DESC: &str = r#"`term::clear` blanks every cell of the surface and moves the cursor to the home
position (row 0, column 0). It takes no arguments.

Two details are easy to get wrong and worth stating plainly.

**The cleared surface is black, not your current background colour.** Clearing
does not honour `term::setBackground` — whatever colour you last set, what you
get is a blank black surface. To get a coloured background, set the colour and
then draw over the region: the colour goes into the cells that drawn text
occupies, not into the ones the clear leaves behind.

**The clear does move the cursor.** It homes it to (0, 0), so a following
`term::moveTo(0, 0)` is redundant.

Like the rest of the surface, `term::clear` is buffered: it changes the surface
and sends nothing to the terminal. The blanked screen appears when the
program calls `term::sync`. It also leaves the *current* attributes alone — the
foreground, background, bold, underline, and cursor-visibility settings that
subsequent drawing will use are untouched; only the cells are.

The call is gated: while TUI mode is off it does nothing (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`). `term::on` already hands
back a cleared surface, so an explicit `term::clear` is for blanking again between
frames — which is exactly what the canonical render loop does."#;

const EX: &str = r#"Blank the surface and draw from the top of each frame:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()          ' also homes the cursor to (0, 0)
  io::print("a fresh screen")
  term::sync()
  term::off()
END SUB
```

Clear at the top of a render loop:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET rows AS List OF String = ["first", "second"]
  FOR EACH row IN rows
    term::clear()
    io::print(row)
    term::sync()
  NEXT
  term::off()
END SUB
```"#;
/// `abi_function` body for `term::clear` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_clear(
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
        name: "clear",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_clear),
        }],
    });
}
