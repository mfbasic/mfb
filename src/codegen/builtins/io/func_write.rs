//! `io::write` — descriptor entry + authored docs, and the shared stdout write
//! emitter this file owns.
//!
//! `io` lowers through per-function `Body::abi_function` clean-room lowerings
//! (plan-101). Beyond the `write` member itself, this file owns the shared stdout
//! **write** emitter (`lower_io_write_helper`, its `emit_append_to_stdout_buffer`
//! buffer helper) and the shared `lower_write_family` adapter that
//! `io::print`/`io::printError`/`io::writeError` also dispatch through (they
//! `use super::func_write::lower_write_family`).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::stdout::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::term::grid as term_grid;
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

/// `abi_function` body for `io::write` — write to stdout with no trailing newline.
/// The `String` and `AttributedString` overloads share this one helper.
pub(crate) fn lower_write(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_write_family(builder, ctx, false, false, IO_APP_WRITE_SYMBOL, "io.write")
}

const INTRO: &str = r#"Write a `String` to standard output with no trailing newline"#;
const DESC: &str = r#"`io::write` writes `value` to standard output exactly as stored and adds nothing.
The text is treated as UTF-8 and emitted byte for byte, with no escaping and no
newline translation. An empty `String` writes nothing at all. It is the
newline-free counterpart of `io::print`, which is the same call with a trailing
LF appended.

Only `String` is accepted, and exactly one argument; there is no implicit
conversion, so convert other values first — for example with `toString`.

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write is a failure and raises
`ErrOutput`.

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer rather than written immediately, so it may
not be visible to an external reader until drained. The buffer is drained when it
fills, on `io::flush`, before any standard-input read, and at program exit —
which is why a prompt written with `io::write` still appears before a following
`io::readLine` even under buffering. While the program is in `term::` TUI mode,
standard output is retained rather than printed and nothing reaches the terminal
until `term::sync` presents the frame. Output goes to whatever is bound to
standard output: file descriptor 1 in a console program, and the application
transcript window in app mode (`mfb build --app`)."#;
const EX: &str = r#"Write a prompt on the same line as the answer:

```
IMPORT io

SUB main()
  io::write("Name: ")
  LET name AS String = io::readLine()
END SUB
```

Build a line from several pieces:

