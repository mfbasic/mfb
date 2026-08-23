//! ALSA `audio` frame I/O code generation (read/write + available/poll/xruns query).

use super::gen_alsa_shared::*;
use super::gen_common::*;
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

/// write(output, bytes): loop snd_pcm_writei until every frame is accepted,
/// recovering from xruns (§3.5).
pub(crate) fn lower_write(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let ok_frames = format!("{symbol}_ok");
    let recover = format!("{symbol}_recover");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v13 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v14 = vregs.next();
    let v8 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), SRC_OFF), // byteList
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&dev_fail),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v10, abi::return_register(), H_BYTES_PER_FRAME),
        abi::store_u64(&v10, abi::stack_pointer(), BPF_OFF),
        // total bytes, frame-alignment check
        abi::load_u64(&v13, abi::c_arg(1), COLLECTION_OFFSET_COUNT),
        abi::compare_immediate(&v13, "0"),
        abi::branch_eq(&invalid),
        abi::load_u64(&v10, abi::stack_pointer(), BPF_OFF),
        abi::subtract_immediate(&v11, &v10, 1),
        abi::and_registers(&v12, &v13, &v11),
        abi::compare_immediate(&v12, "0"),
        abi::branch_ne(&invalid),
        // src = byteList + HEADER + CAPACITY*ENTRY (the data region starts past
        // the CAPACITY-sized entry array; an append-built list has spare
        // capacity, so COUNT*ENTRY would mis-address it). totalFrames = total/bpf.
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
        abi::store_u64(&v14, abi::stack_pointer(), SRC_OFF),
        abi::unsigned_divide_registers(&v13, &v13, &v10),
        abi::store_u64(&v13, abi::stack_pointer(), TOTAL_OFF), // total frames
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OFFSET_OFF),
    ]);
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
    // cache writei and recover fn-ptrs
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_pcm_writei",
        &unavailable,
    )?;
    instructions.push(abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFF));
    instructions.push(abi::store_u64(&v9, abi::stack_pointer(), FN2_OFF));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_pcm_recover",
        &unavailable,
    )?;
    // (recover fn-ptr stays in FNPTR_OFF)
    instructions.extend([
        abi::label(&loop_top),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), TOTAL_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&loop_done),
        // snd_pcm_writei(pcm, src + offset*bpf, total-offset)
        abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
        abi::load_u64(&v12, abi::stack_pointer(), SRC_OFF),
        abi::load_u64(&v13, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v14, &v9, &v13),
        abi::add_registers(abi::c_arg(1), &v12, &v14),
        abi::subtract_registers(abi::c_arg(2), &v10, &v9),
        abi::load_u64(&v8, abi::stack_pointer(), FN2_OFF),
        abi::branch_link_register(&v8),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&ok_frames),
        // negative: -EINTR retries, else recover.
        abi::move_immediate(&v10, "Integer", EINTR),
        abi::subtract_registers(&v10, abi::ZERO, &v10),
        abi::compare_registers(abi::return_register(), &v10),
        abi::branch_eq(&loop_top),
        abi::branch(&recover),
        abi::label(&ok_frames),
        abi::load_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), N_OFF),
        abi::add_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), OFFSET_OFF),
        abi::branch(&loop_top),
        abi::label(&recover),
        // xruns++ ; snd_pcm_recover(pcm, err, 1)
        abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v12, &v11, S_XRUNS),
        abi::add_immediate(&v12, &v12, 1),
        abi::store_u64(&v12, &v11, S_XRUNS),
        abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), N_OFF), // err
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        abi::load_u64(&v8, abi::stack_pointer(), FNPTR_OFF),
        abi::branch_link_register(&v8),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::branch(&loop_top),
        abi::label(&loop_done),
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
    instructions.push(abi::label(&unavailable));
    emit_fail(
        symbol,
        "ErrAudioUnavailable",
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
    Ok((instructions, relocations, FRAME))
}

