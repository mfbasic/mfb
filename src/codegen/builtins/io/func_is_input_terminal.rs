//! `io::isInputTerminal` — descriptor entry + authored docs, and the shared
//! terminal-predicate emitter this file owns.
//!
//! `io` lowers through per-function `Body::abi_function` clean-room lowerings
//! (plan-101). Beyond the `isInputTerminal` member itself, this file owns the
//! shared terminal-predicate emitter (`emit_is_terminal_body`) and the
//! `lower_is_terminal_common` adapter that `io::isOutputTerminal`/`io::isErrorTerminal`
//! also dispatch through (they
//! `use super::func_is_input_terminal::lower_is_terminal_common`).

use super::app_unsupported;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

/// `abi_function` body for `io::isInputTerminal` — `isatty(0)` (fd 0).
pub(crate) fn lower_is_input_terminal(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_is_terminal_common(builder, ctx, 0, "io.isInputTerminal")
}

const INTRO: &str = r#"Report whether standard input is an interactive terminal"#;
const DESC: &str = r#"`io::isInputTerminal` returns `TRUE` when standard input is connected to a
terminal and `FALSE` when it is redirected from a file, a pipe, or any other
non-terminal source. It takes no arguments.

The answer comes from an `isatty` probe of file descriptor 0: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.

The probe inspects state only. It does not modify the stream, consume any input,
or block waiting for data, so it is safe to call before deciding whether to
prompt interactively, enable line editing, or read a piped stream straight
through. In app mode the program has no real standard streams — input is served by
the application window, which is treated as an interactive console — so this call
returns `TRUE` without probing a descriptor."#;
const EX: &str = r#"Prompt only when a human is attached, otherwise read the piped stream:

```
IMPORT io

SUB main()
  IF io::isInputTerminal() THEN
    io::print(io::input("Name: "))
  ELSE
    io::print(io::readLine())
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isInputTerminal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_input_terminal),
        }],
    });
}

// --- shared terminal-predicate emitter + adapter (relocated from native/) ---

/// Shared `abi_function` body for the three terminal predicates
/// `io::is{Input,Output,Error}Terminal`, which differ only in the probed file
/// descriptor (`fd`) and the result label (`text`). Console: `isatty(fd)` via
/// `emit_is_terminal_body`. App mode: the window is the interactive
/// console, so all three return `TRUE` (`emit_app_io_is_terminal`).
pub(crate) fn lower_is_terminal_common(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    fd: u8,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.build_mode.is_app() {
        // App mode: the window is the interactive console — the platform hook
        // appends `RESULT = TRUE/OK` directly into the builder's vreg stream.
        ctx.platform
            .emit_app_io_is_terminal(&symbol, &mut builder.instructions, &mut builder.relocations)
            .ok_or_else(|| app_unsupported(ctx.platform))??;
    } else {
        // Console: `isatty(fd)` vreg body spliced in; the wrapper finalizes.
        let (instructions, relocations, frame_size) =
            emit_is_terminal_body(&symbol, ctx.platform_imports, ctx.platform, fd)?;
        builder.instructions.extend(instructions);
        builder.relocations.extend(relocations);
        builder.stack_size = frame_size;
    }
    Ok(ValueResult {
        type_: "Boolean".to_string(),
        location: Operand::from("void"),
        text: text.to_string(),
    })
}

/// Emit the console `isatty(fd)` vreg body (pre-finalization): returns
/// `(instructions, relocations, frame_size)`; the caller splices it into the
/// builder and the `abi_function` wrapper finalizes.
fn emit_is_terminal_body(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    fd: u8,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>, usize), String> {
    const FRAME_SIZE: usize = 16;
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        &fd.to_string(),
    ));
    platform.emit_is_terminal(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
    ]);
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}
