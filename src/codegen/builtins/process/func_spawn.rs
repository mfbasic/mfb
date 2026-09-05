//! `process::spawn` — registry entry.
//!
//! Per-member file. `Body::abi_function`: the member's OS-seam body branches on OS
//! family (libc vs kernel32) and delegates to the arch-neutral emission in
//! `../native/{unix,windows}`; the shared `lower_abi_function_helper` wraps it once
//! into the `_mfb_rt_*` helper. This file carries the registry entry, that body, and
//! the docs.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use std::collections::HashMap;

use crate::codegen::error::emission::emit_fail;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_shared::*;
use super::gen_unix::*;
use super::gen_windows::*;
const INTRO: &str =
    r#"Run a program directly from an argument list, returning a handle to the child."#;
const DESC: &str = r#"`process::spawn` starts a child process from an explicit argument vector and
returns a `Process` handle to it. `args[0]` is the executable and is
resolved on `PATH` (`execvp` on Unix); the remaining elements are passed as the
child's arguments verbatim. **No shell is involved** — quoting, globbing, pipes,
redirection, and environment-variable expansion are *not* interpreted, so an
argument that contains spaces or shell metacharacters reaches the program as one
literal argument. Use `process::shell` when you need a shell to parse a command
line.

Windows hands a child one command line instead of a vector, so on Windows
`spawn` builds that line and quotes each element the way the child's C runtime
will un-quote it: an element containing a space, a tab, or a `"` is wrapped in
quotes, an empty element becomes `""`, and backslashes are doubled exactly where
the runtime would otherwise read them as escapes. A child that splits its command
line the standard way — which includes every program built against a C runtime,
and anything using `CommandLineToArgvW` — therefore parses back the same vector
you passed, so the "one literal argument" promise above holds on Windows too. A
program that parses its own command line instead (`cmd.exe` is the notable one)
sees the quoted form.

The child is created with three pipes wired to its standard input, output, and
error, so the parent can `process::send` to it and `process::receive` from it.
Creation forks and execs; an exec failure in the child (for example a program that
is not found) is reported back to the parent over a close-on-exec self-pipe and
surfaces as `ErrSpawnFailed`, not as a silently running child.


The returned `Process` is a resource handle that cannot be copied. It closes
itself when its binding goes out of scope, which **force-kills and reaps** a
still-running child (`SIGKILL` + `waitpid` on Unix) so no runaway process or zombie
is left; call `process::waitFor` first if the child should be allowed to finish, or
`process::detach` to let it outlive the program.

The empty argument list is rejected with `ErrInvalidArgument` — there is no program
to run.


The four-argument form takes a working directory, an environment map, and a
replace flag, and works on every supported platform — but the two systems reach
the same result by different routes, and one detail is visible to a caller. Unix
applies both in the child after the fork (`chdir`, then `unsetenv`/`setenv`);
Windows builds a single environment block up front and hands it to the child,
because that is what `CreateProcess` accepts.

That makes the merge rule a real distinction. On Windows, environment names are
case-insensitive, so a map key overrides an inherited variable that differs only
in case: a map containing `path` replaces the inherited `Path`, and the child
gets one entry, not two. The comparison folds ASCII letters only. On Unix names
are case-sensitive, so `path` and `Path` are two different variables there.

With `envReplace` TRUE the child gets **only** the map, on both systems — the
inherited environment is not merged in and nothing is added back. On Windows a
child that is itself `cmd.exe` will still show a few variables of its own making
(`COMSPEC`, `PATHEXT`, `PROMPT`): those are synthesized by the shell after it
starts, not inherited, and a non-shell child sees only what the map contained."#;
const EX: &str = r#"Run a program and read its first line of output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  io::print(process::receive(child))
  RETURN 0
