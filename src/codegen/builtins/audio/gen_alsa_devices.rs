//! ALSA `audio::devices` code generation.

use super::gen_alsa_shared::*;
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::*;
use crate::codegen::memory::marshal::{
    emit_build_record_list, MarshalRegs, RecordBuildScratch, RecordListScratch,
};
use crate::target::shared::abi;
use std::collections::HashMap;

/// `audio::devices` on Linux: one `audio::AudioDevice` per ALSA PCM hint, each built
/// as the flat record (plan-132) by `emit_device_record` and gathered into a
/// `List OF AudioDevice` by `emit_build_record_list`. ALSA hints usually permit
/// both directions and report no default, so every device carries `canInput` and
/// `canOutput` TRUE and both default flags FALSE (a precise IOID split is a
/// refinement).
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
        // The flag words every ALSA device carries.
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), DEV_ONE_OFF),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), DEV_ZERO_OFF),
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
    emit_device_record(
        symbol,
        &DeviceRecordSlots {
            id: DEVID_OFF,
            name: NAME_OFF,
            can_input: DEV_ONE_OFF,
            can_output: DEV_ONE_OFF,
            is_default_input: DEV_ZERO_OFF,
            is_default_output: DEV_ZERO_OFF,
            record: RecordBuildScratch {
                size: DEVREC_SIZE_OFF,
                result: DEVREC_RESULT_OFF,
                cursor: DEVREC_CURSOR_OFF,
                block_size: DEVREC_BLOCK_OFF,
            },
            pairs: DEVPAIRS_OFF,
            index: OFFSET_OFF,
        },
        &alloc_fail,
        &mut vregs,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // advance
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
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
