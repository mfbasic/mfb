//! macOS `audio` frame I/O code generation (read/write + available/poll/xruns query).

use super::gen_common::*;
use super::gen_macos_shared::*;
use super::gen_os_seam::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::emit_arena_free;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// write(output, bytes): block until every byte is queued for playback.
pub(crate) fn lower_write(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_ready = format!("{symbol}_wait_ready");
    let have_buf = format!("{symbol}_have_buf");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let cap_ok = format!("{symbol}_cap_ok");
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
    let v16 = vregs.next();
    let v17 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), DEVID_OFF), // byteList ptr
        // Guard write-after-close via the arena-resident mirror (state may be
        // unmapped): if handle->H_CLOSED, raise.
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64(&v10, abi::stack_pointer(), BPF_OFF),
        abi::load_u64(&v11, abi::return_register(), H_BUFFER_FRAMES),
        abi::multiply_registers(&v12, &v11, &v10),
        abi::store_u64(&v12, abi::stack_pointer(), CAP_OFF),
        abi::load_u64(&v13, abi::c_arg(1), COLLECTION_OFFSET_COUNT),
        abi::store_u64(&v13, abi::stack_pointer(), TOTAL_OFF),
        // The byte payload starts past the CAPACITY-sized entry array, not the
        // COUNT-sized one: an append-built list carries spare capacity, so
        // HEADER + CAPACITY*ENTRY is the data-region base (byteList + count*ENTRY
        // would land in the middle of the entry array — bug: static playback).
    ]);
    push_collection_data_base_from_capacity(
        &mut instructions,
        &v14,
        abi::c_arg(1),
        &v12,
        &v14,
        &v14,
    );
    instructions.extend([
        abi::store_u64(&v14, abi::stack_pointer(), QUEUE_OFF), // src base
        abi::compare_immediate(&v13, "0"),
        abi::branch_eq(&invalid),
        abi::load_u64(&v10, abi::stack_pointer(), BPF_OFF),
        abi::subtract_immediate(&v10, &v10, 1), // mask = bpf-1
        abi::and_registers(&v11, &v13, &v10),
        abi::compare_immediate(&v11, "0"),
        abi::branch_ne(&invalid),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF),
        // Resume the buffer a previous write left part-filled, if any.
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v10, S_PENDING_FILL),
        abi::store_u64(&v9, abi::stack_pointer(), FILL_OFF),
        abi::load_u64(&v9, &v10, S_PENDING_BUF),
        abi::store_u64(&v9, abi::stack_pointer(), BUFPTR_OFF),
        abi::label(&write_loop),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&write_done),
        // A part-filled buffer is already in hand; only take a new one when it
        // is not.
        abi::load_u64(&v9, abi::stack_pointer(), FILL_OFF),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&have_buf),
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
        abi::label(&wait_loop),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v10, S_FREE_TOP),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&wait_ready),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
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
        abi::branch(&wait_loop),
        abi::label(&wait_ready),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v10, S_FREE_TOP),
        abi::subtract_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, &v10, S_FREE_TOP),
        abi::add_immediate(&v11, &v10, S_FREE_BUFS),
        abi::move_immediate(&v12, "Integer", "8"),
        abi::multiply_registers(&v13, &v9, &v12),
        abi::add_registers(&v11, &v11, &v13),
        abi::load_u64(&v14, &v11, 0),
        abi::store_u64(&v14, abi::stack_pointer(), BUFPTR_OFF),
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
        // n = min(total - offset, cap - fill)
        abi::label(&have_buf),
        abi::load_u64(&v9, abi::stack_pointer(), TOTAL_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), OFFSET_OFF),
        abi::subtract_registers(&v9, &v9, &v10),
        abi::load_u64(&v11, abi::stack_pointer(), CAP_OFF),
        abi::load_u64(&v12, abi::stack_pointer(), FILL_OFF),
        abi::subtract_registers(&v11, &v11, &v12), // room left in the buffer
        abi::compare_registers(&v9, &v11),
        abi::branch_le(&cap_ok),
        abi::move_register(&v9, &v11),
        abi::label(&cap_ok),
        abi::store_u64(&v9, abi::stack_pointer(), I_OFF), // n
        abi::load_u64(&v12, abi::stack_pointer(), QUEUE_OFF),
        abi::load_u64(&v13, abi::stack_pointer(), OFFSET_OFF),
        abi::add_registers(&v12, &v12, &v13), // src
        abi::load_u64(&v14, abi::stack_pointer(), BUFPTR_OFF),
        abi::load_u64(&v15, &v14, 8), // mAudioData
        abi::load_u64(&v16, abi::stack_pointer(), FILL_OFF),
        abi::add_registers(&v15, &v15, &v16), // append after what is there
        abi::move_immediate(&v16, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v16, &v9),
        abi::branch_ge(&copy_done),
        abi::load_u8(&v17, &v12, 0),
        abi::store_u8(&v17, &v15, 0),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v15, &v15, 1),
        abi::add_immediate(&v16, &v16, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        // fill += n; offset += n
        abi::load_u64(&v9, abi::stack_pointer(), FILL_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), I_OFF),
        abi::add_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), FILL_OFF),
        abi::load_u64(&v11, abi::stack_pointer(), OFFSET_OFF),
        abi::add_registers(&v11, &v11, &v10),
        abi::store_u64(&v11, abi::stack_pointer(), OFFSET_OFF),
        // Only a full buffer may be enqueued: the queue never finishes a short
        // one (bug-370). A partial tail stays in hand for the next write, or for
        // close to pad with silence.
        abi::load_u64(&v11, abi::stack_pointer(), CAP_OFF),
        abi::compare_registers(&v9, &v11),
        abi::branch_lt(&write_loop),
        abi::load_u64(&v14, abi::stack_pointer(), BUFPTR_OFF),
        abi::store_u32(&v11, &v14, 16), // mAudioDataByteSize = cap
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
        // Enqueued, so nothing is in hand any more.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), FILL_OFF),
        abi::branch(&write_loop),
        abi::label(&write_done),
        // Hand the part-filled buffer (if any) to the next write or to close.
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), FILL_OFF),
        abi::store_u64(&v9, &v10, S_PENDING_FILL),
        abi::load_u64(&v9, abi::stack_pointer(), BUFPTR_OFF),
        abi::store_u64(&v9, &v10, S_PENDING_BUF),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
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
    emit_fail(
        symbol,
        "ErrAudioDevice",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, F))
}

