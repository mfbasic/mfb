//! `term::getForeground` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_get_foreground`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Read the current foreground colour as a `color::Color`"#;

const DESC: &str = r#"`term::getForeground` returns the colour that subsequently drawn text will be
written in, as a `color::Color`. It takes no arguments.

**A program that names the result must `IMPORT color`** as well as `term` —
imports are not transitive and a package cannot re-export another's types. Reading
the channels off the result without naming the type needs no extra import.

**`alpha` is always `255`.** A terminal cell has no alpha channel, so there is
nothing here to report; the field exists because a `color::Color` has four
channels, and it is fixed fully opaque rather than left meaningless.

The value is the module's current foreground attribute, unpacked from the 24-bit
value that `term::setForeground` stored. Immediately after `term::on` — which resets
the foreground to white — it is (255, 255, 255); after a `term::setForeground` call it
is exactly the colour that was set, until the next `term::setForeground` or the next
`term::on`.

Unlike most of the module, this call does not simply do nothing while TUI mode is
off: it returns the **inert default**, white, (255, 255, 255). A program cannot tell "off" from "on
with that foreground set" by this call alone — use `term::isOn` for that.

This is the *current attribute*, not the colour of anything on screen. Each cell
of the grid carries the attributes that were current when its glyph was written,
so this call says what the next drawing will use, not what the cell under the
cursor looks like."#;

const EX: &str = r#"```
IMPORT term
IMPORT color

SUB main()
  term::on()
  term::setForeground(color::rgb(0, 128, 255))
  LET c AS color::Color = term::getForeground()
  term::off()
END SUB
```

Save the current colour, draw a highlight, then restore it. The saved value goes
straight back in, with no channel unpacking:

```
IMPORT term
IMPORT color
IMPORT io

SUB main()
  term::on()
  LET saved AS color::Color = term::getForeground()
  term::setForeground(color::rgb(255, 0, 0))
  io::print("warning")
  term::setForeground(saved)
  term::sync()
  term::off()
END SUB
```"#;
/// `abi_function` body for `term::get_foreground` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_get_foreground(
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
        name: "getForeground",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_get_foreground),
        }],
    });
}
