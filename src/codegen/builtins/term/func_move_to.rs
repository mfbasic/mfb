//! `term::moveTo` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_move_to`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Move the cursor to a row and column of the surface"#;

const DESC: &str = r#"`term::moveTo` sets the position at which the next text drawn through the
`term::` surface — including `io::print` and `io::write` — will start. Coordinates
are **zero-based** and measured from the top-left corner: row 0 is the topmost
line, column 0 is the leftmost column, and `(0, 0)` is the home position. Like
every other `term::` position, the arguments are written **row first, then
column** — the surface is a grid of character cells, never pixels, so there are no
`x`/`y` coordinates anywhere in this package.

**Both coordinates are clamped at both ends**, on every backend. A negative value
becomes 0, and a value at or past the edge becomes the last valid cell — `rows-1`
for the row, `columns-1` for the column, using the current surface dimensions that
`term::terminalSize` reports. The cursor can therefore never be placed outside the
grid, and no error is raised for an out-of-range request.

The move is buffered, like everything else on this surface: it records the new
cursor position and sends nothing to the terminal. The position is
honoured by the next glyph written and by the frame `term::sync` presents.
Moving the cursor draws nothing, erases nothing, and leaves the colours and
attributes alone.

Drawing advances the cursor on its own: each glyph moves it one column right,
wrapping to column 0 of the next row at the right edge and scrolling the surface
up by one row at the bottom. A line feed in the drawn text moves to column 0 of
the next row, a carriage return moves to column 0 of the same row, and
`io::print`'s trailing newline advances a row as well. `term::clear` homes the
cursor to (0, 0).

The call is gated: while TUI mode is off it does nothing and reports no
error (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`)."#;

const EX: &str = r#"Draw at the top-left corner:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()
  term::moveTo(0, 0)
  io::print("top-left")
  term::sync()
  term::off()
END SUB
```

Draw near the middle of the surface:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  term::moveTo(size.rows / 2, size.columns / 2)
  io::write("middle")
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::move_to` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_move_to(
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
        name: "moveTo",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "row",
                    desc: "Zero-based row, counting from 0 at the top. Clamped to `0` at the low end and to `rows-1` at the high end.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "column",
                    desc: "Zero-based column, counting from 0 at the left. Clamped to `0` at the low end and to `columns-1` at the high end.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_move_to),
        }],
    });
}
