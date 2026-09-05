//! `process::shell` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor and those entry fns.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use std::collections::HashMap;

use super::gen_shared::ProcBodyParts;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_unix::*;
use super::gen_windows::*;
const INTRO: &str =
    r#"Run a command line through the platform shell, returning a handle to the child."#;
const DESC: &str = r#"`process::shell` runs `cmd` as a shell command line and returns a `Process`
handle to the resulting child. Unlike `process::spawn`, which execs a program
directly, `shell` hands the string to the platform shell, so shell features work:
pipelines (`|`), redirection (`>`, `<`), globbing, command sequencing, quoting,
and environment-variable expansion are all interpreted by the shell.

Which shell depends on the platform: `/bin/sh -c` on Unix, `cmd.exe /S /C` on
Windows. The string is handed over unchanged — `shell` does not translate between
dialects, so a command line has to be written for the shell that will read it.
`echo hi | sort` works on both; `ls -l` and `dir` do not. When a program must run
on both and the command is simple, `process::spawn` avoids the question entirely.

On Windows a line is wrapped in quotes before `cmd` sees it and `/S` makes `cmd`
strip exactly that wrap, so a command line containing quotes — or starting with
one — reaches the shell intact.

`process::receive` returns each line as the child wrote it. Windows programs end
their lines `\r\n`, so a line read from a Windows child keeps its trailing `\r`;
trim it if the value is compared or printed inline.


Because the string is parsed by a shell, values interpolated into `cmd` are subject
to shell word-splitting and metacharacter interpretation; build the command with
care when any part comes from untrusted input. When you do not need a shell — you
have a program and its arguments already separated — prefer `process::spawn`, which
avoids the shell entirely.

The child is wired to three pipes for its standard streams exactly as with
`process::spawn`, and the returned handle behaves the same way: it closes itself
when its binding goes out of scope, force-killing and reaping a still-running child unless
it is first awaited with `process::waitFor` or ended with `process::detach`."#;
const EX: &str = r#"Run a pipeline and read the result. `echo` and `sort` are spelled the
same way in both shells, so this line runs unchanged on Unix and on Windows:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo hello | sort")
  io::print(process::receive(sh))
  RETURN 0
END FUNC
```

Run a command and wait for its exit code:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("exit 0")
  io::print(toString(process::waitFor(sh)))
  RETURN 0
END FUNC
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::shell` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_shell(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        if ctx.platform.family() == crate::codegen::engine::types::PlatformFamily::Windows {
            lower_process_shell_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        } else {
            lower_process_shell_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "shell",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "cmd",
                desc: "The command line to run through the platform shell (`/bin/sh -c` on Unix). Also accepts the alternate named-argument spelling `command`.",
                aliases: &["command"],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(super::PROCESS_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_shell),
        }],
    });
}

pub(crate) fn lower_process_shell_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    const LOCAL: usize = SPAWN_LOCAL;
    let mut v = Vregs::new();
    let cmdstr = v.next();
    let argv = v.next();
    let cstr = v.next();
    let srcp = v.next();
    let dstp = v.next();
    let len = v.next();
    let j = v.next();
    let byte = v.next();
    let alloc_fail = format!("{symbol}_alloc_fail");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let fork_fail = format!("{symbol}_fork_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::move_register(&cmdstr, abi::return_register()),
        // argv = alloc(4*8, 8)  ["/bin/sh", "-c", cmd, NULL]
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ];
    let mut relocations = Vec::new();
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::move_register(&argv, abi::mfb_return(1)));
    // argv[0] = "/bin/sh"
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "8"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::move_register(&cstr, abi::mfb_return(1)));
    emit_cstring_literal("/bin/sh", &cstr, &byte, &mut instructions);
    instructions.push(abi::store_u64(&cstr, &argv, 0));
    // argv[1] = "-c"
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "3"),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::move_register(&cstr, abi::mfb_return(1)));
    emit_cstring_literal("-c", &cstr, &byte, &mut instructions);
    instructions.push(abi::store_u64(&cstr, &argv, 8));
    // argv[2] = cmd (copy the String's bytes into a fresh NUL-terminated cstr)
    instructions.extend([
        abi::load_u64(&len, &cmdstr, 0),
        abi::add_immediate(abi::return_register(), &len, 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&cstr, abi::mfb_return(1)),
        abi::add_immediate(&srcp, &cmdstr, 8),
        abi::move_register(&dstp, &cstr),
        abi::move_immediate(&j, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&j, &len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&byte, &srcp, 0),
        abi::store_u8(&byte, &dstp, 0),
        abi::add_immediate(&srcp, &srcp, 1),
        abi::add_immediate(&dstp, &dstp, 1),
        abi::add_immediate(&j, &j, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &dstp, 0),
        abi::store_u64(&cstr, &argv, 16),
        // argv[3] = NULL
        abi::store_u64(abi::ZERO, &argv, 24),
    ]);
    emit_spawn_tail(
        symbol,
        &mut v,
        &argv,
        None,
        None,
        &alloc_fail,
        &fork_fail,
        &done,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&fork_fail));
    emit_fail(
        symbol,
        "ErrSpawnFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, LOCAL))
}