END FUNC
```

Run a program in a chosen directory with an extra environment variable merged in:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  LET env AS Map OF String TO String = Map OF String TO String { "GREETING" := "hi" }
  RES child = process::spawn(["printenv", "GREETING"], "/tmp", env, FALSE)
  io::print(process::receive(child))
  RETURN 0
END FUNC
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::spawn` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_spawn(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        if ctx.platform.family() == crate::codegen::engine::types::PlatformFamily::Windows {
            lower_process_spawn_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        } else {
            lower_process_spawn_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // Both overloads lower through the same posix/win emitters; the full form is
    // selected at codegen by argument count (`builder_values` → `process.spawnEnv`),
    // and each emitter branches on the runtime-call name internally.
    let args = Parameter {
        name: "args",
        desc: "The argument vector. `args[0]` is the executable, resolved on `PATH`; the rest are the child's arguments, passed literally with no shell interpretation. Must be non-empty.",
        aliases: &[],
        ty: ParameterType::list_of(ParameterType::String),
        default: DefaultValue::None,
    };
    pkg.add_function(RegistryFunction {
        name: "spawn",
        intro: INTRO,
        desc: DESC,
        example: EX,
        // The two overloads have structurally different positional layouts; the
        // per-position render only shows the first (`List OF String`). The
        // `"or"`-joined string names both forms (the net/audio overloaded idiom).
        expected_arguments: Some(
            "List OF String or List OF String, String, Map OF String TO String, Boolean",
        ),
        internal_only: false,
        implementations: vec![
            // Bare argv form.
            Implementation {
                params: vec![args.clone()],
                return_type: ParameterType::named(super::PROCESS_TYPE_ID),
                errors: vec![],
                    body: Body::abi_function_aliased(lower_spawn, &["spawnEnv"]),
            },
            // Full form: working directory + environment map + replace/merge flag.
            Implementation {
                params: vec![
                    args,
                    Parameter {
                        name: "cwd",
                        desc: "(full form) The working directory to switch to before running the child. An empty string leaves the parent's working directory in effect.",
                        aliases: &[],
                        ty: ParameterType::String,
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "env",
                        desc: "(full form) Environment variables for the child. On Windows a name matches an inherited one case-insensitively, so a key here replaces an inherited variable that differs only in case.",
                        aliases: &[],
                        ty: ParameterType::map_of(ParameterType::String, ParameterType::String),
                        default: DefaultValue::None,
                    },
                    Parameter {
                        name: "envReplace",
                        desc: "(full form) `TRUE` to run with *only* `env` (the inherited environment is cleared first); `FALSE` to merge `env` over the inherited environment.",
                        aliases: &[],
                        ty: ParameterType::Boolean,
                        default: DefaultValue::None,
                    },
                ],
                return_type: ParameterType::named(super::PROCESS_TYPE_ID),
                errors: vec![],
                    body: Body::abi_function_aliased(lower_spawn, &["spawnEnv"]),
            },
        ],
    });
}

pub(crate) fn lower_process_spawn_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    if call == "process.spawnEnv" {
        const LOCAL: usize = SPAWN_LOCAL;
        let mut v = Vregs::new();
        let args = v.next();
        let cwd_str = v.next();
        let env_map = v.next();
        let envrep = v.next();
        let cwdptr = v.next();
        let cwdlen = v.next();
        let cwdcstr = v.next();
        let n = v.next();
        let cap = v.next();
        let dbase = v.next();
        let argv = v.next();
        let i = v.next();
        let entry = v.next();
        let vlen = v.next();
        let srcp = v.next();
        let dstp = v.next();
        let cstr = v.next();
        let j = v.next();
        let byte = v.next();
        let tmp = v.next();
        let sp = v.next();
        let dp = v.next();
        let cnt = v.next();

        let invalid = format!("{symbol}_invalid_args");
        let alloc_fail = format!("{symbol}_alloc_fail");
        let cwd_copy = format!("{symbol}_cwd_copy");
        let cwd_copy_done = format!("{symbol}_cwd_copy_done");
        let build_loop = format!("{symbol}_argv_loop");
        let build_done = format!("{symbol}_argv_done");
        let copy_loop = format!("{symbol}_copy_loop");
        let copy_done = format!("{symbol}_copy_done");
        let fork_fail = format!("{symbol}_fork_fail");
        let done = format!("{symbol}_done");

        // Capture the four arguments (x0..x3) before any clobbering libc call.
        let mut instructions = vec![
            abi::move_register(&args, abi::return_register()),
            abi::move_register(&cwd_str, abi::c_arg(1)),
            abi::move_register(&env_map, abi::c_arg(2)),
            abi::move_register(&envrep, abi::c_arg(3)),
            abi::load_u64(&n, &args, LIST_COUNT),
            abi::compare_immediate(&n, "0"),
            abi::branch_eq(&invalid),
        ];
        let mut relocations = Vec::new();
        // cwd C string (empty cwd → "\0", whose leading NUL makes the child skip chdir).
        instructions.extend([
            abi::add_immediate(&cwdptr, &cwd_str, 8),
            abi::load_u64(&cwdlen, &cwd_str, 0),
        ]);
        emit_copy_to_cstring(
            symbol,
            &cwdptr,
            &cwdlen,
            &cwdcstr,
            &sp,
            &dp,
            &cnt,
            &byte,
            &cwd_copy,
            &cwd_copy_done,
            &alloc_fail,
            &mut instructions,
            &mut relocations,
        );
        // Build argv from the args list (same entry-array walk as spawn).
        instructions.extend([
            abi::load_u64(&cap, &args, LIST_CAP),
            abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
            abi::multiply_registers(&cap, &cap, &tmp),
            abi::add_immediate(&cap, &cap, LIST_HEADER),
            abi::add_registers(&dbase, &args, &cap),
            abi::add_immediate(&tmp, &n, 1),
            abi::move_immediate(&byte, "Integer", "8"),
            abi::multiply_registers(&tmp, &tmp, &byte),
            abi::move_register(abi::return_register(), &tmp),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register(&argv, abi::mfb_return(1)),
            abi::move_immediate(&i, "Integer", "0"),
            abi::label(&build_loop),
            abi::compare_registers(&i, &n),
            abi::branch_eq(&build_done),
            abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
            abi::multiply_registers(&entry, &i, &tmp),
            abi::add_immediate(&entry, &entry, LIST_HEADER),
            abi::add_registers(&entry, &args, &entry),
            abi::load_u64(&srcp, &entry, ENTRY_VOFF),
            abi::add_registers(&srcp, &dbase, &srcp),
            abi::load_u64(&vlen, &entry, ENTRY_VLEN),
            abi::add_immediate(abi::return_register(), &vlen, 1),
            abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register(&cstr, abi::mfb_return(1)),
            abi::move_immediate(&tmp, "Integer", "8"),
            abi::multiply_registers(&tmp, &i, &tmp),
            abi::add_registers(&tmp, &argv, &tmp),
            abi::store_u64(&cstr, &tmp, 0),
            abi::move_register(&dstp, &cstr),
            abi::move_immediate(&j, "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers(&j, &vlen),
            abi::branch_eq(&copy_done),
            abi::load_u8(&byte, &srcp, 0),
            abi::store_u8(&byte, &dstp, 0),
            abi::add_immediate(&srcp, &srcp, 1),
            abi::add_immediate(&dstp, &dstp, 1),
            abi::add_immediate(&j, &j, 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::store_u8(abi::ZERO, &dstp, 0),
            abi::add_immediate(&i, &i, 1),
            abi::branch(&build_loop),
            abi::label(&build_done),
            abi::move_immediate(&tmp, "Integer", "8"),
            abi::multiply_registers(&tmp, &n, &tmp),
            abi::add_registers(&tmp, &argv, &tmp),
            abi::store_u64(abi::ZERO, &tmp, 0),
        ]);
        emit_spawn_tail(
            symbol,
            &mut v,
            &argv,
            Some(&cwdcstr),
            Some((&env_map, &envrep)),
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
        instructions.push(abi::label(&invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
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
    } else {
        const LOCAL: usize = SPAWN_LOCAL;

        let mut v = Vregs::new();
        let list = v.next();
        let n = v.next();
        let cap = v.next();
        let dbase = v.next();
        let argv = v.next();
        let i = v.next();
        let entry = v.next();
        let vlen = v.next();
        let srcp = v.next();
        let dstp = v.next();
        let cstr = v.next();
        let j = v.next();
        let byte = v.next();
        let tmp = v.next();

        let invalid = format!("{symbol}_invalid_args");
        let alloc_fail = format!("{symbol}_alloc_fail");
        let build_loop = format!("{symbol}_argv_loop");
        let build_done = format!("{symbol}_argv_done");
        let copy_loop = format!("{symbol}_copy_loop");
        let copy_done = format!("{symbol}_copy_done");
        let fork_fail = format!("{symbol}_fork_fail");
        let done = format!("{symbol}_done");

        let mut instructions = vec![
            abi::move_register(&list, abi::return_register()),
            abi::load_u64(&n, &list, LIST_COUNT),
            abi::compare_immediate(&n, "0"),
            abi::branch_eq(&invalid),
            // dbase = list + HEADER + cap*ENTRY
            abi::load_u64(&cap, &list, LIST_CAP),
            abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
            abi::multiply_registers(&cap, &cap, &tmp),
            abi::add_immediate(&cap, &cap, LIST_HEADER),
            abi::add_registers(&dbase, &list, &cap),
            // argv = alloc((n+1)*8, 8)
            abi::add_immediate(&tmp, &n, 1),
            abi::move_immediate(&byte, "Integer", "8"),
            abi::multiply_registers(&tmp, &tmp, &byte),
            abi::move_register(abi::return_register(), &tmp),
            abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        ];
        let mut relocations = Vec::new();
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register(&argv, abi::mfb_return(1)),
            abi::move_immediate(&i, "Integer", "0"),
            abi::label(&build_loop),
            abi::compare_registers(&i, &n),
            abi::branch_eq(&build_done),
            // entry = list + HEADER + i*ENTRY
            abi::move_immediate(&tmp, "Integer", &LIST_ENTRY.to_string()),
            abi::multiply_registers(&entry, &i, &tmp),
            abi::add_immediate(&entry, &entry, LIST_HEADER),
            abi::add_registers(&entry, &list, &entry),
            abi::load_u64(&srcp, &entry, ENTRY_VOFF),
            abi::add_registers(&srcp, &dbase, &srcp),
            abi::load_u64(&vlen, &entry, ENTRY_VLEN),
            // cstr = alloc(vlen+1, 1)
            abi::add_immediate(abi::return_register(), &vlen, 1),
            abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_register(&cstr, abi::mfb_return(1)),
            // argv[i] = cstr
            abi::move_immediate(&tmp, "Integer", "8"),
            abi::multiply_registers(&tmp, &i, &tmp),
            abi::add_registers(&tmp, &argv, &tmp),
            abi::store_u64(&cstr, &tmp, 0),
            // copy vlen bytes srcp -> cstr, NUL-terminate
            abi::move_register(&dstp, &cstr),
            abi::move_immediate(&j, "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers(&j, &vlen),
            abi::branch_eq(&copy_done),
            abi::load_u8(&byte, &srcp, 0),
            abi::store_u8(&byte, &dstp, 0),
            abi::add_immediate(&srcp, &srcp, 1),
            abi::add_immediate(&dstp, &dstp, 1),
            abi::add_immediate(&j, &j, 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::store_u8(abi::ZERO, &dstp, 0),
            abi::add_immediate(&i, &i, 1),
            abi::branch(&build_loop),
            abi::label(&build_done),
            // argv[n] = NULL
            abi::move_immediate(&tmp, "Integer", "8"),
            abi::multiply_registers(&tmp, &n, &tmp),
            abi::add_registers(&tmp, &argv, &tmp),
            abi::store_u64(abi::ZERO, &tmp, 0),
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
        instructions.push(abi::label(&invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
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
}

pub(crate) fn lower_process_spawn_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    if call == "process.spawnEnv" {
        // plan-119-C: the four-argument overload. Unix applies the working
        // directory and the environment IN the fork child (`chdir`, then
        // `unsetenv`/`setenv` loops); `CreateProcessA` instead reads both as
        // pointers before the child exists, so both are materialized here — one
        // NUL-terminated path and one `name=value\0…\0\0` ANSI block — and the
        // merge semantics are reproduced by *building* the block.
        //
        // Same shared frame and depth-1, no-vreg discipline as the one-argument
        // arm below; `spawnEnv` just reaches further up it (`WIN_ENV_*`).
        const FRAME: usize = 0x2D0; // covers WIN_ENV_SCRATCH_END, 16-aligned
        const _: () = assert!(FRAME >= WIN_ENV_SCRATCH_END && FRAME % 16 == 0);
        let sp = abi::stack_pointer();

        let invalid = format!("{symbol}_invalid");
        let alloc_fail = format!("{symbol}_alloc_fail");
        let spawn_fail = format!("{symbol}_spawn_fail");
        let done = format!("{symbol}_done");

        let mut relocations = Vec::new();
        // All four arguments are stashed BEFORE anything else runs: every helper
        // call clobbers the argument bank, and writing one of these registers
        // early destroys an argument that has not been read yet
        // (`.ai/arch-abi.md`).
        let mut instructions = vec![
            abi::subtract_stack(FRAME),
            abi::store_u64(abi::return_register(), sp, WIN_CMD_LIST),
            abi::store_u64(abi::mfb_arg(1), sp, WIN_ENV_CWDSTR),
            abi::store_u64(abi::mfb_arg(2), sp, WIN_ENV_MAP),
            abi::store_u64(abi::mfb_arg(3), sp, WIN_ENV_REPLACE),
        ];
        emit_win_build_cmdline(
            symbol,
            "argv",
            &invalid,
            &alloc_fail,
            &mut instructions,
            &mut relocations,
        );
        emit_win_build_cwd(symbol, &alloc_fail, &mut instructions, &mut relocations);
        emit_win_build_env_block(
            symbol,
            &alloc_fail,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
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
        instructions.push(abi::label(&invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
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
    } else {
        // The body is two shared pieces (plan-119-A): `emit_win_build_cmdline`
        // joins the argv list into one `lpCommandLine`, and `emit_win_spawn_tail`
        // does the pipes, `CreateProcessA` and the record. Both address the frame
        // `sp`-relative at stack-adjust depth 1 — this one
        // `subtract_stack(FRAME)`/`add_stack(FRAME)` bracket is theirs, and no
        // abstract vregs appear anywhere inside it, so `finalize_frame` cannot
        // spill and shift the six outgoing stack args out from under the callee.
        // The slot map is documented on `WIN_SPAWN_SI` in `gen_windows.rs`.
        const FRAME: usize = 0x240; // covers WIN_CMDLINE_SCRATCH_END, 16-aligned
        const _: () = assert!(FRAME >= WIN_CMDLINE_SCRATCH_END && FRAME % 16 == 0);
        let sp = abi::stack_pointer();

        let invalid = format!("{symbol}_invalid");
        let alloc_fail = format!("{symbol}_alloc_fail");
        let spawn_fail = format!("{symbol}_spawn_fail");
        let done = format!("{symbol}_done");

        let mut relocations = Vec::new();
        let mut instructions = vec![
            abi::subtract_stack(FRAME),
            // The argv list pointer arrives in the return register.
            abi::store_u64(abi::return_register(), sp, WIN_CMD_LIST),
        ];
        emit_win_build_cmdline(
            symbol,
            "argv",
            &invalid,
            &alloc_fail,
            &mut instructions,
            &mut relocations,
        );
        // The one-argument overload inherits both the environment and the working
        // directory, so the tail's two optional pointers are NULL.
        instructions.extend([
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
        instructions.push(abi::label(&invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
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
        // Every path funnels here; unwind the frame before returning.
        instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
        Ok((instructions, relocations, 0))
    }
}
