//! macOS `audio` stream lifecycle code generation (openInput/openOutput + close).

use super::gen_common::*;
use super::gen_macos_shared::*;
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// openOutput(sampleRate, channels, bufferFrames) or the device overload.
pub(crate) fn lower_open_output(
    symbol: &str,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let buf_loop = format!("{symbol}_buf_loop");
    let buf_done = format!("{symbol}_buf_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v15 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();

    // Argument staging. The device overload shifts the scalar args by one.
    if device {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), DEVID_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(3), abi::stack_pointer(), BF_OFF),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), BF_OFF),
        ]);
    }
    // Zero the state slot so the open-error cleanup can tell the page was not yet
    // mapped (nothing to munmap/dispose before mmap and AudioQueueNew* run).
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF));
    emit_validate_open(
        symbol,
        SR_OFF,
        CH_OFF,
        BF_OFF,
        &invalid,
        &mut instructions,
        &mut vregs,
    );
    // bytesPerFrame = channels * 2
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::move_immediate(&v10, "Integer", "2"),
        abi::multiply_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), BPF_OFF),
        // AudioHandle (arena, 64 B).
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &H_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::store_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        // Canonical plan-80 header: tag@0, kind (handle)@8, closed@16, STATE@24.
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_AUDIO),
        abi::store_u64(&v9, &v15, RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, &v15, RESOURCE_OFFSET_STATE),
        abi::move_immediate(&v9, "Integer", KIND_OUTPUT),
        abi::store_u64(&v9, &v15, H_KIND),
        abi::store_u64(abi::ZERO, &v15, H_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SR_OFF),
        abi::store_u64(&v9, &v15, H_SAMPLE_RATE),
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::store_u64(&v9, &v15, H_CHANNELS),
        abi::load_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::store_u64(&v9, &v15, H_BYTES_PER_FRAME),
        abi::load_u64(&v9, abi::stack_pointer(), BF_OFF),
        abi::store_u64(&v9, &v15, H_BUFFER_FRAMES),
    ]);
    // mmap the AudioState page.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // addr
        abi::move_immediate(abi::c_arg(1), "Integer", &STATE_PAGE.to_string()),
        abi::move_immediate(abi::c_arg(2), "Integer", MMAP_PROT),
        abi::move_immediate(abi::c_arg(3), "Integer", MMAP_FLAGS),
        abi::bitwise_not(abi::c_arg(4), abi::ZERO), // fd = -1
        abi::move_immediate(abi::c_arg(5), "Integer", "0"), // offset
    ]);
    platform.emit_external_call(
        "mmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // MAP_FAILED == (void*)-1
        abi::add_immediate(&v9, abi::return_register(), 1),
        abi::compare_immediate(&v9, MAP_FAILED_CMP),
        abi::branch_eq(&dev_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::return_register(), &v15, H_STATE),
        // Zero the bookkeeping words (mmap zero-fills, but be explicit).
        abi::load_u64(&v15, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v15, S_XRUNS),
        abi::store_u64(abi::ZERO, &v15, S_CLOSED),
        abi::store_u64(abi::ZERO, &v15, S_STARTED),
        abi::store_u64(abi::ZERO, &v15, S_FREE_TOP),
        abi::store_u64(abi::ZERO, &v15, S_RING_CAP),
        abi::move_immediate(&v9, "Integer", &STATE_PAGE.to_string()),
        abi::store_u64(&v9, &v15, S_MAP_SIZE),
    ]);
    // pthread_mutex_init(state+S_MUTEX, NULL); pthread_cond_init(state+S_COND, NULL)
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_init",
        STATE_OFF,
        S_MUTEX,
    )?;
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_init",
        STATE_OFF,
        S_COND,
    )?;
    // Build the ASBD.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SR_OFF),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], &v9),
        abi::store_double(abi::FP_SCRATCH[0], abi::stack_pointer(), ASBD_OFF),
        abi::move_immediate(&v9, "Integer", FORMAT_LPCM),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 8),
        abi::move_immediate(&v9, "Integer", FORMAT_FLAGS),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 12),
        abi::load_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 16), // mBytesPerPacket
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u32(&v10, abi::stack_pointer(), ASBD_OFF + 20), // mFramesPerPacket
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 24),  // mBytesPerFrame
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 28), // mChannelsPerFrame
        abi::move_immediate(&v9, "Integer", "16"),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 32), // mBitsPerChannel
        abi::store_u32(abi::ZERO, abi::stack_pointer(), ASBD_OFF + 36),
    ]);
    // AudioQueueNewOutput(&asbd, callback, handle, NULL, NULL, 0, &state->osobject)
    instructions.extend([abi::add_immediate(
        abi::return_register(),
        abi::stack_pointer(),
        ASBD_OFF,
    )]);
    emit_data_address(
        symbol,
        abi::c_arg(1),
        AUDIO_OUTPUT_CALLBACK_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::move_immediate(abi::c_arg(4), "Integer", "0"),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
        abi::load_u64(abi::c_arg(6), abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(6), abi::c_arg(6), S_OSOBJECT),
    ]);
    platform.emit_external_call(
        "AudioQueueNewOutput",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
    ]);
    // Optionally select the named device.
    if device {
        emit_select_device(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &dev_fail,
            &mut vregs,
        )?;
    }
    // Allocate NUM_BUFFERS buffers; all start free.
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), I_OFF),
        abi::label(&buf_loop),
        abi::load_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::compare_immediate(&v9, &NUM_BUFFERS.to_string()),
        abi::branch_eq(&buf_done),
        // AudioQueueAllocateBuffer(queue, bufferFrames*bytesPerFrame, &buf)
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::load_u64(&v11, abi::stack_pointer(), BF_OFF),
        abi::load_u64(&v12, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v11, &v11, &v12),
        abi::move_register(abi::c_arg(1), &v11),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), BUFPTR_OFF),
    ]);
    platform.emit_external_call(
        "AudioQueueAllocateBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // freebufs[i] = buf; (i already == free_top since all free)
        abi::load_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(&v10, &v10, S_FREE_BUFS),
        abi::move_immediate(&v11, "Integer", "8"),
        abi::multiply_registers(&v12, &v9, &v11),
        abi::add_registers(&v10, &v10, &v12),
        abi::load_u64(&v11, abi::stack_pointer(), BUFPTR_OFF),
        abi::store_u64(&v11, &v10, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::branch(&buf_loop),
        abi::label(&buf_done),
        // free_top = NUM_BUFFERS
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(&v9, "Integer", &NUM_BUFFERS.to_string()),
        abi::store_u64(&v9, &v10, S_FREE_TOP),
        // No buffer is part-filled yet.
        abi::store_u64(abi::ZERO, &v10, S_PENDING_BUF),
        abi::store_u64(abi::ZERO, &v10, S_PENDING_FILL),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v10, S_STARTED),
        // AudioQueueStart(queue, NULL)
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "AudioQueueStart",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // Success: return the handle.
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&dev_fail));
    emit_open_cleanup(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;
    emit_fail(
        symbol,
        "ErrAudioDevice",
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

    Ok((instructions, relocations, F))
}

/// closeOutput(output): drain, stop, dispose, destroy, munmap. Idempotent.
pub(crate) fn lower_close_output(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let already = format!("{symbol}_already");
    let drain_loop = format!("{symbol}_drain_loop");
    let drain_done = format!("{symbol}_drain_done");
    let no_pending = format!("{symbol}_no_pending");
    let pad_loop = format!("{symbol}_pad_loop");
    let pad_done = format!("{symbol}_pad_done");
    let enq_ok = format!("{symbol}_enq_ok");
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
    let v17 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&already),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
        // Set if the padded buffer below is rejected by the device; consumed
        // under the mutex further down.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), I_OFF),
        // cap = bufferFrames * bytesPerFrame
        abi::load_u64(&v11, abi::return_register(), H_BUFFER_FRAMES),
        abi::load_u64(&v12, abi::return_register(), H_BYTES_PER_FRAME),
        abi::multiply_registers(&v13, &v11, &v12),
        abi::store_u64(&v13, abi::stack_pointer(), CAP_OFF),
        // A part-filled buffer left over from the last write has to go out
        // before the drain can succeed, and it has to go out FULL: the queue
        // never finishes a buffer holding less than a period, so a short one
        // would never come back and the drain would wait forever (bug-370).
        // Pad the unused tail with silence. Done before the mutex is taken —
        // enqueuing can run the callback, which takes that same non-recursive
        // mutex.
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v10, S_PENDING_FILL),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&no_pending),
        abi::load_u64(&v14, &v10, S_PENDING_BUF),
        abi::store_u64(&v14, abi::stack_pointer(), BUFPTR_OFF),
        abi::load_u64(&v15, &v14, 8), // mAudioData
        abi::add_registers(&v15, &v15, &v9),
        abi::move_immediate(&v17, "Integer", "0"),
        abi::label(&pad_loop),
        abi::compare_registers(&v9, &v13),
        abi::branch_ge(&pad_done),
        abi::store_u8(&v17, &v15, 0),
        abi::add_immediate(&v15, &v15, 1),
        abi::add_immediate(&v9, &v9, 1),
        abi::branch(&pad_loop),
        abi::label(&pad_done),
        abi::load_u64(&v14, abi::stack_pointer(), BUFPTR_OFF),
        abi::load_u64(&v13, abi::stack_pointer(), CAP_OFF),
        abi::store_u32(&v13, &v14, 16), // mAudioDataByteSize = cap
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v10, S_PENDING_FILL),
        abi::store_u64(abi::ZERO, &v10, S_PENDING_BUF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUFPTR_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "AudioQueueEnqueueBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&enq_ok),
        // The device refused it, so the queue will never hand this buffer back.
        // Note it; the drain below returns it to the free stack under the mutex,
        // or that drain would be the very hang this padding exists to prevent.
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::label(&enq_ok),
        abi::label(&no_pending),
    ]);
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
        STATE_OFF,
        S_MUTEX,
    )?;
    instructions.extend([
        // A rejected pad buffer never reached the queue, so put it back on the
        // free stack here, where the mutex makes that safe against the callback.
        abi::load_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&drain_loop),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v10, S_FREE_TOP),
        abi::add_immediate(&v11, &v10, S_FREE_BUFS),
        abi::move_immediate(&v12, "Integer", "8"),
        abi::multiply_registers(&v13, &v9, &v12),
        abi::add_registers(&v11, &v11, &v13),
        abi::load_u64(&v14, abi::stack_pointer(), BUFPTR_OFF),
        abi::store_u64(&v14, &v11, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, &v10, S_FREE_TOP),
        abi::label(&drain_loop),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v10, S_FREE_TOP),
        abi::compare_immediate(&v9, &NUM_BUFFERS.to_string()),
        abi::branch_ge(&drain_done),
        abi::add_immediate(abi::return_register(), &v10, S_COND),
        abi::add_immediate(abi::c_arg(1), &v10, S_MUTEX),
    ]);
    platform.emit_external_call(
        "pthread_cond_wait",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&drain_loop),
        abi::label(&drain_done),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v10, S_CLOSED),
    ]);
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
        STATE_OFF,
        S_MUTEX,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "AudioQueueStop",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "AudioQueueDispose",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_destroy",
        STATE_OFF,
        S_COND,
    )?;
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_destroy",
        STATE_OFF,
        S_MUTEX,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v10, H_CLOSED),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::c_arg(1), abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_external_call(
        "munmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    Ok((instructions, relocations, F))
}

