//! `term::terminalSize` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_terminal_size`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report the current size of the terminal surface as a `term::TermSize`"#;

const DESC: &str = r#"`term::terminalSize` returns the size of the drawing surface as a
`term::TermSize` record with two `Integer` fields: `columns`, the width in
character cells, and `rows`, the height. Both are counts of whole cells, never
pixels. Valid cursor positions are rows `0` through `rows-1` and columns `0`
through `columns-1`. It takes no arguments.

**This is the one `term::` read that is not silently inert while TUI mode is
off.** There is no meaningful default size to report, so calling it before
`term::on` or after `term::off` raises `ErrUnsupported` rather than returning
something invented. Guard with `term::isOn` if the call site may run outside TUI
mode.

While TUI mode is on, the size is read live from the terminal, so it reflects
the terminal as it is at the moment of the call. If the terminal cannot say —
standard output is not a terminal, or the host does not answer — or if it
reports zero rows or zero columns, the call raises
`ErrUnsupported`.

Because the query is live, the answer can change between calls when the user
resizes the window. A program that lays out, centres, or bounds-checks against
these dimensions should ask again rather than cache the first answer. The
drawing surface itself is resized by `term::sync`, which re-reads the terminal
on entry and, when the size changed, resizes the surface keeping the top-left
overlap and repaints in full — so immediately after a resize and before the next
`term::sync`, this call can report the new size while the surface is still the
old one.

The call has no side effects: it draws nothing, moves no cursor, and changes no
`term::` state. Besides `ErrUnsupported` it can raise `ErrOutOfMemory` while
producing its result.

In app mode (`mfb build --app`) the size comes from the application's terminal
view rather than from the console, and the same `ErrUnsupported` is raised when TUI mode
is off or no view is attached. **A Windows `--app` build is the exception on both
counts**: its surface is a fixed **80 columns by 25 rows** that does not follow the
window, so this call always reports that size, `term::didResize` always reads
`FALSE`, and — unlike every other backend — it reports the size rather than
raising `ErrUnsupported` when TUI mode is off. A program that must work on every
backend should treat `ErrUnsupported` as possible and not depend on getting it."#;

const EX: &str = r#"Report the surface dimensions:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  LET size AS term::TermSize = term::terminalSize()
  term::off()
  io::print(toString(size.columns) & "x" & toString(size.rows))
END SUB
```

Draw near the centre of the surface:

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
/// `abi_function` body for `term::terminal_size` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_terminal_size(
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
        name: "terminalSize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("TermSize"),
            errors: vec![],
            body: Body::abi_function(lower_terminal_size),
        }],
    });
}
