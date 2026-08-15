//! `process::shell` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/shell.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;

use super::native::unix::*;
use super::native::windows::*;

const INTRO: &str =
    r#"Run a command line through the platform shell, returning a handle to the child."#;
const DESC: &str = r#"`process::shell` runs `cmd` as a shell command line and returns an owned `Process`
handle to the resulting child. Unlike `process::spawn`, which execs a program
directly, `shell` hands the string to the platform shell — `/bin/sh -c` on Unix —
so shell features work: pipelines (`|`), redirection (`>`, `<`), globbing (`*`),
command sequencing (`;`, `&&`), quoting, and environment-variable expansion are all
interpreted by the shell.

Because the string is parsed by a shell, values interpolated into `cmd` are subject
to shell word-splitting and metacharacter interpretation; build the command with
care when any part comes from untrusted input. When you do not need a shell — you
have a program and its arguments already separated — prefer `process::spawn`, which
avoids the shell entirely.

The child is wired to three pipes for its standard streams exactly as with
`process::spawn`, and the returned handle has the same ownership: it is closed by
lexical drop at scope exit, which force-kills and reaps a still-running child unless
it is first awaited with `process::waitFor` or released with `process::detach`."#;
const EX: &str = r#"Run a pipeline and read the result:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo hello | tr a-z A-Z")
  io::print(process::receive(sh))
  RETURN 0
END FUNC
```

Run a command and wait for its exit code:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("true")
  io::print(toString(process::waitFor(sh)))
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "shell",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "cmd",
                desc: "The command line to run through the platform shell (`/bin/sh -c` on Unix). Also accepts the alternate named-argument spelling `command`.",
                aliases: &["command"],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Named(super::PROCESS_TYPE),
            errors: vec![],
            body: Body::native(
                Some(lower_process_shell_helper_posix),
                Some(lower_process_shell_helper_win),
                None,
            ),
        }],
    });
}

pub(crate) fn lower_process_shell_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
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
        abi::label("entry"),
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
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], LOCAL);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(crate) fn lower_process_shell_helper_win(
    _call: &str,
    _symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    unimplemented_on_windows("shell")
}