/// `process::shell` on Windows (plan-119-B): run the line through
/// `cmd.exe /S /C "<line>"` and hand the rest to the shared spawn tail.
///
/// The Windows twin of `lower_process_shell_helper_posix`, and the same shape:
/// build the shell invocation, then call the tail. Where posix builds an argv
/// (`["/bin/sh", "-c", cmd]`) because `execvp` takes a vector, Windows builds one
/// string because `CreateProcessA` takes one.
///
/// `cmd.exe` is named without a path: `CreateProcessA` with a NULL
/// `lpApplicationName` resolves it through the system directory, which is
/// box-verified — no `COMSPEC` read is involved.
///
/// **`/S` is load-bearing.** Without it `cmd` applies a two-branch heuristic to
/// decide whether to keep or strip the quotes around the command (it inspects the
/// quote count, the characters between them, and whether the quoted text names an
/// executable). With `/S` it always takes the simple branch: strip the leading
/// quote and the final quote, and run everything in between. That makes a line
/// which itself contains — or starts with — a quote behave predictably, and
/// `scripts/test-winprocess.sh` pins exactly that case.
///
/// Frame: the shared depth-1, no-vreg frame described on `WIN_SPAWN_SI`. The four
/// scratch slots start at `WIN_SPAWN_SCRATCH`; the command-line *builder*'s slots
/// are not used here, since there is no argv list to walk.
pub(crate) fn lower_process_shell_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    /// `cmd.exe /S /C "` — the closing `"` and the NUL are appended after the line.
    const PREFIX: &str = "cmd.exe /S /C \"";
    const QUOTE: &str = "34";
    /// The MFB `String` object (length@0, bytes@8).
    const SRC: usize = WIN_SPAWN_SCRATCH;
    /// Its byte length.
    const LEN: usize = WIN_SPAWN_SCRATCH + 0x08;
    /// Write cursor into the command line.
    const DP: usize = WIN_SPAWN_SCRATCH + 0x10;
    const FRAME: usize = 0x200; // covers SRC..DP, 16-aligned
    const _: () = assert!(FRAME >= WIN_SPAWN_SCRATCH + 0x18 && FRAME % 16 == 0);
    let sp = abi::stack_pointer();

    let alloc_fail = format!("{symbol}_alloc_fail");
    let spawn_fail = format!("{symbol}_spawn_fail");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");

    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::subtract_stack(FRAME),
        // The command String arrives in the return register.
        abi::store_u64(abi::return_register(), sp, SRC),
        abi::load_u64(abi::mfb_arg(0), sp, SRC),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u64(abi::mfb_arg(2), sp, LEN),
        // cmd = arena_alloc(PREFIX + len + closing quote + NUL, align 1)
        abi::add_immediate(abi::return_register(), abi::mfb_arg(2), PREFIX.len() + 2),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ];
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, WIN_SPAWN_CMD),
        abi::move_register(abi::mfb_arg(1), abi::mfb_return(1)),
    ]);
    // The literal prefix, byte by byte (no vreg is available for a helper here).
    for (offset, ch) in PREFIX.bytes().enumerate() {
        instructions.push(abi::move_immediate(
            abi::mfb_arg(2),
            "Integer",
            &ch.to_string(),
        ));
        instructions.push(abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), offset));
    }
    instructions.extend([
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), PREFIX.len()),
        abi::store_u64(abi::mfb_arg(1), sp, DP),
        // Copy the String's bytes in verbatim — a shell command line is *supposed*
        // to be interpreted, so nothing here is quoted or escaped beyond the wrap.
        abi::load_u64(abi::mfb_arg(0), sp, SRC),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 8),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(2), sp, LEN),
        abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
        abi::branch_eq(&copy_done),
        abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        // closing wrap quote + NUL
        abi::move_immediate(abi::mfb_arg(2), "Integer", QUOTE),
        abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0),
        // `shell` has no cwd or environment of its own: the child inherits both.
        abi::store_u64(abi::ZERO, sp, WIN_SPAWN_ENV),
        abi::store_u64(abi::ZERO, sp, WIN_SPAWN_CWD),
    ]);
    emit_win_spawn_tail(
        symbol,
        &alloc_fail,
        &spawn_fail,
        &done,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&spawn_fail));
    emit_fail(
        symbol,
        "ErrSpawnFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    Ok((instructions, relocations, 0))
}