/// openInput(sampleRate, channels, bufferFrames) or the device overload.
pub(crate) fn lower_open_input(
    symbol: &str,
    device: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let unavailable = format!("{symbol}_unavailable");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let buf_loop = format!("{symbol}_buf_loop");
    let buf_done = format!("{symbol}_buf_done");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v15 = vregs.next();
    if device {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), DEVID_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(3), abi::stack_pointer(), BF_OFF),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), SR_OFF),
            abi::store_u64(abi::c_arg(1), abi::stack_pointer(), CH_OFF),
            abi::store_u64(abi::c_arg(2), abi::stack_pointer(), BF_OFF),
        ]);
    }
    // Zero the state slot so the open-error cleanup can tell the page was not yet
    // mapped (nothing to munmap/dispose before mmap and AudioQueueNew* run).
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), STATE_OFF));
    emit_validate_open(
        symbol,
        SR_OFF,
        CH_OFF,
        BF_OFF,
        &invalid,
        &mut instructions,
        &mut vregs,
    );
    // §4.5: for the default overload, require a default input device.
    if !device {
        instructions.extend([
            abi::move_immediate(&v9, "Integer", SEL_DEFIN),
            abi::store_u32(&v9, abi::stack_pointer(), PRECHK_ADDR),
            abi::move_immediate(&v9, "Integer", SCOPE_GLOBAL),
            abi::store_u32(&v9, abi::stack_pointer(), PRECHK_ADDR + 4),
            abi::store_u32(abi::ZERO, abi::stack_pointer(), PRECHK_ADDR + 8),
            abi::move_immediate(&v9, "Integer", "4"),
            abi::store_u32(&v9, abi::stack_pointer(), PRECHK_SIZE),
            abi::store_u32(abi::ZERO, abi::stack_pointer(), PRECHK_ID),
            abi::move_immediate(abi::return_register(), "Integer", SYS_OBJECT),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), PRECHK_ADDR),
            abi::move_immediate(abi::c_arg(2), "Integer", "0"),
            abi::move_immediate(abi::c_arg(3), "Integer", "0"),
            abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), PRECHK_SIZE),
            abi::add_immediate(abi::c_arg(5), abi::stack_pointer(), PRECHK_ID),
        ]);
        platform.emit_external_call(
            "AudioObjectGetPropertyData",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ne(&unavailable),
            abi::load_u32(&v9, abi::stack_pointer(), PRECHK_ID),
            abi::compare_immediate(&v9, "0"),
            abi::branch_eq(&unavailable),
        ]);
    }
    // bytesPerFrame, ringCap, mapSize.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::move_immediate(&v10, "Integer", "2"),
        abi::multiply_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), BPF_OFF),
        // ringCap = bufferFrames * bytesPerFrame * NUM_BUFFERS
        abi::load_u64(&v11, abi::stack_pointer(), BF_OFF),
        abi::multiply_registers(&v11, &v11, &v9),
        abi::move_immediate(&v12, "Integer", &NUM_BUFFERS.to_string()),
        abi::multiply_registers(&v11, &v11, &v12),
        abi::store_u64(&v11, abi::stack_pointer(), RINGCAP_OFF),
        // mapSize = round_up(S_RING + ringCap, STATE_PAGE)
        abi::add_immediate(&v11, &v11, S_RING),
        abi::add_immediate(&v11, &v11, STATE_PAGE - 1),
        abi::shift_right_immediate(&v11, &v11, 14),
        abi::shift_left_immediate(&v11, &v11, 14),
        abi::store_u64(&v11, abi::stack_pointer(), MAPSIZE_OFF),
        // AudioHandle.
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &H_RECORD_SIZE.to_string(),
        ),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)),
        abi::store_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        // Canonical plan-80 header: tag@0, kind (handle)@8, closed@16, STATE@24.
        abi::move_immediate(&v9, "Integer", RESOURCE_TAG_AUDIO),
        abi::store_u64(&v9, &v15, RESOURCE_OFFSET_TAG),
        abi::store_u64(abi::ZERO, &v15, RESOURCE_OFFSET_STATE),
        abi::move_immediate(&v9, "Integer", KIND_INPUT),
        abi::store_u64(&v9, &v15, H_KIND),
        abi::store_u64(abi::ZERO, &v15, H_CLOSED),
        abi::load_u64(&v9, abi::stack_pointer(), SR_OFF),
        abi::store_u64(&v9, &v15, H_SAMPLE_RATE),
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::store_u64(&v9, &v15, H_CHANNELS),
        abi::load_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::store_u64(&v9, &v15, H_BYTES_PER_FRAME),
        abi::load_u64(&v9, abi::stack_pointer(), BF_OFF),
        abi::store_u64(&v9, &v15, H_BUFFER_FRAMES),
        // mmap(0, mapSize, PROT, FLAGS, -1, 0)
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), MAPSIZE_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", MMAP_PROT),
        abi::move_immediate(abi::c_arg(3), "Integer", MMAP_FLAGS),
        abi::bitwise_not(abi::c_arg(4), abi::ZERO),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "mmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::add_immediate(&v9, abi::return_register(), 1),
        abi::compare_immediate(&v9, MAP_FAILED_CMP),
        abi::branch_eq(&dev_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v15, abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::return_register(), &v15, H_STATE),
        abi::load_u64(&v15, abi::stack_pointer(), STATE_OFF),
        abi::store_u64(abi::ZERO, &v15, S_XRUNS),
        abi::store_u64(abi::ZERO, &v15, S_CLOSED),
        abi::store_u64(abi::ZERO, &v15, S_STARTED),
        abi::store_u64(abi::ZERO, &v15, S_RING_HEAD),
        abi::store_u64(abi::ZERO, &v15, S_RING_TAIL),
        abi::store_u64(abi::ZERO, &v15, S_RING_FILL),
        abi::load_u64(&v9, abi::stack_pointer(), RINGCAP_OFF),
        abi::store_u64(&v9, &v15, S_RING_CAP),
        abi::load_u64(&v9, abi::stack_pointer(), MAPSIZE_OFF),
        abi::store_u64(&v9, &v15, S_MAP_SIZE),
    ]);
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_init",
        STATE_OFF,
        S_MUTEX,
    )?;
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_init",
        STATE_OFF,
        S_COND,
    )?;
    // ASBD.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SR_OFF),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], &v9),
        abi::store_double(abi::FP_SCRATCH[0], abi::stack_pointer(), ASBD_OFF),
        abi::move_immediate(&v9, "Integer", FORMAT_LPCM),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 8),
        abi::move_immediate(&v9, "Integer", FORMAT_FLAGS),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 12),
        abi::load_u64(&v9, abi::stack_pointer(), BPF_OFF),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 16),
        abi::move_immediate(&v10, "Integer", "1"),
        abi::store_u32(&v10, abi::stack_pointer(), ASBD_OFF + 20),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 24),
        abi::load_u64(&v9, abi::stack_pointer(), CH_OFF),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 28),
        abi::move_immediate(&v9, "Integer", "16"),
        abi::store_u32(&v9, abi::stack_pointer(), ASBD_OFF + 32),
        abi::store_u32(abi::ZERO, abi::stack_pointer(), ASBD_OFF + 36),
        // AudioQueueNewInput(&asbd, callback, handle, NULL, NULL, 0, &osobject)
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), ASBD_OFF),
    ]);
    emit_data_address(
        symbol,
        abi::c_arg(1),
        AUDIO_INPUT_CALLBACK_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::move_immediate(abi::c_arg(4), "Integer", "0"),
        abi::move_immediate(abi::c_arg(5), "Integer", "0"),
        abi::load_u64(abi::c_arg(6), abi::stack_pointer(), STATE_OFF),
        abi::add_immediate(abi::c_arg(6), abi::c_arg(6), S_OSOBJECT),
    ]);
    platform.emit_external_call(
        "AudioQueueNewInput",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
    ]);
    if device {
        emit_select_device(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &dev_fail,
            &mut vregs,
        )?;
    }
    // Allocate NUM_BUFFERS buffers and enqueue each (input buffers must be
    // enqueued to receive captured data).
    instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), I_OFF),
        abi::label(&buf_loop),
        abi::load_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::compare_immediate(&v9, &NUM_BUFFERS.to_string()),
        abi::branch_eq(&buf_done),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::load_u64(&v11, abi::stack_pointer(), BF_OFF),
        abi::load_u64(&v12, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v11, &v11, &v12),
        abi::move_register(abi::c_arg(1), &v11),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), BUFPTR_OFF),
    ]);
    platform.emit_external_call(
        "AudioQueueAllocateBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        // AudioQueueEnqueueBuffer(queue, buf, 0, NULL)
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUFPTR_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "AudioQueueEnqueueBuffer",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, abi::stack_pointer(), I_OFF),
        abi::branch(&buf_loop),
        abi::label(&buf_done),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v10, S_STARTED),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "AudioQueueStart",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        symbol,
        "ErrInvalidArgument",
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
    instructions.push(abi::label(&dev_fail));
    emit_open_cleanup(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;
    emit_fail(
        symbol,
        "ErrAudioDevice",
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
    Ok((instructions, relocations, F))
}

/// closeInput(input): drop captured data, stop, dispose, destroy, munmap.
pub(crate) fn lower_close_input(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let already = format!("{symbol}_already");
    let done = format!("{symbol}_done");
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&already),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
    ]);
    // Set closed under the mutex first (a racing callback must not touch the ring).
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
        STATE_OFF,
        S_MUTEX,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v10, S_CLOSED),
    ]);
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
        STATE_OFF,
        S_MUTEX,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "AudioQueueStop",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "AudioQueueDispose",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_destroy",
        STATE_OFF,
        S_COND,
    )?;
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_destroy",
        STATE_OFF,
        S_MUTEX,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), HANDLE_OFF),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::store_u64(&v9, &v10, H_CLOSED),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::c_arg(1), abi::return_register(), S_MAP_SIZE),
    ]);
    platform.emit_external_call(
        "munmap",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    Ok((instructions, relocations, F))
}
