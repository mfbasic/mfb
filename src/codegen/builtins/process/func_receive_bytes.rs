//! `process::receiveBytes` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/receiveBytes.md`.

use std::collections::HashMap;

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, RegistryFunction,
    RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::emit_fail;
use crate::target::shared::code::*;

use super::native::*;

const INTRO: &str = r#"Read one available chunk of raw bytes from a child's output."#;
const DESC: &str = r#"`process::receiveBytes` reads the next available chunk of raw bytes from a child's
output stream and returns it as a `List OF Byte`. It performs one underlying read,
so it returns as soon as any data is available rather than waiting to fill a fixed
size, and the returned list is frequently shorter than the amount the child will
eventually produce. It does no line framing, decoding, or newline translation, so
it is the right call for binary output; use `process::receive` for text lines.


Without a `from` argument it reads the child's standard output; pass a `Stream`
value to choose standard output or standard error. The call blocks until at least
one byte is available or the stream ends. A pipe read returns any buffered bytes
before signalling end of stream, so late output is drained; only a read that finds
end of stream with nothing buffered raises `ErrResourceClosed`. On success the
result always holds at least one byte — end of output is never an empty list."#;
const EX: &str = r#"Read one chunk of raw output and report its length:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  LET chunk = process::receiveBytes(child)
  io::print(toString(len(chunk)))
  RETURN 0
END FUNC
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    // The optional trailing `from AS Stream` widens arity to 2 and is NOT
    // default-padded: the 2-arg form is selected at codegen (`builder_values` →
    // `process.receiveBytesFrom`), and the emitter branches on the runtime-call name.
    pkg.add_function(RegistryFunction {
        name: "receiveBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "p",
                    desc: "The child process handle to read from. Also accepts the alternate named-argument spelling `process`.",
                    aliases: &["process"],
                    ty: ParameterType::Named(super::PROCESS_TYPE),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "from",
                    desc: "Optional. Which output stream to read: `Stream.StdOut` (the default) or `Stream.StdErr`.",
                    aliases: &[],
                    ty: ParameterType::Named(super::STREAM_TYPE),
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::native_os_seam(
                Some(lower_process_receivebytes_helper_posix),
                Some(lower_process_receivebytes_helper_win),
                &["receiveBytesFrom"],
            ),
        }],
    });
}