/// available/poll/xruns(stream): read the mutex-guarded counters, branching on
/// handle->kind. Output uses free_top*bufferFrames; input the ring (lands with
/// the input phase).
pub(crate) fn lower_query(
    symbol: &str,
    kind: Query,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let is_input = format!("{symbol}_input");
    let have = format!("{symbol}_have");
    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let done = format!("{symbol}_done");
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v11 = vregs.next();
    let v10 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        HANDLE_OFF,
    ));
    if let Query::PollTimeout = kind {
        // Spill `timeoutMs` (ARG[1]) before any libc call clobbers it.
        instructions.push(abi::store_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            TIMEOUT_OFF,
        ));
        // plan-73-B: reject a negative `timeoutMs` (ErrInvalidArgument); clamp a
        // too-large one to INT_MAX, storing the clamped value back for the deadline.
        let timeout_clamped = format!("{symbol}_pt_clamped");
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFF),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&invalid),
            abi::move_immediate(&v11, "Integer", TIMEOUT_CLAMP_MS),
            abi::compare_registers(&v9, &v11),
            abi::branch_le(&timeout_clamped),
            abi::move_register(&v9, &v11),
            abi::label(&timeout_clamped),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFF),
        ]);
    }
    instructions.extend([
        // Closed-resource guard: a defaulted/closed handle has an invalid (null)
        // state page, so return the empty answer (0 / FALSE) without locking it.
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
    ]);
    if let Query::Poll = kind {
        // plan-73-B: omit=block — poll(stream) waits indefinitely until the stream
        // is ready (input: ring fill; output: a free buffer), then returns TRUE (the
        // convention's readiness-query omit rule). Infinite `pthread_cond_wait`, the
        // same primitive the blocking read/write use; callers wanting the old
        // immediate check pass `, 0` (pollTimeout).
        let poll_loop = format!("{symbol}_poll_loop");
        let poll_ready = format!("{symbol}_poll_ready");
        let poll_input = format!("{symbol}_poll_input");
        let poll_have = format!("{symbol}_poll_have");
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
            abi::label(&poll_loop),
            abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v11, &v9, H_KIND),
            abi::compare_immediate(&v11, KIND_INPUT),
            abi::branch_eq(&poll_input),
            abi::load_u64(&v12, &v10, S_FREE_TOP),
            abi::branch(&poll_have),
            abi::label(&poll_input),
            abi::load_u64(&v12, &v10, S_RING_FILL),
            abi::label(&poll_have),
            abi::compare_immediate(&v12, "0"),
            abi::branch_ne(&poll_ready),
            // Not ready: block on the stream condition until the callback signals.
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
        instructions.extend([abi::branch(&poll_loop), abi::label(&poll_ready)]);
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
        instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"));
    } else if let Query::PollTimeout = kind {
        // pollTimeout(input, timeoutMs): wait up to `timeoutMs` for data (input:
        // ring fill; output: a free buffer), returning TRUE the moment it is
        // available and FALSE at the deadline. Mirrors the timed-read wait but
        // yields a Boolean. The result is stashed in I_OFF and loaded into the
        // result register only after the unlock (which clobbers caller-saved).
        let pt_loop = format!("{symbol}_pt_loop");
        let pt_ready = format!("{symbol}_pt_ready");
        let pt_expired = format!("{symbol}_pt_expired");
        let pt_input = format!("{symbol}_pt_input");
        let pt_have = format!("{symbol}_pt_have");
        let pt_result = format!("{symbol}_pt_result");
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
        // deadline = now + timeoutMs*1e6 (CLOCK_MONOTONIC = 6 on macOS).
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_external_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), CLK_OFF),
            abi::move_immediate(&v10, "Integer", "1000000000"),
            abi::multiply_registers(&v9, &v9, &v10),
            abi::load_u64(&v11, abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers(&v9, &v9, &v11),
            abi::load_u64(&v12, abi::stack_pointer(), TIMEOUT_OFF),
            abi::move_immediate(&v13, "Integer", "1000000"),
            abi::multiply_registers(&v12, &v12, &v13),
            abi::add_registers(&v9, &v9, &v12),
            abi::store_u64(&v9, abi::stack_pointer(), DEADLINE_OFF),
            abi::label(&pt_loop),
            // available = input ? S_RING_FILL : S_FREE_TOP (nonzero => ready).
            abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v11, &v9, H_KIND),
            abi::compare_immediate(&v11, KIND_INPUT),
            abi::branch_eq(&pt_input),
            abi::load_u64(&v12, &v10, S_FREE_TOP),
            abi::branch(&pt_have),
            abi::label(&pt_input),
            abi::load_u64(&v12, &v10, S_RING_FILL),
            abi::label(&pt_have),
            abi::compare_immediate(&v12, "0"),
            abi::branch_ne(&pt_ready),
            // No data yet: has the deadline passed?
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_external_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), CLK_OFF),
            abi::move_immediate(&v10, "Integer", "1000000000"),
            abi::multiply_registers(&v9, &v9, &v10),
            abi::load_u64(&v11, abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers(&v9, &v9, &v11), // now
            abi::load_u64(&v12, abi::stack_pointer(), DEADLINE_OFF),
            abi::compare_registers(&v9, &v12),
            abi::branch_ge(&pt_expired),
            // remaining = deadline - now, split into a relative timespec.
            abi::subtract_registers(&v12, &v12, &v9),
            abi::move_immediate(&v13, "Integer", "1000000000"),
            abi::unsigned_divide_registers(&v14, &v12, &v13),
            abi::store_u64(&v14, abi::stack_pointer(), TS_OFF),
            abi::multiply_registers(&v14, &v14, &v13),
            abi::subtract_registers(&v14, &v12, &v14),
            abi::store_u64(&v14, abi::stack_pointer(), TS_OFF + 8),
            // pthread_cond_timedwait_relative_np(cond, mutex, ts)
            abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::return_register(), &v10, S_COND),
            abi::add_immediate(abi::c_arg(1), &v10, S_MUTEX),
            abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), TS_OFF),
        ]);
        platform.emit_external_call(
            "pthread_cond_timedwait_relative_np",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::branch(&pt_loop),
            abi::label(&pt_ready),
            abi::move_immediate(&v9, "Integer", "1"),
            abi::store_u64(&v9, abi::stack_pointer(), I_OFF),
            abi::branch(&pt_result),
            abi::label(&pt_expired),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), I_OFF),
            abi::label(&pt_result),
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
        instructions.push(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            I_OFF,
        ));
    } else {
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
            abi::load_u64(&v9, abi::stack_pointer(), HANDLE_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(&v11, &v9, H_KIND),
            abi::compare_immediate(&v11, KIND_INPUT),
            abi::branch_eq(&is_input),
            abi::load_u64(&v12, &v10, S_FREE_TOP),
            abi::load_u64(&v13, &v9, H_BUFFER_FRAMES),
            abi::multiply_registers(&v12, &v12, &v13),
            abi::branch(&have),
            abi::label(&is_input),
            abi::load_u64(&v12, &v10, S_RING_FILL),
            abi::load_u64(&v13, &v9, H_BYTES_PER_FRAME),
            // frames = fill / bytesPerFrame; bytesPerFrame is 2 (mono) or 4 (stereo),
            // so >>1 then a further >>1 when stereo.
            abi::shift_right_immediate(&v12, &v12, 1),
            abi::compare_immediate(&v13, "2"),
            abi::branch_eq(&have),
            abi::shift_right_immediate(&v12, &v12, 1),
            abi::label(&have),
            abi::store_u64(&v12, abi::stack_pointer(), I_OFF),
            abi::load_u64(&v14, &v10, S_XRUNS),
            abi::store_u64(&v14, abi::stack_pointer(), CAP_OFF),
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
        match kind {
            Query::Available => instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                I_OFF,
            )),
            Query::Xruns => instructions.push(abi::load_u64(
                RESULT_VALUE_REGISTER,
                abi::stack_pointer(),
                CAP_OFF,
            )),
            // plan-73-B: `Query::Poll` now takes the blocking branch above (omit =
            // block), so it never reaches this immediate-read path.
            Query::Poll | Query::PollTimeout => {
                unreachable!("poll/pollTimeout handled above")
            }
        }
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        // Closed handle: the empty answer (available/xruns 0, poll FALSE).
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
    ]);
    // plan-73-B: negative `timeoutMs` on `pollTimeout` → ErrInvalidArgument. Only
    // `PollTimeout` branches here, so guard it to keep the other queries' codegen
    // byte-identical.
    if matches!(kind, Query::PollTimeout) {
        instructions.push(abi::branch(&done));
        instructions.push(abi::label(&invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, F))
}

