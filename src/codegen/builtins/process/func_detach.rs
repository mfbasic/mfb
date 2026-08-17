//! `process::detach` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;
use crate::types::ParameterType;

use super::native::*;

const INTRO: &str =
    r#"Relinquish ownership of a child so it keeps running after the program exits."#;
const DESC: &str = r#"`process::detach` relinquishes ownership of a child **without** killing it. It
closes the parent-side pipe ends, arranges for the operating system to auto-reap the
child when it eventually exits (on Unix, by setting `SIGCHLD` to be ignored so the
kernel reaps it and no zombie is left), and marks the handle closed. The child keeps
running on its own and survives the parent's exit.

This is the counterpart to the default drop behavior. Normally letting a `Process`
go out of scope force-kills and reaps the child; `detach` is the deliberate opt-out
for a child that should outlive the program — a daemon, a background job, a handoff
to another process.

Because `detach` marks the handle closed, it consumes the handle for all practical
purposes: every later `process::` call on it — including a second `detach` — raises
`ErrResourceClosed`, and the eventual scope-drop is a no-op rather than a kill."#;
const EX: &str = r#"Start a background job and let it outlive the program:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES job = process::shell("sleep 5")
  process::detach(job)
  io::print("job detached")
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "detach",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "p",
                desc: "The child process handle to release. Also accepts the alternate named-argument spelling `process`.",
                aliases: &["process"],
                ty: ParameterType::Named(super::PROCESS_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(
                Some(lower_process_detach_helper_posix),
                Some(lower_process_detach_helper_win),
                None,
            ),
        }],
    });
}

pub(crate) fn lower_process_detach_helper_posix(
    _call: &str,
    symbol: &str,
    _ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let sigchld = if platform.family() == PlatformFamily::MacOS {
        "20"
    } else {
        "17"
    };
    let mut v = Vregs::new();
    let file = v.next();
    let fd = v.next();
    let one = v.next();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&fd, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&fd, "0"),
        abi::branch_ne(&closed_l),
    ];
    let mut relocations = Vec::new();
    for off in [PROC_STDIN_W, PROC_STDOUT_R, PROC_STDERR_R] {
        let skip = format!("{symbol}_skip_{off}");
        instructions.extend([
            abi::load_u64(&fd, &file, off),
            abi::compare_immediate(&fd, "0"),
            abi::branch_lt(&skip),
            abi::move_register(abi::c_arg(0), &fd),
        ]);
        platform.emit_libc_call(
            "close",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::label(&skip));
    }
    // signal(SIGCHLD, SIG_IGN=1) -> kernel auto-reaps, no zombie.
    instructions.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", sigchld),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_libc_call(
        "signal",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, &file, RESOURCE_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(crate) fn lower_process_detach_helper_win(
    _call: &str,
    symbol: &str,
    _ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FILE: usize = 0x20;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
    ];
    for off in [
        PROC_STDIN_W,
        PROC_STDOUT_R,
        PROC_STDERR_R,
        RESOURCE_OFFSET_HANDLE,
    ] {
        let skip = format!("{symbol}_skip_{off}");
        instructions.extend([
            abi::load_u64(abi::mfb_arg(0), sp, FILE),
            abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), off),
            abi::compare_immediate(abi::mfb_arg(0), "0"),
            abi::branch_lt(&skip), // -1 sentinel (already closed) — skip
        ]);
        platform.emit_libc_call(
            "CloseHandle",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::label(&skip));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
