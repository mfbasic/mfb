//! `process::signal` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/signal.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;
use crate::types::ParameterType;

const INTRO: &str = r#"Deliver a cross-platform signal bucket to a child process."#;
const DESC: &str = r#"`process::signal` delivers one of the four `Signal` buckets to the child behind a
`Process` handle. The bucket abstracts over platform signal numbers so the same
call works on Unix and Windows. `Signal.None` is a no-op. On Unix, `Signal.Kill`
sends `SIGKILL`, `Signal.Terminate` sends `SIGTERM`, and `Signal.Error` sends
`SIGABRT`.


On Windows there is no way to deliver an arbitrary signal to a child without a
shared console, so every terminating bucket maps to the same best-effort
`TerminateProcess`, with a POSIX-flavored exit code (`128 + signo`, so `137`/`143`/
`134` for `Kill`/`Terminate`/`Error`) that a later `process::waitFor` can read back;
there is no per-signal fidelity. The full platform mapping is tabulated in
`mfb man process types`.

Delivery does not wait for or reap the child; call `process::waitFor` afterward to
collect the exit status, or `process::didSignal` to read back which bucket a
terminated child died on. Signalling a handle that has already been dropped or
detached raises `ErrResourceClosed`."#;
const EX: &str = r#"Ask a long-running child to stop, then wait for it:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "30"])
  process::signal(child, Signal.Terminate)
  io::print(toString(process::waitFor(child)))
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "signal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "p",
                    desc: "The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`.",
                    aliases: &["process"],
                    ty: ParameterType::Named(super::PROCESS_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "sig",
                    desc: "The bucket to deliver: `Signal.None` (no-op), `Signal.Kill`, `Signal.Terminate`, or `Signal.Error`. Also accepts the alternate named-argument spelling `signal`.",
                    aliases: &["signal"],
                    ty: ParameterType::Named(super::SIGNAL_TYPE),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(
                Some(lower_process_signal_helper_posix),
                Some(lower_process_signal_helper_win),
                None,
            ),
        }],
    });
}

pub(crate) fn lower_process_signal_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let sig = v.next();
    let num = v.next();
    let closed_l = format!("{symbol}_closed");
    let set_kill = format!("{symbol}_set_kill");
    let set_term = format!("{symbol}_set_term");
    let do_kill = format!("{symbol}_do_kill");
    let done_ok = format!("{symbol}_done_ok");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&sig, abi::c_arg(1)),
        abi::load_u64(&num, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&num, "0"),
        abi::branch_ne(&closed_l),
        // None (0) -> no-op.
        abi::compare_immediate(&sig, "0"),
        abi::branch_eq(&done_ok),
        abi::compare_immediate(&sig, "1"),
        abi::branch_eq(&set_kill),
        abi::compare_immediate(&sig, "2"),
        abi::branch_eq(&set_term),
        // Error (3) -> SIGABRT.
        abi::move_immediate(&num, "Integer", "6"),
        abi::branch(&do_kill),
        abi::label(&set_kill),
        abi::move_immediate(&num, "Integer", "9"),
        abi::branch(&do_kill),
        abi::label(&set_term),
        abi::move_immediate(&num, "Integer", "15"),
        abi::label(&do_kill),
        abi::load_u64(abi::c_arg(0), &file, RESOURCE_OFFSET_HANDLE),
        abi::move_register(abi::c_arg(1), &num),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "kill",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&done_ok),
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

pub(crate) fn lower_process_signal_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FILE: usize = 0x20;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let set_kill = format!("{symbol}_set_kill");
    let set_term = format!("{symbol}_set_term");
    let do_kill = format!("{symbol}_do_kill");
    let done_ok = format!("{symbol}_done_ok");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        // sig arrives in the 2nd MFB arg; keep it in mfb_arg(1) across the checks.
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(2), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(2), "0"),
        abi::branch_ne(&closed_l),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_eq(&done_ok), // None -> no-op
        abi::compare_immediate(abi::mfb_arg(1), "1"),
        abi::branch_eq(&set_kill),
        abi::compare_immediate(abi::mfb_arg(1), "2"),
        abi::branch_eq(&set_term),
        // Error (3) -> exit code 134 (128 + SIGABRT).
        abi::move_immediate(abi::mfb_arg(1), "Integer", "134"),
        abi::branch(&do_kill),
        abi::label(&set_kill),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "137"), // 128 + SIGKILL
        abi::branch(&do_kill),
        abi::label(&set_term),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "143"), // 128 + SIGTERM
        abi::label(&do_kill),
        // TerminateProcess(hProcess, code)
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), RESOURCE_OFFSET_HANDLE),
    ];
    platform.emit_libc_call(
        "TerminateProcess",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&done_ok),
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
