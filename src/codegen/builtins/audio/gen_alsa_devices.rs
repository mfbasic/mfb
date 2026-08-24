//! ALSA `audio::devices` code generation.

use super::gen_alsa_shared::*;
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::*;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) fn lower_devices(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let unavailable = format!("{symbol}_unavailable");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let count_loop = format!("{symbol}_count");
    let count_done = format!("{symbol}_count_done");
    let fill_loop = format!("{symbol}_fill");
    let fill_done = format!("{symbol}_fill_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    emit_dlopen(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &unavailable,
    )?;
    // snd_device_name_hint(-1, "pcm", &hints)
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_device_name_hint",
        &unavailable,
        false,
        |ins, _relocs| {
            ins.push(abi::bitwise_not(abi::return_register(), abi::ZERO)); // -1
            emit_data_address(symbol, abi::c_arg(1), "_mfb_audio_alsa_pcm", ins, _relocs);
            ins.push(abi::add_immediate(
                abi::c_arg(2),
                abi::stack_pointer(),
                HINTS_OFF,
            ));
        },
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&unavailable),
        // count NULL-terminated hints
        abi::load_u64(&v9, abi::stack_pointer(), HINTS_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), COUNT_OFF),
        abi::label(&count_loop),
        abi::load_u64(&v10, &v9, 0),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&count_done),
        abi::load_u64(&v11, abi::stack_pointer(), COUNT_OFF),
        abi::add_immediate(&v11, &v11, 1),
        abi::store_u64(&v11, abi::stack_pointer(), COUNT_OFF),
        abi::add_immediate(&v9, &v9, 8),
        abi::branch(&count_loop),
        abi::label(&count_done),
    ]);
    // Allocate List OF AudioDevice (48-byte records inline).
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFF),
        abi::move_immediate(&v11, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v12, &v10, &v11),
        abi::add_immediate(&v12, &v12, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v13, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v14, &v10, &v13),
        abi::add_registers(abi::return_register(), &v12, &v14),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::store_u64(&v15, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(&v9, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&v9, "Byte", &COLLECTION_TYPE_OBJECT.to_string()),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&v9, "Byte", "1"),
        abi::store_u8(&v9, &v15, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFF),
        abi::store_u64(&v10, &v15, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&v10, &v15, COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate(&v13, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v14, &v10, &v13),
        abi::store_u64(&v14, &v15, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&v14, &v15, COLLECTION_OFFSET_DATA_CAPACITY),
        // data region base = list + HEADER + count*ENTRY
        abi::add_immediate(&v11, &v15, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v12, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::add_registers(&v14, &v11, &v13),
        abi::store_u64(&v14, abi::stack_pointer(), SRC_OFF), // data region base
        abi::store_u64(&v11, abi::stack_pointer(), TOTAL_OFF), // entry cursor base
        abi::load_u64(&v9, abi::stack_pointer(), HINTS_OFF),
        abi::store_u64(&v9, abi::stack_pointer(), HINT_PTR_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF), // index
        abi::label(&fill_loop),
        abi::load_u64(&v9, abi::stack_pointer(), HINT_PTR_OFF),
        abi::load_u64(&v10, &v9, 0), // hint
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&fill_done),
        abi::store_u64(&v10, abi::stack_pointer(), N_OFF), // current hint
    ]);
    // id = get_hint(hint, "NAME")
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_device_name_get_hint",
        &unavailable,
        true,
        |ins, _relocs| {
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                N_OFF,
            ));
            emit_data_address(
                symbol,
                abi::c_arg(1),
                "_mfb_audio_alsa_hint_name",
                ins,
                _relocs,
            );
        },
    )?;
    // `snd_device_name_get_hint` returns a `char*` in the C-return bank (`rax`);
    // this raw-`blr` result is not staged into the aligned bank (`rdi`) on x86-64
    // SysV, so read it from `c_return(0)` (byte-identical on AArch64). See bug-452.
    instructions.push(abi::move_register(&v9, abi::c_return(0)));
    emit_string_from_cstr(
        symbol,
        "id",
        DEVID_OFF,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        v9.as_str(),
        &mut vregs,
    );
    // free the id cstring
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RC_OFF,
    ));
    platform.emit_external_call(
        "free",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // name = get_hint(hint, "DESC")
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_device_name_get_hint",
        &unavailable,
        true,
        |ins, _relocs| {
            // Reload the hint by dereferencing HINT_PTR_OFF rather than reading N_OFF:
            // `emit_string_from_cstr` reused N_OFF as strlen scratch while building the
            // id String, so N_OFF now holds the id length, not the hint pointer. Using
            // it here passed libasound an integer as `const void* hint` (bug-167
            // finding B: SIGSEGV / empty device name).
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                HINT_PTR_OFF,
            ));
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::return_register(),
                0,
            ));
            emit_data_address(
                symbol,
                abi::c_arg(1),
                "_mfb_audio_alsa_hint_desc",
                ins,
                _relocs,
            );
        },
    )?;
    // `snd_device_name_get_hint` returns a `char*` in the C-return bank (`rax`);
    // this raw-`blr` result is not staged into the aligned bank (`rdi`) on x86-64
    // SysV, so read it from `c_return(0)` (byte-identical on AArch64). See bug-452.
    instructions.push(abi::move_register(&v9, abi::c_return(0)));
    emit_string_from_cstr(
        symbol,
        "name",
        NAME_OFF,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        v9.as_str(),
        &mut vregs,
    );
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RC_OFF,
    ));
    platform.emit_external_call(
        "free",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Build the record: id, name, canInput=1, canOutput=1, defaults=0 (a precise
    // IOID split is a refinement; ALSA hints usually permit both directions).
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::move_immediate(&v10, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::load_u64(&v12, abi::stack_pointer(), SRC_OFF),
        abi::add_registers(&v12, &v12, &v11), // record ptr
        abi::load_u64(&v13, abi::stack_pointer(), DEVID_OFF),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_ID),
        abi::load_u64(&v13, abi::stack_pointer(), NAME_OFF),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_NAME),
        abi::move_immediate(&v13, "Integer", "1"),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_CAN_INPUT),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_CAN_OUTPUT),
        abi::store_u64(abi::ZERO, &v12, DEVICE_FIELD_IS_DEFAULT_INPUT),
        abi::store_u64(abi::ZERO, &v12, DEVICE_FIELD_IS_DEFAULT_OUTPUT),
        // entry descriptor
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::move_immediate(&v10, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::load_u64(&v12, abi::stack_pointer(), TOTAL_OFF),
        abi::add_registers(&v12, &v12, &v11),
        abi::move_immediate(&v13, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&v13, &v12, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, &v12, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, &v12, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::move_immediate(&v10, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::store_u64(&v11, &v12, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate(&v13, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::store_u64(&v13, &v12, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // advance
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), HINT_PTR_OFF),
        abi::add_immediate(&v9, &v9, 8),
        abi::store_u64(&v9, abi::stack_pointer(), HINT_PTR_OFF),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);
    // snd_device_name_free_hint(hints)
    emit_alsa_call(
        &mut vregs,
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_device_name_free_hint",
        &unavailable,
        false,
        |ins, _relocs| {
            ins.push(abi::load_u64(
                abi::return_register(),
                abi::stack_pointer(),
                HINTS_OFF,
            ));
        },
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&unavailable),
    ]);
    emit_fail(
        symbol,
        "ErrAudioUnavailable",
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
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME))
}