/// read(input, frames[, timeoutMs]). Drains the ring incrementally into the
/// pre-allocated result across multiple callback fills, so the ring can be small
/// relative to a large read. On timeout expiry, returns the whole frames
/// gathered so far (possibly none) in a right-sized list.
pub(crate) fn lower_read(
    symbol: &str,
    timeout: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let drain_loop = format!("{symbol}_drain");
    let drain_done = format!("{symbol}_drain_done");
    let wait_data = format!("{symbol}_wait");
    let have_data = format!("{symbol}_have");
    let copy_loop = format!("{symbol}_copy");
    let copy_wrap_ok = format!("{symbol}_copy_wrap");
    let copy_done = format!("{symbol}_copy_done");
    let chunk_ok = format!("{symbol}_chunk_ok");
    let ret_full = format!("{symbol}_ret_full");
    let fin_loop = format!("{symbol}_fin");
    let fin_done = format!("{symbol}_fin_done");
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
    let v16 = vregs.next();
    let v20 = vregs.next();
    let v17 = vregs.next();
    let v18 = vregs.next();
    let v19 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), FRAMES_OFF),
    ]);
    if timeout {
        instructions.push(abi::store_u64(
            abi::c_arg(2),
            abi::stack_pointer(),
            TIMEOUT_OFF,
        ));
    }
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64(&v10, abi::stack_pointer(), BPF_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), FRAMES_OFF),
        abi::compare_immediate(&v9, "1"),
        abi::branch_lt(&invalid),
        abi::move_immediate(&v11, "Integer", READ_FRAMES_MAX),
        abi::compare_registers(&v9, &v11),
        abi::branch_gt(&invalid),
        abi::multiply_registers(&v12, &v9, &v10),
        abi::store_u64(&v12, abi::stack_pointer(), NEED_OFF),
    ]);
    if timeout {
        // plan-73-B: reject a negative `timeoutMs` (ErrInvalidArgument); clamp a
        // too-large one to INT_MAX rather than raising, then store the clamped value
        // back for the deadline math below. Mirrors net::poll's clamp.
        let timeout_clamped = format!("{symbol}_timeout_clamped");
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFF),
            abi::compare_immediate(&v9, "0"),
            abi::branch_lt(&invalid),
            abi::move_immediate(&v11, "Integer", TIMEOUT_CLAMP_MS),
            abi::compare_registers(&v9, &v11),
            abi::branch_le(&timeout_clamped),
            abi::move_register(&v9, &v11),
            abi::label(&timeout_clamped),
            abi::store_u64(&v9, abi::stack_pointer(), TIMEOUT_OFF),
        ]);
    }
    // Allocate the result (sized for the full request) before the lock, and cache
    // its payload base. `got` accumulates across ring fills.
    emit_alloc_byte_list(
        symbol,
        "main",
        NEED_OFF,
        LIST_PTR_OFF,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(&v11, abi::stack_pointer(), LIST_PTR_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), NEED_OFF),
        abi::move_immediate(&v13, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v9, &v13),
        abi::add_immediate(&v13, &v13, COLLECTION_HEADER_SIZE),
        abi::add_registers(&v11, &v11, &v13),
        abi::store_u64(&v11, abi::stack_pointer(), PAYLOAD_OFF), // payload base
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BYTES_GOT_OFF),
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
    if timeout {
        // deadline = now + timeoutMs*1e6 (CLOCK_MONOTONIC = 6).
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_external_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), CLK_OFF),
            abi::move_immediate(&v10, "Integer", "1000000000"),
            abi::multiply_registers(&v9, &v9, &v10),
            abi::load_u64(&v11, abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers(&v9, &v9, &v11),
            abi::load_u64(&v12, abi::stack_pointer(), TIMEOUT_OFF),
            abi::move_immediate(&v13, "Integer", "1000000"),
            abi::multiply_registers(&v12, &v12, &v13),
            abi::add_registers(&v9, &v9, &v12),
            abi::store_u64(&v9, abi::stack_pointer(), DEADLINE_OFF),
        ]);
    }
    instructions.extend([
        abi::label(&drain_loop),
        abi::load_u64(&v9, abi::stack_pointer(), BYTES_GOT_OFF),
        abi::load_u64(&v11, abi::stack_pointer(), NEED_OFF),
        abi::compare_registers(&v9, &v11),
        abi::branch_ge(&drain_done),
        // wait for data
        abi::label(&wait_data),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v9, &v10, S_RING_FILL),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&have_data),
    ]);
    if timeout {
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "6"),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), CLK_OFF),
        ]);
        platform.emit_external_call(
            "clock_gettime",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), CLK_OFF),
            abi::move_immediate(&v10, "Integer", "1000000000"),
            abi::multiply_registers(&v9, &v9, &v10),
            abi::load_u64(&v11, abi::stack_pointer(), CLK_OFF + 8),
            abi::add_registers(&v9, &v9, &v11), // now
            abi::load_u64(&v12, abi::stack_pointer(), DEADLINE_OFF),
            abi::compare_registers(&v9, &v12),
            abi::branch_ge(&drain_done),              // expired
            abi::subtract_registers(&v12, &v12, &v9), // remaining
            abi::move_immediate(&v13, "Integer", "1000000000"),
            abi::unsigned_divide_registers(&v14, &v12, &v13),
            abi::store_u64(&v14, abi::stack_pointer(), TS_OFF),
            abi::multiply_registers(&v14, &v14, &v13),
            abi::subtract_registers(&v14, &v12, &v14),
            abi::store_u64(&v14, abi::stack_pointer(), TS_OFF + 8),
            abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
            abi::add_immediate(abi::return_register(), &v10, S_COND),
            abi::add_immediate(abi::c_arg(1), &v10, S_MUTEX),
            abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), TS_OFF),
        ]);
        platform.emit_external_call(
            "pthread_cond_timedwait_relative_np",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::branch(&wait_data));
    } else {
        instructions.extend([
            abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
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
        instructions.push(abi::branch(&wait_data));
    }
    // have_data: chunk = min(need-got, fill); copy chunk bytes from ring[tail] to
    // payload[got], wrapping; advance tail/fill/got.
    instructions.extend([
        abi::label(&have_data),
        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v13, &v10, S_RING_FILL),
        abi::load_u64(&v9, abi::stack_pointer(), NEED_OFF),
        abi::load_u64(&v11, abi::stack_pointer(), BYTES_GOT_OFF),
        abi::subtract_registers(&v9, &v9, &v11), // remaining = need-got
        abi::compare_registers(&v9, &v13),
        abi::branch_le(&chunk_ok),
        abi::move_register(&v9, &v13), // chunk = fill
        abi::label(&chunk_ok),
        // dst = payload + got
        abi::load_u64(&v12, abi::stack_pointer(), PAYLOAD_OFF),
        abi::add_registers(&v12, &v12, &v11),
        abi::add_immediate(&v14, &v10, S_RING), // ring base
        abi::load_u64(&v16, &v10, S_RING_TAIL),
        abi::load_u64(&v20, &v10, S_RING_CAP),
        abi::move_immediate(&v17, "Integer", "0"), // i
        abi::label(&copy_loop),
        abi::compare_registers(&v17, &v9),
        abi::branch_ge(&copy_done),
        abi::add_registers(&v18, &v14, &v16),
        abi::load_u8(&v19, &v18, 0),
        abi::add_registers(&v18, &v12, &v17),
        abi::store_u8(&v19, &v18, 0),
        abi::add_immediate(&v16, &v16, 1),
        abi::compare_registers(&v16, &v20),
        abi::branch_lt(&copy_wrap_ok),
        abi::move_immediate(&v16, "Integer", "0"),
        abi::label(&copy_wrap_ok),
        abi::add_immediate(&v17, &v17, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        // tail = %v16; fill -= chunk; got += chunk
        abi::store_u64(&v16, &v10, S_RING_TAIL),
        abi::load_u64(&v13, &v10, S_RING_FILL),
        abi::subtract_registers(&v13, &v13, &v9),
        abi::store_u64(&v13, &v10, S_RING_FILL),
        abi::load_u64(&v11, abi::stack_pointer(), BYTES_GOT_OFF),
        abi::add_registers(&v11, &v11, &v9),
        abi::store_u64(&v11, abi::stack_pointer(), BYTES_GOT_OFF),
        abi::branch(&drain_loop),
        abi::label(&drain_done),
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
    // If we filled the request, return the pre-allocated result; otherwise (timed
    // partial) build a right-sized list of `got` bytes.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), BYTES_GOT_OFF),
        abi::load_u64(&v11, abi::stack_pointer(), NEED_OFF),
        abi::compare_registers(&v9, &v11),
        abi::branch_ge(&ret_full),
    ]);
    emit_alloc_byte_list(
        symbol,
        "final",
        BYTES_GOT_OFF,
        FINAL_LIST_OFF,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        // copy `got` bytes from the oversized payload into the final payload.
        abi::load_u64(&v9, abi::stack_pointer(), BYTES_GOT_OFF),
        abi::load_u64(&v11, abi::stack_pointer(), FINAL_LIST_OFF),
        abi::move_immediate(&v13, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v9, &v13),
        abi::add_immediate(&v13, &v13, COLLECTION_HEADER_SIZE),
        abi::add_registers(&v11, &v11, &v13), // final payload
        abi::load_u64(&v12, abi::stack_pointer(), PAYLOAD_OFF), // source payload
        abi::move_immediate(&v17, "Integer", "0"),
        abi::label(&fin_loop),
        abi::compare_registers(&v17, &v9),
        abi::branch_ge(&fin_done),
        abi::add_registers(&v18, &v12, &v17),
        abi::load_u8(&v19, &v18, 0),
        abi::add_registers(&v18, &v11, &v17),
        abi::store_u8(&v19, &v18, 0),
        abi::add_immediate(&v17, &v17, 1),
        abi::branch(&fin_loop),
        abi::label(&fin_done),
        // Return the oversized pre-allocated list to the arena — the right-sized
        // `final` list is what we return, so the full `need`-byte block leaks
        // otherwise. size = need*ENTRY + HEADER + need (emit_alloc_byte_list).
        abi::load_u64(&v9, abi::stack_pointer(), NEED_OFF),
        abi::move_immediate(&v10, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v11, &v9, &v10),
        abi::add_immediate(&v11, &v11, COLLECTION_HEADER_SIZE),
        abi::add_registers(&v11, &v11, &v9),
        abi::move_register(abi::c_arg(1), &v11),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LIST_PTR_OFF),
    ]);
    emit_arena_free(symbol, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), FINAL_LIST_OFF),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&ret_full),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_PTR_OFF),
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