/// read(input, frames[, timeoutMs]): loop snd_pcm_readi into the pre-allocated
/// result. The blocking form fills exactly `frames`; the timed form stops at the
/// deadline and returns the whole frames gathered (§3.4).
pub(crate) fn lower_read(
    symbol: &str,
    timeout: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let unavailable = format!("{symbol}_unavailable");
    let dev_fail = format!("{symbol}_dev_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let loop_top = format!("{symbol}_loop");
    let loop_done = format!("{symbol}_loop_done");
    let ok_frames = format!("{symbol}_ok");
    let recover = format!("{symbol}_recover");
    let done = format!("{symbol}_done");
    // Single-pass drain of frames already in the capture ring at the deadline
    // (timed read only; emitted near loop_done). Referenced from both `if timeout`
    // blocks, so declared at function scope.
    let expired_drain = format!("{symbol}_expired_drain");
    let drain_cap = format!("{symbol}_drain_cap");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v8 = vregs.next();
    let v14 = vregs.next();
    let v16 = vregs.next();
    let v17 = vregs.next();
    let v18 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), FRAMES_OFF),
    ]);
    if timeout {
        // Spill `timeoutMs` (ARG[2]) before any dlopen/libc call clobbers it.
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
    emit_alloc_byte_list(
        symbol,
        "main",
        NEED_OFF,
        LIST_OFF,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    // payload base = list + HEADER + need*ENTRY
    instructions.extend([
        abi::load_u64(&v11, abi::stack_pointer(), LIST_OFF),
        abi::load_u64(&v9, abi::stack_pointer(), NEED_OFF),
        abi::move_immediate(&v13, "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers(&v13, &v9, &v13),
        abi::add_immediate(&v13, &v13, COLLECTION_HEADER_SIZE),
        abi::add_registers(&v11, &v11, &v13),
        abi::store_u64(&v11, abi::stack_pointer(), SRC_OFF), // payload base
        abi::store_u64(abi::ZERO, abi::stack_pointer(), FRAMES_GOT_OFF), // frames read
    ]);
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
    if timeout {
        // Cache the poll fn-ptrs (dlsym clobbers FNPTR_OFF, which later holds the
        // recover fn-ptr, so resolve these first) and pin the absolute deadline.
        emit_dlsym(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            "snd_pcm_wait",
            &unavailable,
        )?;
        instructions.push(abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFF));
        instructions.push(abi::store_u64(&v9, abi::stack_pointer(), WAIT_FN_OFF));
        emit_dlsym(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            "snd_pcm_avail_update",
            &unavailable,
        )?;
        instructions.push(abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFF));
        instructions.push(abi::store_u64(&v9, abi::stack_pointer(), AVAIL_FN_OFF));
        // deadline = now + timeoutMs*1e6 (Linux CLOCK_MONOTONIC = 1).
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "1"),
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
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_pcm_readi",
        &unavailable,
    )?;
    instructions.push(abi::load_u64(&v9, abi::stack_pointer(), FNPTR_OFF));
    instructions.push(abi::store_u64(&v9, abi::stack_pointer(), FN2_OFF));
    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "snd_pcm_recover",
        &unavailable,
    )?;
    instructions.extend([
        abi::label(&loop_top),
        abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), FRAMES_OFF),
        abi::compare_registers(&v9, &v10),
        abi::branch_ge(&loop_done),
    ]);
    if timeout {
        // Bound the blocking read by the deadline: on expiry return the partial
        // frames gathered so far; otherwise wait (bounded) for a period and then
        // read only what is available, so `snd_pcm_readi` returns promptly.
        let want_cap = format!("{symbol}_want_cap");
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "1"),
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
            abi::branch_ge(&expired_drain), // expired -> drain buffered, then partial
            // remaining_ms = (deadline - now) / 1e6; sub-ms remaining -> drain.
            abi::subtract_registers(&v12, &v12, &v9),
            abi::move_immediate(&v13, "Integer", "1000000"),
            abi::unsigned_divide_registers(&v13, &v12, &v13),
            abi::compare_immediate(&v13, "0"),
            abi::branch_eq(&expired_drain),
            // snd_pcm_wait(pcm, remaining_ms): 1 ready, 0 timeout, <0 error.
            abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
            abi::move_register(abi::c_arg(1), &v13),
            abi::load_u64(&v8, abi::stack_pointer(), WAIT_FN_OFF),
            abi::branch_link_register(&v8),
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&loop_done), // timeout -> partial
            abi::branch_lt(&recover),   // error (e.g. xrun) -> recover (N_OFF = err)
            // avail = snd_pcm_avail_update(pcm)
            abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
            abi::load_u64(&v8, abi::stack_pointer(), AVAIL_FN_OFF),
            abi::branch_link_register(&v8),
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&recover), // avail error -> recover
            // want = min(frames - got, avail); a zero avail re-arms the wait.
            abi::move_register(&v14, abi::return_register()), // avail frames
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), FRAMES_OFF),
            abi::subtract_registers(&v10, &v10, &v9), // remaining frames
            abi::compare_registers(&v14, &v10),
            abi::branch_ge(&want_cap),
            abi::move_register(&v10, &v14), // want = avail
            abi::label(&want_cap),
            abi::compare_immediate(&v10, "0"),
            abi::branch_eq(&loop_top),
            abi::store_u64(&v10, abi::stack_pointer(), WANT_OFF),
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF), // reload for readi math
        ]);
    }
    instructions.extend([
        // snd_pcm_readi(pcm, payload + got*bpf, <count>)
        abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
        abi::load_u64(&v12, abi::stack_pointer(), SRC_OFF),
        abi::load_u64(&v13, abi::stack_pointer(), BPF_OFF),
        abi::multiply_registers(&v14, &v9, &v13),
        abi::add_registers(abi::c_arg(1), &v12, &v14),
    ]);
    if timeout {
        instructions.push(abi::load_u64(abi::c_arg(2), abi::stack_pointer(), WANT_OFF));
    } else {
        instructions.push(abi::subtract_registers(abi::c_arg(2), &v10, &v9));
    }
    instructions.extend([
        abi::load_u64(&v8, abi::stack_pointer(), FN2_OFF),
        abi::branch_link_register(&v8),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFF),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&ok_frames),
        abi::move_immediate(&v10, "Integer", EINTR),
        abi::subtract_registers(&v10, abi::ZERO, &v10),
        abi::compare_registers(abi::return_register(), &v10),
        abi::branch_eq(&loop_top),
        abi::branch(&recover),
        abi::label(&ok_frames),
        abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::load_u64(&v10, abi::stack_pointer(), N_OFF),
        abi::add_registers(&v9, &v9, &v10),
        abi::store_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
        abi::branch(&loop_top),
        abi::label(&recover),
        abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
        abi::load_u64(&v12, &v11, S_XRUNS),
        abi::add_immediate(&v12, &v12, 1),
        abi::store_u64(&v12, &v11, S_XRUNS),
        abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), N_OFF),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        abi::load_u64(&v8, abi::stack_pointer(), FNPTR_OFF),
        abi::branch_link_register(&v8),
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&dev_fail),
        abi::branch(&loop_top),
    ]);
    if timeout {
        // Deadline reached (including `timeoutMs == 0`): drain whole frames already
        // sitting in the capture ring in ONE non-blocking pass, then return partial.
        // Fixes `audio::read(input, frames, 0)` returning 0 frames when the kernel
        // buffer holds data — matching macOS/Windows, which drain their userspace
        // ring at the deadline. Single pass (-> loop_done, never loop_top) so it can
        // neither re-wait nor loop. Reached only via an unconditional branch above,
        // so it is never fallen into.
        instructions.extend([
            abi::label(&expired_drain),
            // avail = snd_pcm_avail_update(pcm); <= 0 (nothing ready / error) -> partial.
            abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
            abi::load_u64(&v8, abi::stack_pointer(), AVAIL_FN_OFF),
            abi::branch_link_register(&v8),
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_le(&loop_done),
            // want = min(avail, frames - got); a zero want -> partial.
            abi::move_register(&v14, abi::return_register()),
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), FRAMES_OFF),
            abi::subtract_registers(&v10, &v10, &v9),
            abi::compare_registers(&v14, &v10),
            abi::branch_ge(&drain_cap),
            abi::move_register(&v10, &v14),
            abi::label(&drain_cap),
            abi::compare_immediate(&v10, "0"),
            abi::branch_eq(&loop_done),
            abi::store_u64(&v10, abi::stack_pointer(), WANT_OFF),
            // snd_pcm_readi(pcm, payload + got*bpf, want) — one shot.
            abi::load_u64(&v11, abi::stack_pointer(), STATE_OFF),
            abi::load_u64(abi::return_register(), &v11, S_OSOBJECT),
            abi::load_u64(&v12, abi::stack_pointer(), SRC_OFF),
            abi::load_u64(&v13, abi::stack_pointer(), BPF_OFF),
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::multiply_registers(&v14, &v9, &v13),
            abi::add_registers(abi::c_arg(1), &v12, &v14),
            abi::load_u64(abi::c_arg(2), abi::stack_pointer(), WANT_OFF),
            abi::load_u64(&v8, abi::stack_pointer(), FN2_OFF),
            abi::branch_link_register(&v8),
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&loop_done), // readi error at expiry -> return partial
            // got += n
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::add_registers(&v9, &v9, abi::return_register()),
            abi::store_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::branch(&loop_done),
        ]);
    }
    instructions.push(abi::label(&loop_done));
    if timeout {
        // Partial timed read: if fewer than `frames` gathered, return a
        // right-sized list of `got` frames and free the oversized pre-alloc.
        let ret_full = format!("{symbol}_ret_full");
        let fin_loop = format!("{symbol}_fin");
        let fin_done = format!("{symbol}_fin_done");
        instructions.extend([
            abi::load_u64(&v9, abi::stack_pointer(), FRAMES_GOT_OFF),
            abi::load_u64(&v10, abi::stack_pointer(), FRAMES_OFF),
            abi::compare_registers(&v9, &v10),
            abi::branch_ge(&ret_full),
            abi::load_u64(&v13, abi::stack_pointer(), BPF_OFF),
            abi::multiply_registers(&v9, &v9, &v13), // gotBytes = got * bpf
            abi::store_u64(&v9, abi::stack_pointer(), GOTBYTES_OFF),
        ]);
        emit_alloc_byte_list(
            symbol,
            "final",
            GOTBYTES_OFF,
            FINAL_LIST_OFF,
            &alloc_fail,
            &mut instructions,
            &mut relocations,
        );
        instructions.extend([
            // copy gotBytes from the oversized payload into the final payload.
            abi::load_u64(&v9, abi::stack_pointer(), GOTBYTES_OFF),
            abi::load_u64(&v11, abi::stack_pointer(), FINAL_LIST_OFF),
            abi::move_immediate(&v13, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v13, &v9, &v13),
            abi::add_immediate(&v13, &v13, COLLECTION_HEADER_SIZE),
            abi::add_registers(&v11, &v11, &v13), // final payload
            abi::load_u64(&v12, abi::stack_pointer(), SRC_OFF), // source payload
            abi::move_immediate(&v16, "Integer", "0"),
            abi::label(&fin_loop),
            abi::compare_registers(&v16, &v9),
            abi::branch_ge(&fin_done),
            abi::add_registers(&v17, &v12, &v16),
            abi::load_u8(&v18, &v17, 0),
            abi::add_registers(&v17, &v11, &v16),
            abi::store_u8(&v18, &v17, 0),
            abi::add_immediate(&v16, &v16, 1),
            abi::branch(&fin_loop),
            abi::label(&fin_done),
            // Return the oversized pre-alloc to the arena (size matches
            // emit_alloc_byte_list: need*ENTRY + HEADER + need).
            abi::load_u64(&v9, abi::stack_pointer(), NEED_OFF),
            abi::move_immediate(&v10, "Integer", &byte_list_entry_stride().to_string()),
            abi::multiply_registers(&v11, &v9, &v10),
            abi::add_immediate(&v11, &v11, COLLECTION_HEADER_SIZE),
            abi::add_registers(&v11, &v11, &v9),
            abi::move_register(abi::c_arg(1), &v11),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), LIST_OFF),
        ]);
        emit_arena_free(symbol, &mut instructions, &mut relocations);
        instructions.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), FINAL_LIST_OFF),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&ret_full),
        ]);
    }
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFF),
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
    Ok((instructions, relocations, FRAME))
}

