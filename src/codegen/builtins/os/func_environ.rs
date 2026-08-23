//! `os::environ` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_environ`]).

use super::gen_env::{emit_env_lock, emit_env_unlock_return};
use super::gen_shared::{alloc_reloc, push_alloc_error, void_result};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::environ()` — walk `char **environ` twice: pass 1 counts entries and the total
/// key+value data bytes (the `=` separator is dropped); pass 2 allocates the
/// `Map OF String` (header + entry table + data + lazy bucket region) and fills it.
/// Each `KEY=VALUE` splits at the first `=`. The whole walk holds the env lock so a
/// concurrent `os::setEnv` cannot relocate/free the array mid-walk (bug-64).
pub(crate) fn lower_environ(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_scan = format!("{symbol}_count_scan");
    let count_scan_done = format!("{symbol}_count_scan_done");
    let count_data = format!("{symbol}_count_data");
    let count_next = format!("{symbol}_count_next");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let key_scan = format!("{symbol}_key_scan");
    let key_scan_done = format!("{symbol}_key_scan_done");
    let key_copy_loop = format!("{symbol}_key_copy_loop");
    let key_copy_done = format!("{symbol}_key_copy_done");
    let val_len_loop = format!("{symbol}_val_len_loop");
    let val_store = format!("{symbol}_val_store");
    let val_copy_loop = format!("{symbol}_val_copy_loop");
    let val_copy_done = format!("{symbol}_val_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let envp = vregs.next();
    let cursor = vregs.next();
    let entry_ptr = vregs.next();
    let count = vregs.next();
    let data_bytes = vregs.next();
    let scan = vregs.next();
    let byte = vregs.next();
    let collection = vregs.next();
    let entry_cursor = vregs.next();
    let data_cursor = vregs.next();
    let data_offset = vregs.next();
    let scratch = vregs.next();
    let key_len = vregs.next();
    let val_ptr = vregs.next();
    let val_len = vregs.next();
    let src = vregs.next();
    let eq_flag = vregs.next();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    emit_env_lock(&mut EmitCtx {
        symbol: symbol.as_str(),
        platform_imports: ctx.platform_imports,
        platform: ctx.platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    ctx.platform.emit_environ_pointer(
        &symbol,
        ctx.platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&envp, abi::return_register()),
        // Pass 1: count entries and data bytes.
        abi::move_register(&cursor, &envp),
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&data_bytes, "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u64(&entry_ptr, &cursor, 0),
        abi::compare_immediate(&entry_ptr, "0"),
        abi::branch_eq(&count_done),
        abi::move_register(&scan, &entry_ptr),
        abi::move_immediate(&eq_flag, "Integer", "0"),
        abi::label(&count_scan),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_scan_done),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_ne(&count_data),
        abi::compare_immediate(&eq_flag, "0"),
        abi::branch_ne(&count_data), // a later '=' is value data
        abi::move_immediate(&eq_flag, "Integer", "1"), // first '=' is the separator
        abi::branch(&count_next),
        abi::label(&count_data),
        abi::add_immediate(&data_bytes, &data_bytes, 1),
        abi::label(&count_next),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&count_scan),
        abi::label(&count_scan_done),
        abi::add_immediate(&count, &count, 1),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // size = HEADER + count*ENTRY_SIZE + data_bytes + count*(2*MAP_BUCKET_SIZE)
        abi::move_immediate(
            &scratch,
            "Integer",
            &(COLLECTION_ENTRY_SIZE + 2 * MAP_BUCKET_SIZE).to_string(),
        ),
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
        // Header.
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_MAP.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::move_immediate(&scratch, "Byte", "0"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_BUCKETS_READY),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&data_cursor, &entry_cursor, &scratch),
        abi::move_immediate(&data_offset, "Integer", "0"),
        // Pass 2: fill.
        abi::move_register(&cursor, &envp),
        abi::label(&fill_loop),
        abi::load_u64(&entry_ptr, &cursor, 0),
        abi::compare_immediate(&entry_ptr, "0"),
        abi::branch_eq(&fill_done),
        // key_len = index of first '=' (or full length if none).
        abi::move_register(&scan, &entry_ptr),
        abi::move_immediate(&key_len, "Integer", "0"),
        abi::label(&key_scan),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&key_scan_done),
        abi::compare_immediate(&byte, "61"), // '='
        abi::branch_eq(&key_scan_done),
        abi::add_immediate(&key_len, &key_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&key_scan),
        abi::label(&key_scan_done),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ),
        abi::store_u64(&key_len, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_register(&src, &entry_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&key_copy_loop),
        abi::compare_registers(&scratch, &key_len),
        abi::branch_eq(&key_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&key_copy_loop),
        abi::label(&key_copy_done),
        abi::add_registers(&data_offset, &data_offset, &key_len),
        abi::add_registers(&val_ptr, &entry_ptr, &key_len),
        abi::move_immediate(&val_len, "Integer", "0"),
        abi::load_u8(&byte, &val_ptr, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&val_store), // no '=': empty value (val_ptr at NUL, len 0)
        abi::add_immediate(&val_ptr, &val_ptr, 1), // skip '='
        abi::move_register(&scan, &val_ptr),
        abi::label(&val_len_loop),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&val_store),
        abi::add_immediate(&val_len, &val_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&val_len_loop),
        abi::label(&val_store),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        abi::store_u64(
            &val_len,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::move_register(&src, &val_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&val_copy_loop),
        abi::compare_registers(&scratch, &val_len),
        abi::branch_eq(&val_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&val_copy_loop),
        abi::label(&val_copy_done),
        abi::add_registers(&data_offset, &data_offset, &val_len),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(&symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol: symbol.as_str(),
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = 0;
    Ok(void_result("os.environ"))
}

const INTRO: &str = r#"Snapshot every environment variable as a map"#;
const DESC: &str = r#"`os::environ` returns a `Map OF String TO String` holding every variable in the
live process environment, keyed by name. It walks the host environment array,
splitting each `NAME=VALUE` entry at its **first** `=`: the text before it is the
key and everything after it — including any further `=` — is the value. An entry
with no `=` maps its whole text to an empty-string value. The snapshot reflects
variables written earlier by `os::setEnv` and omits those removed by
`os::unsetEnv`.

The returned map is an ordinary owned value captured at the moment of the call;
later `os::setEnv`/`os::unsetEnv` calls do not change it, so re-read the
environment with a fresh `os::environ()` to observe subsequent mutations. The map
is unordered, like any `Map`. On the rare host that lists a name twice, the map
retains one entry for that key.

`os::environ` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects."#;
const EX: &str = r#"Read a value out of the environment snapshot:

```
IMPORT os
IMPORT io
IMPORT collections

SUB main()
  LET env AS Map OF String TO String = os::environ()
  io::print(collections::getOr(env, "PATH", ""))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "environ",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::map_of(ParameterType::String, ParameterType::String),
            errors: vec![],
            body: Body::abi_function(lower_environ),
        }],
    });
}
