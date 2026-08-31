//! `term::sync` — abi_function member (native terminal I/O).
//!
//! Registers its own [`lower_sync`] `Body::abi_function`
//! body; the `abi_function` wrapper finalizes it. The heavy terminal emission stays
//! in the shared code layer (`code::lower_term_helper` / `emit_app_term_helper`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Present the composed frame — the only call that puts drawing on screen"#;

const DESC: &str = r#"The `term::` surface is **retained and double-buffered**. While TUI mode is on,
every drawing call — `io::print` and `io::write`, and `term::clear`,
`term::moveTo`, the colour and attribute setters, `term::showCursor` and
`term::hideCursor` — mutates an in-memory cell grid and the current-attribute
state, and touches the terminal not at all. `term::sync` is the one and only
operation that presents a frame. **A program that draws without calling
`term::sync` shows nothing** — this is the single most common mistake when
writing against this surface.

The present works by diffing. Each cell of the back buffer is compared against
the front buffer holding what was last shown, and only the cells that differ are
emitted — as a minimal stream of cursor moves, colour and attribute changes, and
glyphs, coalesced so an unchanged attribute is not re-sent. The whole stream is
built into a scratch buffer and issued as a **single batched write**, not a write
per cell, and each emitted cell is copied back to front so the next present diffs
from the frame actually on screen. The result is that repainting every frame
produces output proportional to what changed, and shows no flicker.

That write is looped to completion. A short write advances the cursor and
re-issues until every byte lands, which matters here more than elsewhere: the
front buffer already claims those cells were painted, so a partially written
frame that was not finished would leave the screen wrong until something else
happened to mark the cells dirty again.

**A terminal resize is handled here.** On entry `term::sync` re-reads the
terminal size; if it changed, it resizes the surface, keeps the content that
still fits in the top-left, clamps the cursor into the new bounds, and repaints
the next frame in full. If the size cannot be re-read or the surface cannot be
resized, the old surface is kept and the frame is presented into it unchanged.

After the changed cells, the present emits a trailing sequence that resets
attributes, moves the terminal cursor to the shadow cursor's position, and shows
or hides it according to `term::showCursor`/`term::hideCursor`. The first present
after `term::on` (or after a resize) is a full repaint because the grid is marked
dirty.

`term::sync` is gated: while TUI mode is off it is a clean no-op, so calling it
before `term::on` or after `term::off` is harmless. `term::off` performs a final
present of its own, so the last frame a program draws is always shown even
without an explicit `term::sync`. In app mode the call requests a single
coalesced redraw of the terminal view."#;

const EX: &str = r#"Compose a frame, then present it once:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  term::clear()
  term::moveTo(0, 0)
  io::print("hello")
  term::sync()
  term::off()
END SUB
```

The canonical render loop — draw the whole frame, present, then read input:

```
IMPORT term
IMPORT io

SUB main()
  term::on()
  MUT running AS Boolean = TRUE
  WHILE running
    term::clear()
    term::moveTo(0, 0)
    io::print("press q to quit")
    term::sync()
    IF io::pollInput(50) THEN
      running = io::readChar() <> "q"
    END IF
  END WHILE
  term::off()
END SUB
```"#;
/// `abi_function` body for `term::sync` — delegates to the shared family-generic
/// [`super::gen_shared::lower_term_helper`] with its own runtime-call name (the
/// app-vs-console dispatch and the heavy per-member emitters live in the shared code
/// layer).
pub(crate) fn lower_sync(
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
        name: "sync",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_sync),
        }],
    });
}