/// available/poll/xruns via snd_pcm_avail_update / snd_pcm_wait / the xruns
/// counter (§3.4).
pub(crate) fn lower_query(
    symbol: &str,
    kind: Query,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<AudioBodyParts, String> {
    let unavailable = format!("{symbol}_unavailable");
    let invalid = format!("{symbol}_invalid");
    let closed = format!("{symbol}_closed");
    let clamp = format!("{symbol}_clamp");
    let done = format!("{symbol}_done");
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v12 = vregs.next();
    let v11 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE_OFF),
        // Spill the incoming `timeoutMs` (ARG[1]) before any dlopen/libc call
        // clobbers it; the `PollTimeout` arm reloads it from `FRAMES_OFF` as the
        // `snd_pcm_wait` timeout. Without this store that slot is uninitialized
        // stack (bug-167 finding A). `FRAMES_OFF` is otherwise unused in this
        // function, so the store is harmless for the other queries.
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), FRAMES_OFF),
        abi::load_u64(&v9, abi::return_register(), H_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        abi::load_u64(&v10, abi::return_register(), H_STATE),
        abi::store_u64(&v10, abi::stack_pointer(), STATE_OFF),
    ]);
    match kind {
        Query::Xruns => {
            instructions.extend([
                abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                abi::load_u64(RESULT_VALUE_REGISTER, &v10, S_XRUNS),
                // No `branch(&clamp)` here: the label is the very next
                // instruction, so the branch only ever fell through to its own
                // target (bug-326-A22). The other two arms reach `clamp` from a
                // real conditional.
                abi::label(&clamp),
            ]);
        }
        Query::Available => {
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
            emit_alsa_call(
                &mut vregs,
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "snd_pcm_avail_update",
                &unavailable,
                false,
                |ins, _relocs| {
                    ins.extend([
                        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
                    ]);
                },
            )?;
            // clamp negative to 0
            instructions.extend([
                abi::move_register(&v12, abi::return_register()),
                abi::compare_immediate(&v12, "0"),
                abi::branch_ge(&clamp),
                abi::move_immediate(&v12, "Integer", "0"),
                abi::label(&clamp),
                abi::move_register(RESULT_VALUE_REGISTER, &v12),
            ]);
        }
        Query::Poll => {
            // plan-73-B: omit=block — `snd_pcm_wait(pcm, -1)` blocks indefinitely
            // until the PCM is ready for I/O, then return TRUE (the convention's
            // readiness-query omit rule). `-1` is the infinite timeout, formed with
            // bitwise-not of zero (encoder-safe, no negative immediate). Callers
            // wanting the old immediate check pass `, 0` (pollTimeout).
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
            emit_alsa_call(
                &mut vregs,
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "snd_pcm_wait",
                &unavailable,
                false,
                |ins, _relocs| {
                    ins.extend([
                        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
                        abi::bitwise_not(abi::c_arg(1), abi::ZERO), // -1 = infinite
                    ]);
                },
            )?;
            // snd_pcm_wait returns 1 = ready (for an infinite wait it blocks until
            // then); < 0 = error → FALSE.
            let set = format!("{symbol}_poll_set");
            instructions.extend([
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::compare_immediate(abi::return_register(), "1"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&set),
            ]);
        }
        Query::PollTimeout => {
            // plan-73-B: reject a negative `timeoutMs` (ErrInvalidArgument); clamp a
            // too-large one to INT_MAX (`snd_pcm_wait` takes a C `int`). The value
            // was spilled to FRAMES_OFF at entry; store the clamped value back so the
            // wait call reloads it.
            let timeout_clamped = format!("{symbol}_pt_clamped");
            instructions.extend([
                abi::load_u64(&v9, abi::stack_pointer(), FRAMES_OFF),
                abi::compare_immediate(&v9, "0"),
                abi::branch_lt(&invalid),
                abi::move_immediate(&v11, "Integer", TIMEOUT_CLAMP_MS),
                abi::compare_registers(&v9, &v11),
                abi::branch_le(&timeout_clamped),
                abi::move_register(&v9, &v11),
                abi::label(&timeout_clamped),
                abi::store_u64(&v9, abi::stack_pointer(), FRAMES_OFF),
            ]);
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
            emit_alsa_call(
                &mut vregs,
                &mut EmitCtx {
                    symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "snd_pcm_wait",
                &unavailable,
                false,
                |ins, _relocs| {
                    ins.extend([
                        abi::load_u64(&v10, abi::stack_pointer(), STATE_OFF),
                        abi::load_u64(abi::return_register(), &v10, S_OSOBJECT),
                        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), FRAMES_OFF),
                    ]);
                },
            )?;
            // snd_pcm_wait returns 1 ready, 0 timeout, <0 error → Boolean(>0)
            let set = format!("{symbol}_pt_set");
            instructions.extend([
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
                abi::compare_immediate(abi::return_register(), "1"),
                abi::branch_ne(&set),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
                abi::label(&set),
                abi::label(&clamp),
            ]);
        }
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
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
    // plan-73-B: negative `timeoutMs` on `pollTimeout` → ErrInvalidArgument. Only
    // `PollTimeout` branches here, so emitting the path for the other queries would
    // needlessly perturb their byte-identical codegen.
    if matches!(kind, Query::PollTimeout) {
        instructions.push(abi::label(&invalid));
        emit_fail(
            symbol,
            "ErrInvalidArgument",
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME))
}
