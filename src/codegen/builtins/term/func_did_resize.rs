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

A resize is noticed in both places a surface can live. In a terminal,
`term::sync` re-reads the size on each frame, so a change is picked up the next
time you present. In an `--app` build, the window reports its own size changes,
so live window resizes are reported the same way.

Like `term::isOn`, this query is **not gated**: it reads state only and touches
neither the terminal nor the surface. Before any `term::on` — or on a
fixed-size app window that never resizes — it simply reads `FALSE`. A companion `term::terminalSize` call returns the new extent after
`term::didResize` reports a change. In an `mfb build --app` program, `term::didResize` — like every other `term::` call — raises `ErrWrongMode` unless the program is in `app::Mode.Console`; see `mfb man app`."#;

const EX: &str = r#"Reflow a layout only when the terminal changes size. The check
belongs in a frame loop, after the frame is presented: `term::sync` is what
re-reads the terminal's size.

```
IMPORT os
IMPORT term

SUB main()
  term::on()
  MUT frames AS Integer = 0
  WHILE frames < 20
    term::clear()
    term::drawText(0, 0, "frame " & toString(frames))
    term::sync()
    os::sleep(100)
    IF term::didResize() THEN
      term::clear()
      term::drawText(0, 0, "reflowed")
      term::sync()
    END IF
    frames = frames + 1
  END WHILE
  term::off()
END SUB
```

Read the new extent on the frame that reports the resize:

```
IMPORT io
IMPORT os
IMPORT term

SUB main()
  term::on()
  MUT size AS term::TermSize = term::terminalSize()
  MUT frames AS Integer = 0
  WHILE frames < 20
    term::clear()
    term::drawText(0, 0, toString(size.columns) & "x" & toString(size.rows))
    term::sync()
    os::sleep(100)
    IF term::didResize() THEN
      size = term::terminalSize()
    END IF
    frames = frames + 1
  END WHILE
  term::off()
  io::print("last seen " & toString(size.columns) & "x" & toString(size.rows))
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
        name: "didResize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(lower_did_resize),
        }],
    });
}
