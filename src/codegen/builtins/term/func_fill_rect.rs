//! `term::fillRect` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_fill_rect`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Fill a rectangular region with a block or shade glyph"#;

const DESC: &str = r#"`term::fillRect` fills every cell of a rectangular region with a block or shade
glyph chosen by the `term::FillStyle` enum, using the colours and attributes currently
in effect. The two points `(rowA, columnA)` and `(rowB, columnB)` are **opposite
corners**. Like every `term::` position they are written **row first, then
column**, both **zero-based** from the top-left, and the two corners may be given
in any order. It is the region-filling counterpart to `term::clear`
(which blanks the whole surface): use it to paint a panel background, highlight a
band, or draw solid/█ and shaded/░▒▓ areas. The fill is shown on the next
`term::sync`.

The region is **clamped to the surface**, so a rectangle that runs off an edge
fills only the on-screen part, and one entirely off the surface fills nothing. No
error is raised for an out-of-range request. Filling does not move the shadow
cursor.

`term::FillStyle` selects the glyph: `Filled` (█, solid), `Light` (░), `Medium` (▒),
`Dark` (▓), and the two quadrant patterns `Checker` (▚) and `CheckerAlt` (▞). The
shade variants read as translucent overlays at a glance; the solid block is opaque.

**Two app-mode gaps apply to this call** (see `mfb man term`). In a **Linux**
`--app` build it is not implemented and fills nothing; a Linux terminal is
unaffected. In a **Windows** `--app` build it fills the region but ignores
`fill`, painting the current background colour instead of the block or shade
glyph. The console backend on every platform, and macOS app mode, honour the
style.

The call is gated: while TUI mode is off it does nothing and reports no
error (in a Linux or Windows `mfb build --app` build the gate is
not enforced — see `mfb man term`)."#;

const EX: &str = r#"Paint a solid panel, then a lighter band inside it:

```
IMPORT term
IMPORT color

SUB main()
  term::on()
  term::setBackground(color::rgb(0, 0, 40))
  term::fillRect(term::FillStyle.Filled, 1, 2, 12, 30)
  term::fillRect(term::FillStyle.Light, 3, 4, 5, 28)
  term::sync()
  term::off()
END SUB
```

Fill the whole surface as a background wash:

```
IMPORT term
IMPORT color

SUB main()
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  term::fillRect(term::FillStyle.Medium, 0, 0, size.rows - 1, size.columns - 1)
  term::sync()
  term::off()
END SUB
```"#;

/// `abi_function` body for `term::fill_rect` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_fill_rect(
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
        name: "fillRect",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("FillStyle, Integer, Integer, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "fill",
                    desc: "The block or shade glyph stamped into every cell of the region.",
                    aliases: &[],
                    ty: ParameterType::named("FillStyle"),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "rowA",
                    desc: "Row of the first corner (zero-based, counting from 0 at the top). Clamped to the surface.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "columnA",
                    desc: "Column of the first corner (zero-based, counting from 0 at the left). Clamped to the surface.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "rowB",
                    desc: "Row of the opposite corner; may be less or greater than `rowA`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "columnB",
                    desc: "Column of the opposite corner; may be less or greater than `columnA`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fill_rect),
        }],
    });
}
