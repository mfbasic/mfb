//! macOS `audio::devices` code generation.

use super::gen_macos_shared::*;
use super::gen_os_seam::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::target::shared::abi;
use std::collections::HashMap;

pub(crate) fn lower_devices(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let dev_fail = format!("{symbol}_dev_fail");
    let unavailable = format!("{symbol}_unavailable");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let fill_loop = format!("{symbol}_fill_loop");
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
    let v15 = vregs.next();
    let v14 = vregs.next();

    // Seed CURID_OFF with the system object id — `call_get_property` loads its
    // object from that slot, and the default-device / device-list queries all
    // run against `kAudioObjectSystemObject`. Default ids start at 0 (absent).
    instructions.extend([
        abi::move_immediate(&v9, "Integer", SYS_OBJECT),
        abi::store_u64(&v9, abi::stack_pointer(), CURID_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEFIN_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEFOUT_OFF),
    ]);
    build_propaddr(SEL_DEFIN, SCOPE_GLOBAL, &mut instructions, &mut vregs);
    call_get_property(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        CURID_OFF,
        "4",
        DEFIN_OFF,
        &mut vregs,
    )?;
    build_propaddr(SEL_DEFOUT, SCOPE_GLOBAL, &mut instructions, &mut vregs);
    call_get_property(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        CURID_OFF,
        "4",
        DEFOUT_OFF,
        &mut vregs,
    )?;

    // Device list.
    build_propaddr(SEL_DEVICES, SCOPE_GLOBAL, &mut instructions, &mut vregs);
    // object is still the system object (CURID_OFF = 1).
    call_get_property(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        CURID_OFF,
        IDSBUF_CAP,
        IDSBUF_OFF,
        &mut vregs,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // count = SIZE / 4
        abi::load_u32(&v9, abi::stack_pointer(), SIZE_OFF),
        abi::shift_right_immediate(&v9, &v9, 2),
        abi::store_u64(&v9, abi::stack_pointer(), COUNT_OFF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&unavailable),
        // Allocate List OF AudioDevice: count*ENTRY + HEADER + count*RECORD.
        abi::move_immediate(&v10, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::add_immediate(&v11, &v11, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&v12, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v13, &v9, &v12),
        abi::add_registers(abi::return_register(), &v11, &v13),
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
        abi::move_immediate(&v12, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::store_u64(&v13, &v15, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&v13, &v15, COLLECTION_OFFSET_DATA_CAPACITY),
        // entry cursor base and record data region base.
        abi::add_immediate(&v11, &v15, COLLECTION_HEADER_SIZE),
        abi::store_u64(&v11, abi::stack_pointer(), ENTRY_OFF),
        abi::move_immediate(&v12, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v13, &v10, &v12),
        abi::add_registers(&v14, &v11, &v13),
        abi::store_u64(&v14, abi::stack_pointer(), DATA_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), INDEX_OFF),
        abi::label(&fill_loop),
        abi::load_u64(&v9, abi::stack_pointer(), INDEX_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), COUNT_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&fill_done),
        // CURID = IDSBUF[index]
        abi::add_immediate(&v11, abi::stack_pointer(), IDSBUF_OFF),
        abi::move_immediate(&v12, "Integer", "4"),
        abi::multiply_registers(&v13, &v9, &v12),
        abi::add_registers(&v11, &v11, &v13),
        abi::load_u32(&v14, &v11, 0),
        abi::store_u64(&v14, abi::stack_pointer(), CURID_OFF),
    ]);
    // name, id (UID), channel-capability flags.
    emit_cfstring_field(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        SEL_NAME,
        NAMEPTR_OFF,
        &dev_fail,
        &alloc_fail,
        &mut vregs,
    )?;
    emit_cfstring_field(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        SEL_UID,
        IDPTR_OFF,
        &dev_fail,
        &alloc_fail,
        &mut vregs,
    )?;
    emit_channel_flag(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        SCOPE_INPUT,
        CANIN_OFF,
        &mut vregs,
    )?;
    emit_channel_flag(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        SCOPE_OUTPUT,
        CANOUT_OFF,
        &mut vregs,
    )?;
    // Build the record at DATA_OFF + index*RECORD.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), INDEX_OFF),
        abi::move_immediate(&v10, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::load_u64(&v12, abi::stack_pointer(), DATA_OFF),
        abi::add_registers(&v12, &v12, &v11), // record ptr
        abi::load_u64(&v13, abi::stack_pointer(), IDPTR_OFF),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_ID),
        abi::load_u64(&v13, abi::stack_pointer(), NAMEPTR_OFF),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_NAME),
        abi::load_u64(&v13, abi::stack_pointer(), CANIN_OFF),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_CAN_INPUT),
        abi::load_u64(&v13, abi::stack_pointer(), CANOUT_OFF),
        abi::store_u64(&v13, &v12, DEVICE_FIELD_CAN_OUTPUT),
    ]);
    // isDefaultInput = (CURID == DEFIN) ? 1 : 0
    emit_id_matches(
        CURID_OFF,
        DEFIN_OFF,
        DEVICE_FIELD_IS_DEFAULT_INPUT,
        symbol,
        "in",
        &v12,
        &mut instructions,
        &mut vregs,
    );
    emit_id_matches(
        CURID_OFF,
        DEFOUT_OFF,
        DEVICE_FIELD_IS_DEFAULT_OUTPUT,
        symbol,
        "out",
        &v12,
        &mut instructions,
        &mut vregs,
    );
    // Entry descriptor at ENTRY_OFF + index*ENTRY.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), INDEX_OFF),
        abi::move_immediate(&v10, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::load_u64(&v12, abi::stack_pointer(), ENTRY_OFF),
        abi::add_registers(&v12, &v12, &v11), // entry ptr
        abi::move_immediate(&v13, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&v13, &v12, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, &v12, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, &v12, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        // value_offset = index * RECORD
        abi::move_immediate(&v10, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::store_u64(&v11, &v12, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate(&v13, "Integer", &DEVICE_RECORD_SIZE.to_string()),
        abi::store_u64(&v13, &v12, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // index++
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), INDEX_OFF),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&dev_fail),
    ]);
    emit_fail(
        symbol,
        "ErrAudioDevice",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&unavailable));
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

    Ok((instructions, relocations, FRAME_SIZE))
}