```
IMPORT io

SUB main()
  io::write("x=")
  io::write(toString(3))
  io::print("")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::abi_function(lower_write),
            },
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "The attributed text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                    aliases: &[],
                    ty: ParameterType::Named("AttributedString"),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::abi_function(lower_write),
            },
        ],
    });
}

// --- shared stdout write emitter + adapter (relocated from native/) ---

/// Shared `abi_function` body for the four stdout writers
/// `io::{print,write,printError,writeError}`, which differ only in target stream
/// (`stderr`) and whether a trailing newline is appended (`newline`). Console:
/// `lower_io_write_helper` (loops the `write(fd, …)`, TUI-shadow-grid-routed while
/// `term::` is active). App mode: the transcript-window write hook
/// (`emit_app_io_write_helper`). The string/attributed-string overloads share
/// this one helper (both pass a string-object pointer in arg 0), exactly as the
/// pre-migration `native_os_seam` slot did.
pub(crate) fn lower_write_family(
    builder: &mut CodeBuilder,
    ctx: &AbiCtx,
    stderr: bool,
    newline: bool,
    app_symbol: &str,
    text: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.build_mode.is_app() {
        // App mode: `bl` the standalone transcript-write GUI helper (emitted in
        // `builder/mod.rs` with this member's stderr/newline baked in).
        builder.instructions.push(abi::branch_link(app_symbol));
        builder
            .relocations
            .push(internal_branch(&symbol, app_symbol));
        builder.instructions.push(abi::return_());
    } else {
        let (instructions, relocations, frame_size) = emit_write_body(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            stderr,
            newline,
            ctx.term_state_offset,
        )?;
        builder.instructions.extend(instructions);
        builder.relocations.extend(relocations);
        builder.stack_size = frame_size;
    }
    Ok(ValueResult {
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: text.to_string(),
    })
}

fn emit_append_to_stdout_buffer(
    ctx: &mut EmitCtx,
    src: &str,
    len: &str,
    tag: &str,
    write_error: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let cap = OUT_BUFFER_CAPACITY.to_string();
    let v0 = vregs.next();
    let v1 = vregs.next();
    let v2 = vregs.next();
    let v3 = vregs.next();
    let v4 = vregs.next();
    let v5 = vregs.next();
    let v6 = vregs.next();
    let v7 = vregs.next();
    let v8 = vregs.next();
    let sink = BufferSink {
        state_reg: ARENA_STATE_REGISTER,
        buf_ptr_off: ARENA_OUT_PTR_OFFSET,
        filled_off: ARENA_OUT_FILLED_OFFSET,
        drain_symbol: STDOUT_DRAIN_SYMBOL,
        drain_handle: None,
        cap: &cap,
        prefix: "buf",
        v: [
            v0.as_str(),
            v1.as_str(),
            v2.as_str(),
            v3.as_str(),
            v4.as_str(),
            v5.as_str(),
            v6.as_str(),
            v7.as_str(),
            v8.as_str(),
        ],
        fd: None,
    };
    emit_append_to_buffer(ctx, src, len, tag, write_error, &sink)
}

/// Emit the console stdout/stderr write vreg body (pre-finalization): returns
/// `(instructions, relocations, frame_size)`; the caller splices it in and the
/// `abi_function` wrapper finalizes.
fn emit_write_body(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
    append_newline: bool,
    term_state_offset: Option<usize>,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>, usize), String> {
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    // plan-35-B: while TUI mode is on, stdout writes mutate the shadow grid's back
    // buffer instead of the terminal (the mirror of app mode's `active`-gated grid
    // routing). Only stdout (not stderr) is retained, and only when the program
    // uses `term::` (`term_state_offset` is `Some`) — so a non-term program's
    // `io::write` is byte-identical. The grid path is emitted just before `done`.
    let grid_path = format!("{symbol}_grid");
    // The String object arrives in the return register. Capture it into a vreg
    // that stays live across the active-check branch: the check's own load may be
    // allocated into the return register (rax on x86), clobbering the pointer
    // before the grid path reads it — so save it here and restore the return
    // register for the fall-through (non-TUI) path.
    let strobj_vreg = vregs.next();
    let grid_target = if let Some(tso) = term_state_offset.filter(|_| !stderr) {
        let v29 = vregs.next();
        instructions.push(abi::move_register(&strobj_vreg, abi::return_register()));
        instructions.push(abi::load_u64(
            &v29,
            ARENA_STATE_REGISTER,
            tso + TERM_STATE_ACTIVE_OFFSET,
        ));
        instructions.push(abi::compare_immediate(&v29, "0"));
        instructions.push(abi::branch_ne(&grid_path));
        instructions.push(abi::move_register(abi::return_register(), &strobj_vreg));
        Some(tso)
    } else {
        None
    };
    // Opt-in stdout buffering (plan-14-A): stderr is never buffered, so only the
    // stdout helper gets the prologue. When `OUT_ENABLED == 0` (the default) fall
    // straight through to the unbuffered direct-write path below, byte-identical
    // to pre-plan-14; when enabled, append into the per-arena buffer instead.
    if !stderr {
        let direct = format!("{symbol}_direct");
        let write_error = format!("{symbol}_write_error");
        let v18 = vregs.next();
        let v19 = vregs.next();
        let v17 = vregs.next();
        instructions.extend([
            abi::load_u64(&v18, ARENA_STATE_REGISTER, ARENA_OUT_ENABLED_OFFSET),
            abi::compare_immediate(&v18, "0"),
            abi::branch_eq(&direct),
            // Capture the source pointer/length in vregs before any call clobbers x0.
            abi::load_u64(&v19, abi::return_register(), 0),
            abi::add_immediate(&v17, abi::return_register(), 8),
        ]);
        emit_append_to_stdout_buffer(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &v17,
            &v19,
            "line",
            &write_error,
            &mut vregs,
        )?;
        if append_newline {
            let v16 = vregs.next();
            instructions.extend([
                abi::move_immediate(&v16, "Integer", "10"),
                abi::store_u8(&v16, abi::stack_pointer(), 0),
                abi::add_immediate(&v17, abi::stack_pointer(), 0),
                abi::move_immediate(&v19, "Integer", "1"),
            ]);
            emit_append_to_stdout_buffer(
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                &v17,
                &v19,
                "newline",
                &write_error,
                &mut vregs,
            )?;
        }
        instructions.extend([
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            // The buffered success exit reuses the shared `done` epilogue emitted
            // below (the direct path lands there too), and any drain/write failure
            // above already branched to the shared `write_error` label.
            abi::branch(&format!("{symbol}_done")),
            abi::label(&direct),
        ]);
    }
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let fd_str = if stderr { "2" } else { "1" };
    let direct_loop = format!("{symbol}_direct_loop");
    let direct_written = format!("{symbol}_direct_written");
    // Loop on short writes (bug-51): a single write() may transfer fewer than the
    // string's byte count (pipe/FIFO, filling disk, signal); advance the cursor and
    // retry until nothing remains. A 0 or -1 return is a write failure, never
    // success. %v13/%v14 (cursor/remaining) are vregs, so the allocator spills them
    // across each `bl write` and reloads them afterward (compiler.md register
    // lifetimes) — the pointer/count are never read from a caller-saved register.
    let v14 = vregs.next();
    let v13 = vregs.next();
    instructions.extend([
        abi::load_u64(&v14, abi::return_register(), 0),
        abi::add_immediate(&v13, abi::return_register(), 8),
        abi::label(&direct_loop),
        abi::compare_immediate(&v14, "0"),
        abi::branch_eq(&direct_written),
        abi::move_register(abi::string_data_register(), &v13),
        abi::move_register(abi::string_length_register(), &v14),
        abi::move_immediate(abi::return_register(), "Integer", fd_str),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_transfer_loop_tail(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::return_register(),
        write_uses_raw_syscall(platform),
        &v13,
        &v14,
        &direct_loop,
        &write_error,
    )?;
    instructions.push(abi::label(&direct_written));
    if append_newline {
        let newline_loop = format!("{symbol}_newline_loop");
        let newline_written = format!("{symbol}_newline_written");
        let v9 = vregs.next();
        instructions.extend([
            abi::move_immediate(&v9, "Integer", "10"),
            abi::store_u64(&v9, abi::stack_pointer(), 8),
            abi::add_immediate(&v13, abi::stack_pointer(), 8),
            abi::move_immediate(&v14, "Integer", "1"),
            // A 1-byte write cannot short-count positively, but a 0 return still
            // means the byte was not written — loop and treat 0/-1 as a failure.
            abi::label(&newline_loop),
            abi::compare_immediate(&v14, "0"),
            abi::branch_eq(&newline_written),
            abi::move_register(abi::string_data_register(), &v13),
            abi::move_register(abi::string_length_register(), &v14),
            abi::move_immediate(abi::return_register(), "Integer", fd_str),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        emit_transfer_loop_tail(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            abi::return_register(),
            write_uses_raw_syscall(platform),
            &v13,
            &v14,
            &newline_loop,
            &write_error,
        )?;
        instructions.push(abi::label(&newline_written));
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
    ]);
    raise_error_into(
        symbol,
        "ErrWriteFailed",
        &mut instructions,
        &mut relocations,
    );
    if let Some(tso) = grid_target {
        // TUI-active stdout: route the string (still in the return register) into
        // the shadow-grid back buffer. No terminal write happens here; the frame
        // is shown when the program calls `term::sync`.
        instructions.push(abi::label(&grid_path));
        term_grid::emit_grid_write(
            symbol,
            tso,
            &strobj_vreg,
            append_newline,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        instructions.push(abi::branch(&done));
    }
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, 16))
}