pub(crate) fn lower_process_receivebytes_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let with_from = call == "process.receiveBytesFrom";
    const FD_OFFSET: usize = 0;
    const N_OFFSET: usize = 8;
    const BUF_OFFSET: usize = 16;
    const CHUNK: &str = "65536";
    const EINTR: &str = "4";
    let closed = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let read_retry = format!("{symbol}_read_retry");
    let read_fail = format!("{symbol}_read_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v9", abi::return_register(), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
    ];
    if with_from {
        instructions.extend([
            abi::compare_immediate(abi::c_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64("%v9", abi::return_register(), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64("%v9", abi::return_register(), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64("%v9", abi::return_register(), PROC_STDOUT_R));
    }
    instructions.extend([
        abi::store_u64("%v9", abi::stack_pointer(), FD_OFFSET),
        // Allocate the temporary chunk buffer.
        abi::move_immediate(abi::return_register(), "Integer", CHUNK),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    let mut relocations = Vec::new();
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUF_OFFSET),
        abi::label(&read_retry),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUF_OFFSET),
        abi::move_immediate(abi::c_arg(2), "Integer", CHUNK),
    ]);
    platform.emit_libc_call(
        "read",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&read_fail),
        abi::branch_eq(&closed), // EOF with nothing buffered
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), N_OFFSET),
    ]);
    // Build a List OF Byte with N elements from BUF (mirrors net.read).
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
        abi::move_immediate("%v11", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::mfb_return(1)),
        abi::move_immediate("%v9", "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), N_OFFSET),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE),
        abi::move_immediate("%v12", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v13", "%v10", "%v12"),
        abi::add_registers("%v14", "%v11", "%v13"),
        abi::load_u64("%v15", abi::stack_pointer(), BUF_OFFSET),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_eq(&entry_done),
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("%v12", "Integer", "1"),
            abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        ]);
    }
    instructions.extend([
        abi::add_registers("%v12", "%v14", "%v9"),
        abi::load_u8("%v13", "%v15", 0),
        abi::store_u8("%v13", "%v12", 0),
        abi::add_immediate("%v15", "%v15", 1),
    ]);
    if byte_list_entry_stride() != 0 {
        instructions.push(abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE));
    }
    instructions.extend([
        abi::add_immediate("%v9", "%v9", 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::move_register(RESULT_VALUE_REGISTER, abi::mfb_return(1)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&read_fail),
    ]);
    platform.emit_errno(
        symbol,
        ("%v9").into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("%v9", EINTR),
        abi::branch_eq(&read_retry),
        abi::label(&closed),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
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
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 32);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(crate) fn lower_process_receivebytes_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let with_from = call == "process.receiveBytesFrom";
    const NREAD: usize = 0x28;
    const FILE: usize = 0x30;
    const FD: usize = 0x38;
    const BUF: usize = 0x40;
    const BLOCK: usize = 0x48;
    const N: usize = 0x50;
    const I: usize = 0x58;
    const FROM: usize = 0x60;
    const FRAME: usize = 0x70;
    const CHUNK: &str = "65536";
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
    ];
    if with_from {
        instructions.push(abi::store_u64(abi::mfb_arg(1), sp, FROM));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
    ]);
    if with_from {
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), sp, FROM),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            PROC_STDOUT_R,
        ));
    }
    instructions.extend([
        abi::store_u64(abi::mfb_arg(1), sp, FD),
        // chunk buffer = arena_alloc(CHUNK, 1)
        abi::move_immediate(abi::return_register(), "Integer", CHUNK),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, BUF),
        // ReadFile(fd, buf, CHUNK, &nread, NULL)
        abi::load_u64(abi::mfb_arg(0), sp, FD),
        abi::load_u64(abi::mfb_arg(1), sp, BUF),
        abi::move_immediate(abi::mfb_arg(2), "Integer", CHUNK),
        abi::add_immediate(abi::mfb_arg(3), sp, NREAD),
        abi::store_u64(abi::ZERO, sp, 0x20),
    ]);
    platform.emit_libc_call(
        "ReadFile",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&closed_l), // ReadFile FALSE = broken pipe / EOF
        abi::load_u32(abi::mfb_arg(0), sp, NREAD),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&closed_l), // 0 bytes = EOF, nothing buffered
        abi::store_u64(abi::mfb_arg(0), sp, N),
        // result block = arena_alloc(HEADER + n, 8)  (byte-list stride 0)
        abi::add_immediate(
            abi::return_register(),
            abi::mfb_arg(0),
            COLLECTION_HEADER_SIZE,
        ),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, BLOCK),
        abi::move_register(abi::mfb_arg(0), abi::mfb_return(1)),
        abi::move_immediate(abi::mfb_arg(1), "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_KIND),
        abi::move_immediate(abi::mfb_arg(1), "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(abi::mfb_arg(1), "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            COLLECTION_OFFSET_VALUE_TYPE,
        ),
        abi::move_immediate(abi::mfb_arg(1), "Byte", "1"),
        abi::store_u8(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_COUNT),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            COLLECTION_OFFSET_DATA_LENGTH,
        ),
        abi::store_u64(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ),
        // copy n bytes: dst = block + HEADER (mfb_arg(2)), src = buf (mfb_arg(3))
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(0), COLLECTION_HEADER_SIZE),
        abi::load_u64(abi::mfb_arg(3), sp, BUF),
        abi::store_u64(abi::ZERO, sp, I),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&copy_done),
        abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(3), 0),
        abi::store_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, I),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::load_u64(RESULT_VALUE_REGISTER, sp, BLOCK),
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
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
