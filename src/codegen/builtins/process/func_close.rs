//! `process::close` — descriptor + per-platform OS-seam emission.
//!
//! `Implementation::Os`: the member owns its arch-neutral, OS-branching native
//! emission. `lower_process_close_helper_posix` (libc `close`, macOS/Linux) and
//! `lower_process_close_helper_win` (`CloseHandle`) emit the `_mfb_rt_process_close`
//! helper body; the runtime-call dispatch (`crate::codegen::os`) picks by
//! `platform.family()`. Docs migrated from `src/docs/man/builtins/process/close.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, ParameterType, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;

use super::native::PROC_STDIN_W;

const INTRO: &str =
    r#"Close a child's standard input, signalling end-of-input; the child keeps running."#;
const DESC: &str = r#"`process::close` closes the child's standard input — the parent's write end of the
child's stdin pipe. It sends end-of-input to the child, so a filter that reads
until EOF (`sort`, `cat`, `wc`, `tr`, …) stops waiting for more input and produces
its output. After `close`, further `process::send`/`process::sendBytes` to the same
child raise `ErrResourceClosed`.

`process::close` is **not** a handle-consuming close. Despite the name, it does not
release the `Process` resource: the child keeps running, its output stays readable
with `process::receive`, and the handle remains valid and owned. The resource is
still closed the usual way — by lexical drop at scope exit (which force-kills and
reaps the child) — because `close` is deliberately not treated as an ownership
transfer.

Closing the input is idempotent with respect to the input pipe: once stdin is
closed the call is a harmless no-op. Only a handle that has already been dropped or
detached makes `close` raise `ErrResourceClosed`."#;
const EX: &str = r#"Feed a filter its input, then close stdin so it flushes its output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sorter = process::spawn(["sort"])
  process::send(sorter, "banana")
  process::send(sorter, "apple")
  process::close(sorter)
  io::print(process::receive(sorter))
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "close",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "p",
                desc: "The child process handle whose standard input to close. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`.",
                aliases: &["process"],
                ty: ParameterType::Named(super::PROCESS_TYPE),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(
                Some(lower_process_close_helper_posix),
                Some(lower_process_close_helper_win),
                None,
            ),
        }],
    });
}

pub(crate) fn lower_process_close_helper_posix(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut v = Vregs::new();
    let file = v.next();
    let closed = v.next();
    let fd = v.next();
    let neg = v.next();
    let closed_l = format!("{symbol}_closed");
    let already = format!("{symbol}_stdin_already");
    let done = format!("{symbol}_done");
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&file, abi::return_register()),
        abi::load_u64(&closed, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&closed, "0"),
        abi::branch_ne(&closed_l),
        abi::load_u64(&fd, &file, PROC_STDIN_W),
        abi::compare_immediate(&fd, "0"),
        abi::branch_lt(&already),
        abi::move_register(abi::c_arg(0), &fd),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::bitwise_not(&neg, abi::ZERO), // -1: stdin marked closed
        abi::store_u64(&neg, &file, PROC_STDIN_W),
        abi::label(&already),
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

pub(crate) fn lower_process_close_helper_win(
    _call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FILE: usize = 0x20;
    const FRAME: usize = 0x30;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let already = format!("{symbol}_already");
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
        abi::load_u64(abi::mfb_arg(0), abi::mfb_arg(0), PROC_STDIN_W),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_lt(&already), // -1 sentinel: already closed
    ];
    platform.emit_libc_call(
        "CloseHandle",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, FILE),
        // -1 sentinel (no negative immediate on Win64): 0 - 1.
        abi::move_immediate(abi::mfb_arg(0), "Integer", "0"),
        abi::subtract_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), abi::mfb_arg(1), PROC_STDIN_W),
        abi::label(&already),
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
