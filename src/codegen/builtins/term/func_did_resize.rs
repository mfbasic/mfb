//! `term::didResize` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_did_resize`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether the terminal was resized since the last check"#;

const DESC: &str = r#"`term::didResize` returns `TRUE` when the terminal (CLI) or window (`--app`) has
changed size since the last time `term::didResize` was called, and `FALSE`
otherwise. The flag is **cached**: once a resize is detected it stays `TRUE`
across every intervening call until `term::didResize` observes it, so a program
that only polls occasionally never misses a resize. Reading it clears it, so the
very next call reports `FALSE` unless another resize has happened in between.

The resize is detected wherever the surface tracks its own geometry:

- In the CLI backend the shadow-grid present (`term::sync`) re-reads the terminal
  size each frame and reflows the grid; a genuine change latches the flag.
- In `--app` mode each window backend records the change in its own resize hook —
  macOS in the `setFrameSize:` view callback, and Linux/GTK in the drawing area's
  `resize` signal — so `term::didResize` reports live window resizes too.

Like `term::isOn`, this query is **not gated**: it reads state only and never
touches the terminal, the alternate screen, or the shadow grid. Before any
`term::on` — or on a fixed-size app surface that never reflows — it simply reads
`FALSE`. A companion `term::terminalSize` call returns the new extent after
`term::didResize` reports a change."#;

const EX: &str = r#"Reflow a layout only when the terminal changes size:

```
IMPORT term

SUB main()
  term::on()
  IF term::didResize() THEN
    term::clear()
    term::sync()
  END IF
  term::off()
END SUB
```

Re-read the new extent after a resize:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  IF term::didResize() THEN
    LET size = term::terminalSize()
    io::print("resized to " & toString(size.columns) & "x" & toString(size.rows))
  END IF
  term::off()
END SUB
```"#;
/// `abi_function` body for `term::did_resize` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_did_resize(
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
        name: "didResize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_did_resize),
        }],
    });
}
