//! macOS AudioQueue C-ABI callbacks the OS invokes on the render/capture threads.

use super::gen_macos_shared::*;
use super::gen_os_seam::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// The runtime symbols whose presence in a plan means an output stream is built,
/// so the AudioQueue output callback (whose address `openOutput` takes) must be
/// emitted. Gated here, next to the emitter, rather than re-derived in
/// `code/mod.rs` (bug-330).
pub(crate) const OUTPUT_CALLBACK_SYMBOLS: &[&str] = &[
    "_mfb_rt_audio_audio_openOutput",
    "_mfb_rt_audio_audio_openOutputDevice",
    "_mfb_rt_audio_audio_write",
    "_mfb_rt_audio_audio_closeOutput",
];

/// The input-stream counterpart of [`OUTPUT_CALLBACK_SYMBOLS`].
pub(crate) const INPUT_CALLBACK_SYMBOLS: &[&str] = &[
    "_mfb_rt_audio_audio_openInput",
    "_mfb_rt_audio_audio_openInputDevice",
    "_mfb_rt_audio_audio_read",
    "_mfb_rt_audio_audio_readTimeout",
    "_mfb_rt_audio_audio_closeInput",
];

/// The AudioQueue output callback (C-ABI): void cb(void* handle, AudioQueueRef,
/// AudioQueueBufferRef). Runs on an ordinary AudioQueue thread, so taking the
/// mutex is legal (plan-33-B §3.1). Marks the played buffer free and signals.
pub(crate) fn lower_audio_output_callback(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const CB_FRAME: usize = 64;
    const CB_HANDLE: usize = 8;
    const CB_BUF: usize = 16;
    const CB_STATE: usize = 24;
    let symbol = AUDIO_OUTPUT_CALLBACK_SYMBOL;
    let ret = format!("{symbol}_ret");
    let no_underrun = format!("{symbol}_no_underrun");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CB_HANDLE),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CB_BUF),
        abi::load_u64(&v9, abi::return_register(), H_STATE),
        abi::store_u64(&v9, abi::stack_pointer(), CB_STATE),
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
        CB_STATE,
        S_MUTEX,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), CB_STATE),
        abi::load_u64(&v9, &v10, S_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&ret),
        abi::load_u64(&v9, &v10, S_FREE_TOP),
        abi::add_immediate(&v11, &v10, S_FREE_BUFS),
        abi::move_immediate(&v12, "Integer", "8"),
        abi::multiply_registers(&v13, &v9, &v12),
        abi::add_registers(&v11, &v11, &v13),
        abi::load_u64(&v14, abi::stack_pointer(), CB_BUF),
        abi::store_u64(&v14, &v11, 0),
        abi::add_immediate(&v9, &v9, 1),
        abi::store_u64(&v9, &v10, S_FREE_TOP),
        abi::compare_immediate(&v9, &NUM_BUFFERS.to_string()),
        abi::branch_lt(&no_underrun),
        abi::load_u64(&v12, &v10, S_STARTED),
        abi::compare_immediate(&v12, "0"),
        abi::branch_eq(&no_underrun),
        abi::load_u64(&v13, &v10, S_XRUNS),
        abi::add_immediate(&v13, &v13, 1),
        abi::store_u64(&v13, &v10, S_XRUNS),
        abi::label(&no_underrun),
        abi::add_immediate(abi::return_register(), &v10, S_COND),
    ]);
    platform.emit_external_call(
        "pthread_cond_signal",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&ret));
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
        CB_STATE,
        S_MUTEX,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], CB_FRAME);
    Ok(CodeFunction {
        name: "runtime.audio.outputCallback".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    })
}

