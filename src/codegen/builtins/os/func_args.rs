//! `os::args` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use super::gen_shared::{
    alloc_reloc, push_alloc_error, void_result, OS_ARGC_GLOBAL_SYMBOL, OS_ARGV_GLOBAL_SYMBOL,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::args` — build a `List OF String` from the entry-captured `argv`, excluding
/// `argv[0]` (the program name; D1). Reads the `_mfb_rt_os_argc` / `_mfb_rt_os_argv`
/// globals the program entry fills at startup.
pub(crate) fn lower_args(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_str = format!("{symbol}_count_str");
    let count_str_done = format!("{symbol}_count_str_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let str_len = format!("{symbol}_str_len");
    let str_len_done = format!("{symbol}_str_len_done");
    let str_copy = format!("{symbol}_str_copy");
    let str_copy_done = format!("{symbol}_str_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let argc = vregs.next();
    let argv = vregs.next();
    let index = vregs.next();
    let count = vregs.next();
    let data_bytes = vregs.next();
    let arg_ptr = vregs.next();
    let scan = vregs.next();
    let byte = vregs.next();
    let collection = vregs.next();
    let entry_cursor = vregs.next();
    let data_cursor = vregs.next();
    let data_offset = vregs.next();
    let arg_len = vregs.next();
    let scratch = vregs.next();
    let src = vregs.next();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    push_symbol_address(
        &symbol,
        OS_ARGC_GLOBAL_SYMBOL,
        &argc,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&argc, &argc, 0));
    push_symbol_address(
        &symbol,
        OS_ARGV_GLOBAL_SYMBOL,
        &argv,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&argv, &argv, 0));
    instructions.extend([
        // Pass 1: count args (from index 1) and their total byte length.
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&data_bytes, "Integer", "0"),
        abi::move_immediate(&index, "Integer", "1"),
        abi::label(&count_loop),
        abi::compare_registers(&index, &argc),
        abi::branch_ge(&count_done),
        abi::shift_left_immediate(&scratch, &index, 3),
        abi::add_registers(&scratch, &argv, &scratch),
        abi::load_u64(&arg_ptr, &scratch, 0),
        abi::move_register(&scan, &arg_ptr),
        abi::label(&count_str),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_str_done),
        abi::add_immediate(&data_bytes, &data_bytes, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&count_str),
        abi::label(&count_str_done),
        abi::add_immediate(&count, &count, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // size = HEADER + count*ENTRY_SIZE + data_bytes (a List has no buckets).
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&scratch, &scratch, &data_bytes),
        abi::add_immediate(abi::return_register(), &scratch, COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(&symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::mfb_return(1)),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&data_cursor, &entry_cursor, &scratch),
        abi::move_immediate(&data_offset, "Integer", "0"),
        // Pass 2: fill from index 1.
        abi::move_immediate(&index, "Integer", "1"),
        abi::label(&fill_loop),
        abi::compare_registers(&index, &argc),
        abi::branch_ge(&fill_done),
        abi::shift_left_immediate(&scratch, &index, 3),
        abi::add_registers(&scratch, &argv, &scratch),
        abi::load_u64(&arg_ptr, &scratch, 0),
        abi::move_register(&scan, &arg_ptr),
        abi::move_immediate(&arg_len, "Integer", "0"),
        abi::label(&str_len),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&str_len_done),
        abi::add_immediate(&arg_len, &arg_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&str_len),
        abi::label(&str_len_done),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        abi::store_u64(
            &arg_len,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::move_register(&src, &arg_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&str_copy),
        abi::compare_registers(&scratch, &arg_len),
        abi::branch_eq(&str_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&str_copy),
        abi::label(&str_copy_done),
        abi::add_registers(&data_offset, &data_offset, &arg_len),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(&symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = 0;
    Ok(void_result("os.args"))
}

const INTRO: &str = r#"The command-line arguments after the program name"#;
const DESC: &str = r#"`os::args` returns the program's command-line arguments as a `List OF String`,
**excluding** the program name — element 0 is the first real argument, not the
executable. (The program name is available through `os::executablePath`.) A
program invoked with no arguments returns an empty list.

The arguments are captured at program startup from the values the OS passes in,
so `os::args` reflects the invocation regardless of where in the program it is
called. Each element is an owned `String` copied from the corresponding `argv`
entry."#;
const EX: &str = r#"Print each argument on its own line:

```
IMPORT os
IMPORT io
IMPORT collections

SUB main()
  LET a AS List OF String = os::args()
  FOR i = 0 TO len(a) - 1
    io::print(collections::get(a, i))
  NEXT
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "args",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![],
            body: Body::abi_function(lower_args),
        }],
    });
}
