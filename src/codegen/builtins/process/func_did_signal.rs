//! `process::didSignal` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/didSignal.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;
use crate::types::ParameterType;

use super::native::*;

const INTRO: &str = r#"Report which signal bucket a terminated child died on."#;
const DESC: &str = r#"`process::didSignal` reports how a terminated child died, as one of the four
`Signal` buckets. It reads the raw wait status cached when the child was reaped —
by `process::waitFor` or by a `process::isRunning` that observed the exit — so it
returns `Signal.None` for a child that exited normally *or* that has not yet been
observed to terminate. Await or poll the child first if you need the death cause.



On Unix it decodes the terminating signal (`WTERMSIG`): `SIGKILL` maps to
`Signal.Kill`; the fault signals `SIGILL`, `SIGABRT`, `SIGFPE`, `SIGBUS`, and
`SIGSEGV` map to `Signal.Error`; and every other terminating signal maps to
`Signal.Terminate`. On Windows exit codes carry no signal disposition, so
`didSignal` recovers only the fault case — an NTSTATUS "error"-severity exit code
(e.g. `0xC0000005` `STATUS_ACCESS_VIOLATION`) maps to `Signal.Error`, and every
other outcome maps to `Signal.None`; this is a documented Windows limitation. The
full platform mapping is tabulated in `mfb man process types`.


Reading a handle that has already been dropped or detached raises
`ErrResourceClosed`."#;
const EX: &str = r#"Report how a child died after killing it:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "30"])
  process::signal(child, Signal.Kill)
  LET code = process::waitFor(child)
  IF process::didSignal(child) = Signal.Kill THEN
    io::print("killed")
  END IF
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "didSignal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "p",
                desc: "The terminated child process handle to inspect. Also accepts the alternate named-argument spelling `process`.",
                aliases: &["process"],
                ty: ParameterType::Named(super::PROCESS_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Named(super::SIGNAL_TYPE),
            errors: vec![],
            body: Body::native(
                Some(lower_process_didsignal_helper_posix),
                Some(lower_process_didsignal_helper_win),
                None,
            ),
        }],
    });
}

pub(crate) fn lower_process_didsignal_helper_posix(
    _call: &str,
    symbol: &str,
    _build_mode: crate::target::NativeBuildMode,
    _module_name: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let reaped = v.next();
    let status = v.next();
    let termsig = v.next();
    let closed_l = format!("{symbol}_closed");
    let ret_none = format!("{symbol}_ret_none");
    let ret_kill = format!("{symbol}_ret_kill");
    let ret_error = format!("{symbol}_ret_error");
    let ret_term = format!("{symbol}_ret_term");
    let ret = format!("{symbol}_ret");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&reaped, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&reaped, &file, PROC_REAPED),
        abi::compare_immediate(&reaped, "0"),
        abi::branch_eq(&ret_none),
        abi::load_u64(&status, &file, PROC_STATUS),
        abi::move_immediate(&termsig, "Integer", "127"),
        abi::and_registers(&termsig, &status, &termsig),
        abi::compare_immediate(&termsig, "0"),
        abi::branch_eq(&ret_none),
        abi::compare_immediate(&termsig, "9"),
        abi::branch_eq(&ret_kill),
        // Error bucket: SIGILL(4)/SIGABRT(6)/SIGFPE(8)/SIGBUS(10)/SIGSEGV(11).
        abi::compare_immediate(&termsig, "4"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "6"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "8"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "10"),
        abi::branch_eq(&ret_error),
        abi::compare_immediate(&termsig, "11"),
        abi::branch_eq(&ret_error),
        // Everything else -> Terminate.
        abi::label(&ret_term),
        abi::move_immediate(&termsig, "Integer", "2"),
        abi::branch(&ret),
        abi::label(&ret_none),
        abi::move_immediate(&termsig, "Integer", "0"),
        abi::branch(&ret),
        abi::label(&ret_kill),
        abi::move_immediate(&termsig, "Integer", "1"),
        abi::branch(&ret),
        abi::label(&ret_error),
        abi::move_immediate(&termsig, "Integer", "3"),
        abi::label(&ret),
        abi::move_register(RESULT_VALUE_REGISTER, &termsig),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ];
    let mut relocations = Vec::new();
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

pub(crate) fn lower_process_didsignal_helper_win(
    _call: &str,
    symbol: &str,
    _build_mode: crate::target::NativeBuildMode,
    _module_name: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    let closed_l = format!("{symbol}_closed");
    let ret_none = format!("{symbol}_ret_none");
    let ret_error = format!("{symbol}_ret_error");
    let ret = format!("{symbol}_ret");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(abi::mfb_arg(0), abi::return_register()),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_REAPED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_eq(&ret_none),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STATUS),
        // NTSTATUS severity == 3 (error/exception) iff (code >> 30) == 3.
        abi::shift_right_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 30),
        abi::compare_immediate(abi::mfb_arg(1), "3"),
        abi::branch_eq(&ret_error),
        abi::label(&ret_none),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
        abi::branch(&ret),
        abi::label(&ret_error),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "3"),
        abi::label(&ret),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_arg(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ];
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
