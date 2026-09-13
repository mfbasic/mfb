//! macOS `audio::devices` code generation.

use super::gen_macos_shared::*;
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::marshal::{
    emit_build_record_list, MarshalRegs, RecordBuildScratch, RecordListScratch,
};
use crate::target::shared::abi;
use std::collections::HashMap;

/// `audio::devices` on macOS: every Core Audio device, each built as the flat
/// `audio::AudioDevice` record (plan-132) by `emit_device_record` and gathered into
/// a `List OF AudioDevice` by `emit_build_record_list`.
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
    ]);
    emit_alloc_device_pairs(
        symbol,
        COUNT_OFF,
        DEVPAIRS_OFF,
        &alloc_fail,
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
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
    // isDefaultInput / isDefaultOutput = (CURID == DEFIN / DEFOUT) ? 1 : 0, into
    // their frame slots for the record build.
    emit_id_matches(
        CURID_OFF,
        DEFIN_OFF,
        DEVFLAG_DEFIN_OFF,
        symbol,
        "in",
        abi::stack_pointer(),
        &mut instructions,
        &mut vregs,
    );
    emit_id_matches(
        CURID_OFF,
        DEFOUT_OFF,
        DEVFLAG_DEFOUT_OFF,
        symbol,
        "out",
        abi::stack_pointer(),
        &mut instructions,
        &mut vregs,
    );
    emit_device_record(
        symbol,
        &DeviceRecordSlots {
            id: IDPTR_OFF,
            name: NAMEPTR_OFF,
            can_input: CANIN_OFF,
            can_output: CANOUT_OFF,
            is_default_input: DEVFLAG_DEFIN_OFF,
            is_default_output: DEVFLAG_DEFOUT_OFF,
            record: RecordBuildScratch {
                size: DEVREC_SIZE_OFF,
                result: DEVREC_RESULT_OFF,
                cursor: DEVREC_CURSOR_OFF,
                block_size: DEVREC_BLOCK_OFF,
            },
            pairs: DEVPAIRS_OFF,
            index: INDEX_OFF,
        },
        &alloc_fail,
        &mut vregs,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), INDEX_OFF),
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), INDEX_OFF),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);
    emit_build_record_list(
        symbol,
        "devices",
        DEVPAIRS_OFF,
        COUNT_OFF,
        &RecordListScratch {
            cursor: DEVLIST_CURSOR_OFF,
            index: DEVLIST_INDEX_OFF,
            list: DEVLIST_OFF,
        },
        &MarshalRegs::fresh(&mut vregs),
        RESULT_VALUE_REGISTER,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
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
