//! `process::spawn` — registry entry.
//!
//! Per-member file. `Body::Native`: the member's per-platform OS-seam emitters
//! (`*_posix`/`*_win`) delegate to the arch-neutral emission in
//! `../native/{unix,windows}`, and the runtime-call dispatch
//! (`super::dispatch_os_helper`) picks by `platform.family()`. This file carries the
//! registry entry, those emitters, and the docs migrated from
//! `src/docs/man/builtins/process/spawn.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;
use crate::types::ParameterType;

use super::native::unix::*;
use super::native::windows::*;
use super::native::*;

const INTRO: &str =
    r#"Run a program directly from an argument list, returning a handle to the child."#;
const DESC: &str = r#"`process::spawn` starts a child process from an explicit argument vector and
returns an owned `Process` handle to it. `args[0]` is the executable and is
resolved on `PATH` (`execvp` on Unix); the remaining elements are passed as the
child's arguments verbatim. **No shell is involved** — quoting, globbing, pipes,
redirection, and environment-variable expansion are *not* interpreted, so an
argument that contains spaces or shell metacharacters reaches the program as one
literal argument. Use `process::shell` when you need a shell to parse a command
line.

The child is created with three pipes wired to its standard input, output, and
error, so the parent can `process::send` to it and `process::receive` from it.
Creation forks and execs; an exec failure in the child (for example a program that
is not found) is reported back to the parent over a close-on-exec self-pipe and
surfaces as `ErrSpawnFailed`, not as a silently running child.


The returned `Process` is an owned, non-copyable resource handle. It is closed by
lexical drop when its binding leaves scope, which **force-kills and reaps** a
still-running child (`SIGKILL` + `waitpid` on Unix) so no runaway process or zombie
is left; call `process::waitFor` first if the child should be allowed to finish, or
`process::detach` to let it outlive the program.

The empty argument list is rejected with `ErrInvalidArgument` — there is no program
to run."#;
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

pub(super) fn register(pkg: &mut RegistryPackage) {
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
        implementations: vec![
            // Bare argv form.
            Implementation {
                params: vec![args.clone()],
                return_type: ParameterType::Named(super::PROCESS_TYPE_ID),
                errors: vec![],
                    body: Body::native_os_seam(
                    Some(lower_process_spawn_helper_posix),
                    Some(lower_process_spawn_helper_win),
                    &["spawnEnv"],
                ),
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
                        desc: "(full form) Environment variables for the child, each key/value set with `setenv`.",
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
                return_type: ParameterType::Named(super::PROCESS_TYPE_ID),
                errors: vec![],
                    body: Body::native_os_seam(
                    Some(lower_process_spawn_helper_posix),
                    Some(lower_process_spawn_helper_win),
                    &["spawnEnv"],
                ),
            },
        ],
    });
}

pub(crate) fn lower_process_spawn_helper_posix(
    call: &str,
    symbol: &str,
    _ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
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
            abi::label("entry"),
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
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCAL);
        Ok((frame, instructions, relocations, stack_slots))
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
            abi::label("entry"),
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
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCAL);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