/// The AudioQueue input callback (C-ABI, 6 args): copies the captured buffer into
/// the ring (discarding oldest whole frames on overrun, xruns++), signals, and
/// re-enqueues the buffer.
pub(crate) fn lower_audio_input_callback(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const CB_FRAME: usize = 128;
    const CB_HANDLE: usize = 8;
    const CB_AQ: usize = 16;
    const CB_BUF: usize = 24;
    const CB_STATE: usize = 32;
    const CB_N: usize = 40;
    const CB_SRC: usize = 48;
    let symbol = AUDIO_INPUT_CALLBACK_SYMBOL;
    let closed_exit = format!("{symbol}_closed");
    let no_overrun = format!("{symbol}_no_overrun");
    let copy_loop = format!("{symbol}_copy");
    let copy_done = format!("{symbol}_copy_done");
    let head_ok = format!("{symbol}_head_ok");
    let tail_ok = format!("{symbol}_tail_ok");

    let mut instructions = vec![abi::label("entry")];
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
    let v18 = vregs.next();
    let v19 = vregs.next();
    let v20 = vregs.next();
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CB_HANDLE),
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), CB_AQ),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), CB_BUF),
        abi::load_u64(&v9, abi::return_register(), H_STATE),
        abi::store_u64(&v9, abi::stack_pointer(), CB_STATE),
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
        CB_STATE,
        S_MUTEX,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), CB_STATE),
        abi::load_u64(&v9, &v10, S_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed_exit),
        // n = buffer->mAudioDataByteSize; src = buffer->mAudioData
        abi::load_u64(&v11, abi::stack_pointer(), CB_BUF),
        abi::load_u32(&v9, &v11, 16),
        abi::store_u64(&v9, abi::stack_pointer(), CB_N),
        abi::load_u64(&v12, &v11, 8),
        abi::store_u64(&v12, abi::stack_pointer(), CB_SRC),
        // overrun: if fill + n > ringCap, drop oldest whole frames.
        abi::load_u64(&v13, &v10, S_RING_FILL),
        abi::load_u64(&v14, &v10, S_RING_CAP),
        abi::add_registers(&v15, &v13, &v9), // fill + n
        abi::compare_registers(&v15, &v14),
        abi::branch_le(&no_overrun),
        // drop = (fill+n) - ringCap, rounded up to a whole frame.
        abi::subtract_registers(&v15, &v15, &v14),
        abi::load_u64(&v16, abi::stack_pointer(), CB_HANDLE),
        abi::load_u64(&v16, &v16, H_BYTES_PER_FRAME),
        abi::add_registers(&v15, &v15, &v16),
        abi::subtract_immediate(&v15, &v15, 1),
        abi::unsigned_divide_registers(&v15, &v15, &v16),
        abi::multiply_registers(&v15, &v15, &v16), // drop bytes
        // tail = (tail + drop) mod ringCap
        abi::load_u64(&v17, &v10, S_RING_TAIL),
        abi::add_registers(&v17, &v17, &v15),
        abi::compare_registers(&v17, &v14),
        abi::branch_lt(&tail_ok),
        abi::subtract_registers(&v17, &v17, &v14),
        abi::label(&tail_ok),
        abi::store_u64(&v17, &v10, S_RING_TAIL),
        // fill -= drop
        abi::load_u64(&v13, &v10, S_RING_FILL),
        abi::subtract_registers(&v13, &v13, &v15),
        abi::store_u64(&v13, &v10, S_RING_FILL),
        // xruns++
        abi::load_u64(&v18, &v10, S_XRUNS),
        abi::add_immediate(&v18, &v18, 1),
        abi::store_u64(&v18, &v10, S_XRUNS),
        abi::label(&no_overrun),
        // copy n bytes from src into ring at head, wrapping.
        abi::load_u64(&v9, abi::stack_pointer(), CB_N),
        abi::load_u64(&v12, abi::stack_pointer(), CB_SRC),
        abi::add_immediate(&v19, &v10, S_RING), // ring base
        abi::load_u64(&v16, &v10, S_RING_HEAD),
        abi::load_u64(&v14, &v10, S_RING_CAP),
        abi::move_immediate(&v17, "Integer", "0"), // copied
        abi::label(&copy_loop),
        abi::compare_registers(&v17, &v9),
        abi::branch_ge(&copy_done),
        abi::add_registers(&v18, &v12, &v17), // src+copied
        abi::load_u8(&v20, &v18, 0),
        abi::add_registers(&v18, &v19, &v16), // ring+head
        abi::store_u8(&v20, &v18, 0),
        abi::add_immediate(&v16, &v16, 1),
        abi::compare_registers(&v16, &v14),
        abi::branch_lt(&head_ok),
        abi::move_immediate(&v16, "Integer", "0"),
        abi::label(&head_ok),
        abi::add_immediate(&v17, &v17, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u64(&v16, &v10, S_RING_HEAD),
        // fill += n
        abi::load_u64(&v13, &v10, S_RING_FILL),
        abi::load_u64(&v9, abi::stack_pointer(), CB_N),
        abi::add_registers(&v13, &v13, &v9),
        abi::store_u64(&v13, &v10, S_RING_FILL),
        // signal
        abi::add_immediate(abi::return_register(), &v10, S_COND),
    ]);
    platform.emit_external_call(
        "pthread_cond_signal",
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
        "pthread_mutex_unlock",
        CB_STATE,
        S_MUTEX,
    )?;
    // Re-enqueue the buffer.
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CB_AQ),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), CB_BUF),
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
    instructions.push(abi::return_());
    // Closed path: unlock and return without touching the ring or re-enqueuing.
    instructions.push(abi::label(&closed_exit));
    emit_pthread1(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
        CB_STATE,
        S_MUTEX,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], CB_FRAME);
    Ok(CodeFunction {
        name: "runtime.audio.inputCallback".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    })
}
