//! `term::isOn` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_is_on`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether TUI mode is currently on"#;

const DESC: &str = r#"`term::isOn` returns `TRUE` while the `term::` surface is active — after
`term::on` and before the matching `term::off` — and `FALSE` otherwise, including
before any `term::on` call. It takes no arguments.

While TUI mode is off, most `term::` calls short-circuit: the colour and
attribute setters, `term::hideCursor`, `term::showCursor`, `term::clear`,
`term::moveTo`, `term::sync` and all six drawing calls (`drawHLine`, `drawVLine`,
`drawBox`, `fillRect`, `drawText`, `drawGlyph`) do nothing, and
`term::getForeground`/`getBackground`/`getBold`/`getUnderline` return inert
defaults rather than live state. Two reads have no inert answer and raise
`ErrUnsupported` instead: `term::terminalSize` and `term::measureText`. The rest
work whether TUI mode is on or not: `term::on`, `term::isOn`, `term::didResize`,
and the two mouse calls — `term::enableMouse` switches mouse reporting on or off
at once, even before `term::on`, and `term::pollMouse` reports what arrived.
`term::off` while off only turns off mouse reporting a program left on. That is
what makes this query useful: it is the way to find out whether the surface will
actually do anything.

The answer changes only at `term::on` and `term::off`. `term::off` while already
off leaves it `FALSE`; `term::on` while already on resets the surface again, but
the answer stays `TRUE` throughout.

The call reads state only: it touches neither the terminal, the alternate screen,
nor the surface. In a terminal program it cannot fail; in an `mfb build --app` program, `term::isOn` — like every other `term::` call — raises `ErrWrongMode` unless the program is in `app::Mode.Console`; see `mfb man app`."#;

const EX: &str = r#"Enter TUI mode only once, and leave it at the end:

```
IMPORT term

FUNC main() AS Nothing
  IF NOT term::isOn() THEN
    term::on()
  END IF
  term::off()
END FUNC
```

Draw only when the surface is live — the first call does nothing, the second
draws `status`:

```
IMPORT term
IMPORT io

FUNC drawStatus() AS Nothing
  IF term::isOn() THEN
    term::clear()
    term::moveTo(0, 0)
    io::print("status")
    term::sync()
  END IF
END FUNC

FUNC main() AS Nothing
  drawStatus()
  term::on()
  drawStatus()
  term::off()
END FUNC
```"#;
/// `abi_function` body for `term::is_on` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_is_on(
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
        ctx.mouse_state_offset,
        ctx.input_state_offset,
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
        name: "isOn",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_is_on),
        }],
    });
}