pub(crate) fn lower_process_spawn_helper_win(
    call: &str,
    symbol: &str,
    _ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    if call == "process.spawnEnv" {
        unimplemented_on_windows("spawn")
    } else {
        // Win64 call frame (all `sp`-relative, addressed at stack-adjust depth 1 —
        // this whole body runs inside one `subtract_stack(FRAME)`/`add_stack(FRAME)`
        // bracket so `finalize_frame` does NOT shift these offsets: the six outgoing
        // stack args must land at the *real* `sp+0x20..` the callee reads, and the
        // shadow/SI/PI/state slots must stay consistent with them. This mirrors the
        // fs `emit_build_argv_utf8` pattern — no abstract vregs, hence no spills that
        // would be shifted out from under the depth-1 accesses. State lives in the
        // slots below `PI`; `mfb_arg(0..3)` are transient scratch reloaded from the
        // slots after every helper call (`emit_alloc`/`emit_libc_call` clobber them).
        //   [0x00..0x20)  shadow space for callees
        //   [0x20..0x50)  CreateProcessA stack args 5..10
        //   [SI..SI+104)  STARTUPINFOA (dwFlags@60, hStdInput@80/hStdOutput@88/hStdError@96)
        //   [PI..PI+24)   PROCESS_INFORMATION (hProcess@0, hThread@8, dwProcessId@16)
        //   [SA..SA+24)   SECURITY_ATTRIBUTES (nLength@0, lpSD@8, bInheritHandle@16)
        //   IN_R/IN_W/OUT_R/OUT_W/ERR_R/ERR_W  CreatePipe out-handle slots
        //   LIST/N/DBASE/CMD/DP/IDX/VLENS/REC  scalar state slots
        const SI: usize = 0x50; // STARTUPINFOA (104 bytes)
        const SI_DWFLAGS: usize = 60;
        const SI_HSTDIN: usize = 80;
        const SI_HSTDOUT: usize = 88;
        const SI_HSTDERR: usize = 96;
        const PI: usize = 0xB8; // PROCESS_INFORMATION (24 bytes)
        const SA: usize = 0xD0; // SECURITY_ATTRIBUTES (24 bytes)
        const IN_R: usize = 0xE8; // child stdin read end (child inherits)
        const IN_W: usize = 0xF0; // parent stdin write end (kept)
        const OUT_R: usize = 0xF8; // parent stdout read end (kept)
        const OUT_W: usize = 0x100; // child stdout write end (child inherits)
        const ERR_R: usize = 0x108; // parent stderr read end (kept)
        const ERR_W: usize = 0x110; // child stderr write end (child inherits)
        const LIST: usize = 0x118; // argv list pointer
        const N: usize = 0x120; // argv count
        const DBASE: usize = 0x128; // string data base
        const CMD: usize = 0x130; // cmdline buffer (also the running length before alloc)
        const DP: usize = 0x138; // cmdline write cursor
        const IDX: usize = 0x140; // outer argv index
        const VLENS: usize = 0x148; // current arg byte-length
        const REC: usize = 0x150; // resource record pointer
        const FRAME: usize = 0x160; // 16-aligned
        const HANDLE_FLAG_INHERIT: &str = "1";
        const STARTF_USESTDHANDLES: &str = "256"; // 0x100
        const LIST_COUNT: usize = COLLECTION_OFFSET_COUNT;
        const LIST_CAP: usize = COLLECTION_OFFSET_CAPACITY;
        const HDR: usize = COLLECTION_HEADER_SIZE;
        const ENT: usize = COLLECTION_ENTRY_SIZE;
        const VOFF: usize = COLLECTION_ENTRY_OFFSET_VALUE_OFFSET;
        const VLEN: usize = COLLECTION_ENTRY_OFFSET_VALUE_LENGTH;
        let sp = abi::stack_pointer();

        let invalid = format!("{symbol}_invalid");
        let alloc_fail = format!("{symbol}_alloc_fail");
        let spawn_fail = format!("{symbol}_spawn_fail");
        let sum_loop = format!("{symbol}_sum_loop");
        let sum_done = format!("{symbol}_sum_done");
        let copy_loop = format!("{symbol}_copy_loop");
        let copy_done = format!("{symbol}_copy_done");
        let inner_loop = format!("{symbol}_inner_loop");
        let inner_done = format!("{symbol}_inner_done");
        let no_space = format!("{symbol}_no_space");
        let done = format!("{symbol}_done");

        let mut relocations = Vec::new();
        let mut instructions = vec![
            abi::label("entry"),
            abi::subtract_stack(FRAME),
            // The argv list pointer arrives in the return register.
            abi::store_u64(abi::return_register(), sp, LIST),
            abi::load_u64(abi::mfb_arg(0), sp, LIST),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), LIST_COUNT),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_eq(&invalid),
            abi::store_u64(abi::mfb_arg(1), sp, N),
            // dbase = list + cap*ENT + HDR
            abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), LIST_CAP),
            abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
            abi::multiply_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(3)),
            abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), HDR),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(2)),
            abi::store_u64(abi::mfb_arg(2), sp, DBASE),
            // running length = n (separators + NUL) + sum(vlen); stash in CMD slot.
            abi::store_u64(abi::mfb_arg(1), sp, CMD),
            abi::store_u64(abi::ZERO, sp, IDX),
            abi::label(&sum_loop),
            abi::load_u64(abi::mfb_arg(0), sp, IDX),
            abi::load_u64(abi::mfb_arg(1), sp, N),
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::branch_eq(&sum_done),
            // entry = list + idx*ENT + HDR
            abi::load_u64(abi::mfb_arg(2), sp, LIST),
            abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
            abi::multiply_registers(abi::mfb_arg(1), abi::mfb_arg(0), abi::mfb_arg(3)),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), HDR),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(2), abi::mfb_arg(1)),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(1), VLEN),
            abi::load_u64(abi::mfb_arg(2), sp, CMD),
            abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(2), abi::mfb_arg(1)),
            abi::store_u64(abi::mfb_arg(2), sp, CMD),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::store_u64(abi::mfb_arg(0), sp, IDX),
            abi::branch(&sum_loop),
            abi::label(&sum_done),
            // cmd = arena_alloc(len + 1, align 1)
            abi::load_u64(abi::return_register(), sp, CMD),
            abi::add_immediate(abi::return_register(), abi::return_register(), 1),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        ];
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::store_u64(abi::mfb_return(1), sp, CMD),
            abi::store_u64(abi::mfb_return(1), sp, DP),
            abi::store_u64(abi::ZERO, sp, IDX),
            abi::label(&copy_loop),
            abi::load_u64(abi::mfb_arg(0), sp, IDX),
            abi::load_u64(abi::mfb_arg(1), sp, N),
            abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
            abi::branch_eq(&copy_done),
            // separator space before every arg but the first
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_eq(&no_space),
            abi::move_immediate(abi::mfb_arg(2), "Integer", "32"),
            abi::load_u64(abi::mfb_arg(3), sp, DP),
            abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(3), 0),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::store_u64(abi::mfb_arg(3), sp, DP),
            abi::label(&no_space),
            // entry = list + idx*ENT + HDR
            abi::load_u64(abi::mfb_arg(2), sp, LIST),
            abi::load_u64(abi::mfb_arg(0), sp, IDX),
            abi::move_immediate(abi::mfb_arg(3), "Integer", &ENT.to_string()),
            abi::multiply_registers(abi::mfb_arg(1), abi::mfb_arg(0), abi::mfb_arg(3)),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), HDR),
            abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(2), abi::mfb_arg(1)),
            // vlen -> VLENS slot; srcp -> mfb_arg(0); dp -> mfb_arg(1)
            abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(1), VLEN),
            abi::store_u64(abi::mfb_arg(2), sp, VLENS),
            abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(1), VOFF),
            abi::load_u64(abi::mfb_arg(2), sp, DBASE),
            abi::add_registers(abi::mfb_arg(0), abi::mfb_arg(2), abi::mfb_arg(0)),
            abi::load_u64(abi::mfb_arg(1), sp, DP),
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // j
            abi::label(&inner_loop),
            abi::load_u64(abi::mfb_arg(2), sp, VLENS),
            abi::compare_registers(abi::mfb_arg(3), abi::mfb_arg(2)),
            abi::branch_eq(&inner_done),
            abi::load_u8(abi::mfb_arg(2), abi::mfb_arg(0), 0),
            abi::store_u8(abi::mfb_arg(2), abi::mfb_arg(1), 0),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
            abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
            abi::branch(&inner_loop),
            abi::label(&inner_done),
            abi::store_u64(abi::mfb_arg(1), sp, DP),
            abi::load_u64(abi::mfb_arg(0), sp, IDX),
            abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
            abi::store_u64(abi::mfb_arg(0), sp, IDX),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::load_u64(abi::mfb_arg(1), sp, DP),
            abi::store_u8(abi::ZERO, abi::mfb_arg(1), 0), // NUL-terminate
            // SECURITY_ATTRIBUTES{ nLength = 24, lpSecurityDescriptor = NULL,
            // bInheritHandle = TRUE } — both pipe ends inheritable, then the parent
            // end of each is stripped of inheritance via SetHandleInformation.
            abi::move_immediate(abi::mfb_arg(0), "Integer", "24"),
            abi::store_u32(abi::mfb_arg(0), sp, SA),
            abi::store_u64(abi::ZERO, sp, SA + 8),
            abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
            abi::store_u32(abi::mfb_arg(0), sp, SA + 16),
        ]);
        // Three anonymous pipes: stdin (parent writes IN_W → child reads IN_R),
        // stdout (child writes OUT_W → parent reads OUT_R), stderr (ERR_W/ERR_R).
        // CreatePipe(&read, &write, &sa, 0); on FALSE → spawn_fail.
        for (read_slot, write_slot) in [(IN_R, IN_W), (OUT_R, OUT_W), (ERR_R, ERR_W)] {
            instructions.extend([
                abi::add_immediate(abi::mfb_arg(0), sp, read_slot),
                abi::add_immediate(abi::mfb_arg(1), sp, write_slot),
                abi::add_immediate(abi::mfb_arg(2), sp, SA),
                abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
            ]);
            platform.emit_libc_call(
                "CreatePipe",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_eq(&spawn_fail),
            ]);
        }
        // Strip inheritance from each parent-held end (IN_W/OUT_R/ERR_R) so the child
        // does not hold a duplicate that would keep a pipe from reaching EOF.
        for parent_slot in [IN_W, OUT_R, ERR_R] {
            instructions.extend([
                abi::load_u64(abi::mfb_arg(0), sp, parent_slot),
                abi::move_immediate(abi::mfb_arg(1), "Integer", HANDLE_FLAG_INHERIT),
                abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
            ]);
            platform.emit_libc_call(
                "SetHandleInformation",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        // Zero STARTUPINFOA (104 bytes), set cb = 104, dwFlags = STARTF_USESTDHANDLES,
        // and the three child-end handles.
        for off in (0..104).step_by(8) {
            instructions.push(abi::store_u64(abi::ZERO, sp, SI + off));
        }
        instructions.extend([
            abi::move_immediate(abi::mfb_arg(0), "Integer", "104"),
            abi::store_u32(abi::mfb_arg(0), sp, SI),
            abi::move_immediate(abi::mfb_arg(0), "Integer", STARTF_USESTDHANDLES),
            abi::store_u32(abi::mfb_arg(0), sp, SI + SI_DWFLAGS),
            abi::load_u64(abi::mfb_arg(0), sp, IN_R),
            abi::store_u64(abi::mfb_arg(0), sp, SI + SI_HSTDIN),
            abi::load_u64(abi::mfb_arg(0), sp, OUT_W),
            abi::store_u64(abi::mfb_arg(0), sp, SI + SI_HSTDOUT),
            abi::load_u64(abi::mfb_arg(0), sp, ERR_W),
            abi::store_u64(abi::mfb_arg(0), sp, SI + SI_HSTDERR),
            // CreateProcessA(NULL, cmd, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi).
            // Win64: register args 0..3 in mfb_arg (rcx/rdx/r8/r9); stack args 5..10
            // stored directly at sp+0x20.. (after the 32-byte shadow).
            abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
            abi::store_u64(abi::mfb_arg(0), sp, 0x20), // 5th bInheritHandles = TRUE
            abi::store_u64(abi::ZERO, sp, 0x28),       // 6th dwCreationFlags
            abi::store_u64(abi::ZERO, sp, 0x30),       // 7th lpEnvironment
            abi::store_u64(abi::ZERO, sp, 0x38),       // 8th lpCurrentDirectory
            abi::add_immediate(abi::mfb_arg(0), sp, SI),
            abi::store_u64(abi::mfb_arg(0), sp, 0x40), // 9th &si
            abi::add_immediate(abi::mfb_arg(0), sp, PI),
            abi::store_u64(abi::mfb_arg(0), sp, 0x48), // 10th &pi
            // A register arg is zeroed with an immediate, NOT `move_register(_, ZERO)`:
            // x86-64 has no hardware zero register, so `ZERO` maps to a GPR holding
            // garbage (only `store_*` special-cases it to an immediate 0).
            abi::move_immediate(abi::mfb_arg(0), "Integer", "0"), // lpApplicationName NULL
            abi::load_u64(abi::mfb_arg(1), sp, CMD),              // lpCommandLine
            abi::move_immediate(abi::mfb_arg(2), "Integer", "0"), // lpProcessAttributes NULL
            abi::move_immediate(abi::mfb_arg(3), "Integer", "0"), // lpThreadAttributes NULL
        ]);
        platform.emit_libc_call(
            "CreateProcessA",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
            abi::compare_immediate(abi::c_return(0), "0"),
            abi::branch_eq(&spawn_fail),
        ]);
        // Close the child-end handles the parent no longer needs + the thread handle.
        for close_slot in [PI + 8, IN_R, OUT_W, ERR_W] {
            instructions.push(abi::load_u64(abi::mfb_arg(0), sp, close_slot));
            platform.emit_libc_call(
                "CloseHandle",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        // Allocate + stamp the record.
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
            abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::store_u64(abi::mfb_return(1), sp, REC),
            abi::load_u64(abi::mfb_arg(0), sp, REC),
            abi::move_immediate(abi::mfb_arg(1), "Integer", RESOURCE_TAG_PROCESS),
            abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_TAG),
            abi::load_u64(abi::mfb_arg(1), sp, PI), // hProcess
            abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), RESOURCE_OFFSET_STATE),
            abi::load_u64(abi::mfb_arg(1), sp, IN_W),
            abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDIN_W),
            abi::load_u64(abi::mfb_arg(1), sp, OUT_R),
            abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
            abi::load_u64(abi::mfb_arg(1), sp, ERR_R),
            abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_REAPED),
            abi::load_u32(abi::mfb_arg(1), sp, PI + 16), // dwProcessId
            abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STATUS), // pid cached here on Windows
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), PROC_EXITCODE),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), 80),
            abi::store_u64(abi::ZERO, abi::mfb_arg(0), 88),
            abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_arg(0)),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&spawn_fail),
        ]);
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
        let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
        Ok((frame, instructions, relocations, stack_slots))
    }
}
