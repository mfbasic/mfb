use super::*;

pub(super) enum ThreadSimpleOp {
    IsRunning,
    WaitFor,
    Cancel,
    Drop,
    Poll,
}

pub(super) fn emit_thread_deadline(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    timeout_stack_offset: usize,
    timespec_stack_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let done = format!("{symbol}_deadline_done_{timespec_stack_offset}");
    let nsec_ok = format!("{symbol}_deadline_nsec_ok_{timespec_stack_offset}");
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), timeout_stack_offset),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&done),
        abi::move_immediate(abi::ARG[0], "Integer", "0"),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), timespec_stack_offset),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: instructions,
            relocations: relocations,
        },
        "clock_gettime",
    )?;
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), timeout_stack_offset),
        abi::move_immediate("%v10", "Integer", "1000"),
        abi::signed_divide_registers("%v11", "%v9", "%v10"),
        abi::multiply_subtract_registers("%v12", "%v11", "%v10", "%v9"),
        abi::move_immediate("%v13", "Integer", "1000000"),
        abi::multiply_registers("%v12", "%v12", "%v13"),
        abi::load_u64("%v14", abi::stack_pointer(), timespec_stack_offset),
        abi::add_registers("%v14", "%v14", "%v11"),
        abi::load_u64("%v15", abi::stack_pointer(), timespec_stack_offset + 8),
        abi::add_registers("%v15", "%v15", "%v12"),
        abi::move_immediate("%v13", "Integer", "1000000000"),
        abi::compare_registers("%v15", "%v13"),
        abi::branch_lt(&nsec_ok),
        abi::subtract_registers("%v15", "%v15", "%v13"),
        abi::add_immediate("%v14", "%v14", 1),
        abi::label(&nsec_ok),
        abi::store_u64("%v14", abi::stack_pointer(), timespec_stack_offset),
        abi::store_u64("%v15", abi::stack_pointer(), timespec_stack_offset + 8),
        abi::label(&done),
    ]);
    Ok(())
}

pub(super) fn simple_thread_handle_helper(
    symbol: &str,
    op: ThreadSimpleOp,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 48;
    const HANDLE_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const TAG_OFFSET: usize = 24;
    const ERROR_OFFSET: usize = 32;
    // WaitFor only: origin ErrorLoc of a propagated worker error (0 otherwise).
    const SOURCE_OFFSET: usize = 40;

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([abi::store_u64(
        abi::ARG[0],
        abi::stack_pointer(),
        HANDLE_OFFSET,
    )]);
    match op {
        ThreadSimpleOp::IsRunning => {
            let running = format!("{symbol}_running");
            let closed = format!("{symbol}_closed");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("%v9", abi::ARG[0], THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::ARG[0], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::store_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::compare_immediate("%v9", THREAD_STATE_RUNNING),
                abi::branch_eq(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&done),
                abi::label(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&done),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([abi::label(&done)]);
        }
        ThreadSimpleOp::WaitFor => {
            let loop_label = format!("{symbol}_wait_loop");
            let closed = format!("{symbol}_closed");
            let result_ready = format!("{symbol}_result_ready");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("%v9", abi::ARG[0], THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::ARG[0], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::label(&loop_label),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::compare_immediate("%v9", THREAD_STATE_COMPLETED),
                abi::branch_eq(&result_ready),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
                abi::move_register(abi::ARG[1], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_wait",
            )?;
            instructions.extend([
                abi::branch(&loop_label),
                abi::label(&result_ready),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    "%v8",
                    THREAD_OFFSET_RESULT_ERROR,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, "%v8", THREAD_OFFSET_RESULT_VALUE),
                abi::load_u64(RESULT_TAG_REGISTER, "%v8", THREAD_OFFSET_RESULT_TAG),
                abi::load_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    "%v8",
                    THREAD_OFFSET_RESULT_SOURCE,
                ),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    SOURCE_OFFSET,
                ),
                abi::move_immediate("%v9", "Integer", THREAD_STATE_CLOSED),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_COUNT_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OS_HANDLE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_detach",
            )?;
            instructions.extend([
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::branch(&done),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                // waitFor's own error (resource closed): no worker origin.
                abi::store_u64(abi::ZERO, abi::stack_pointer(), SOURCE_OFFSET),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::label(&done),
                abi::load_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    SOURCE_OFFSET,
                ),
            ]);
        }
        // Close + broadcast both resource-plane queues so a worker parked in a
        // blocking `acceptResource` (or a parent in `transferResource`) re-checks
        // CANCELLED/CLOSED and unblocks. cancel/drop previously touched only the
        // two data-plane queues, so such a worker never woke — a permanent hang and,
        // on drop, a detached leaked thread (bug-205). Mirrors the trampoline-exit
        // close loop; the handle lives at HANDLE_OFFSET on this helper's frame.
        ThreadSimpleOp::Cancel => {
            let closed = format!("{symbol}_closed");
            let closed_unlocked = format!("{symbol}_closed_unlocked");
            let inbound_unlocked = format!("{symbol}_inbound_unlocked");
            instructions.extend([
                abi::load_u64("%v9", abi::ARG[0], THREAD_OFFSET_INBOUND_QUEUE),
                abi::move_register(abi::ARG[0], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::move_immediate("%v9", "Integer", "1"),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::label(&inbound_unlocked),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::ARG[0], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate("%v9", "Integer", "1"),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            // Wake anyone parked on the resource plane too (bug-205).
            emit_close_resource_queues(
                symbol,
                HANDLE_OFFSET,
                platform_imports,
                platform,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&closed_unlocked),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::label(&closed_unlocked),
            ]);
        }
        ThreadSimpleOp::Drop => {
            let already_closed = format!("{symbol}_already_closed");
            let outbound_unlocked = format!("{symbol}_outbound_unlocked");
            let inbound_unlocked = format!("{symbol}_inbound_unlocked");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("%v9", abi::ARG[0], THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::ARG[0], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::store_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&already_closed),
                abi::move_immediate("%v9", "Integer", THREAD_STATE_CLOSED),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_STATE),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_TAIL_OFFSET),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.push(abi::label(&already_closed));
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64("%v9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
                abi::branch_eq(&done),
                abi::label(&outbound_unlocked),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::move_register(abi::ARG[0], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate("%v9", "Integer", "1"),
                abi::store_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64(abi::ZERO, "%v10", THREAD_QUEUE_TAIL_OFFSET),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_broadcast",
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.push(abi::label(&inbound_unlocked));
            // Wake anyone parked on the resource plane before detaching, or a worker
            // blocked in acceptResource never observes CANCELLED and the detached
            // thread leaks forever (bug-205).
            emit_close_resource_queues(
                symbol,
                HANDLE_OFFSET,
                platform_imports,
                platform,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OS_HANDLE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_detach",
            )?;
            instructions.extend([
                abi::label(&done),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            ]);
        }
        ThreadSimpleOp::Poll => {
            let ready = format!("{symbol}_ready");
            let closed = format!("{symbol}_closed");
            let invalid = format!("{symbol}_invalid_timeout");
            let wait_loop = format!("{symbol}_wait_loop");
            let wait_timed = format!("{symbol}_wait_timed");
            let not_ready = format!("{symbol}_not_ready");
            let locked_done = format!("{symbol}_locked_done");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::compare_immediate(abi::ARG[1], "0"),
                abi::branch_lt(&invalid),
                abi::store_u64(abi::ARG[1], abi::stack_pointer(), VALUE_OFFSET),
            ]);
            emit_thread_deadline(
                symbol,
                platform_imports,
                platform,
                VALUE_OFFSET,
                ERROR_OFFSET,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register(abi::ARG[0], "%v9"),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_lock",
            )?;
            instructions.extend([
                abi::label(&wait_loop),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("%v9", "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v10", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
                abi::compare_immediate("%v10", "0"),
                abi::branch_gt(&ready),
                abi::load_u64("%v10", "%v8", THREAD_OFFSET_STATE),
                abi::compare_immediate("%v10", THREAD_STATE_COMPLETED),
                abi::branch_eq(&not_ready),
                abi::load_u64("%v10", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("%v10", "0"),
                abi::branch_gt(&wait_timed),
                abi::branch(&not_ready),
                abi::label(&wait_timed),
                abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
                abi::move_register(abi::ARG[1], "%v9"),
                abi::add_immediate(abi::ARG[2], abi::stack_pointer(), ERROR_OFFSET),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_cond_timedwait",
            )?;
            instructions.extend([
                abi::compare_immediate(abi::RET[0], "0"),
                abi::branch_ne(&not_ready),
                abi::branch(&wait_loop),
                abi::label(&ready),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&locked_done),
                abi::label(&not_ready),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&locked_done),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::branch(&locked_done),
                abi::label(&invalid),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_INVALID_ARGUMENT_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::branch(&done),
                abi::label(&locked_done),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(abi::ARG[0], "%v8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                &mut EmitCtx {
                    symbol: symbol,
                    platform_imports,
                    platform,
                    instructions: &mut instructions,
                    relocations: &mut relocations,
                },
                "pthread_mutex_unlock",
            )?;
            instructions.extend([
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::label(&done),
            ]);
        }
    }
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn thread_queue_write_helper(
    symbol: &str,
    queue_offset: usize,
    parent_send: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 80;
    const HANDLE_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 16;
    const TIMEOUT_OFFSET: usize = 24;
    const QUEUE_OFFSET: usize = 32;
    const TIMESPEC_OFFSET: usize = 40;
    // Byte size of the message copy (arg 3), so a failed send can record it on the
    // pending-free list for the destination to reclaim (bug-147.5b). Must sit past
    // the 16-byte timespec at [40, 56): `emit_thread_deadline` writes tv_nsec at
    // TIMESPEC_OFFSET+8 (=48) and `clock_gettime` writes all 16 bytes, so a size
    // field at 48 would be clobbered by the deadline before the failed-send path
    // reloads it (bug-163).
    const DATA_SIZE_OFFSET: usize = 56;

    let invalid = format!("{symbol}_invalid");
    let closed = format!("{symbol}_closed");
    let interrupted = format!("{symbol}_interrupted");
    let timeout = format!("{symbol}_timeout");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_timed = format!("{symbol}_wait_timed");
    let enqueue = format!("{symbol}_enqueue");
    let tail_wrap = format!("{symbol}_tail_wrap");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::ARG[0], abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), DATA_SIZE_OFFSET),
        abi::compare_immediate(abi::ARG[2], "0"),
        abi::branch_lt(&invalid),
    ]);
    if !parent_send {
        // Re-establish the current-thread register `x20` from the worker's own
        // control block (`x0`) rather than asserting equality; see the matching
        // note in `thread_queue_read_helper`.
        instructions.push(abi::move_register(abi::CURRENT_THREAD, abi::ARG[0]));
    }
    emit_thread_deadline(
        symbol,
        platform_imports,
        platform,
        TIMEOUT_OFFSET,
        TIMESPEC_OFFSET,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
        abi::load_u64("%v9", "%v8", queue_offset),
        abi::store_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::move_register(abi::ARG[0], "%v9"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    instructions.push(abi::label(&wait_loop));
    if parent_send {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v9", "%v8", THREAD_OFFSET_STATE),
            abi::compare_immediate("%v9", THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate("%v9", THREAD_STATE_COMPLETED),
            abi::branch_eq(&interrupted),
            abi::load_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("%v9", "0"),
            abi::branch_ne(&interrupted),
        ]);
    } else {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v9", "%v8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("%v9", "0"),
            abi::branch_ne(&interrupted),
        ]);
    }
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&interrupted),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_lt(&enqueue),
        abi::load_u64("%v12", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("%v12", "0"),
        abi::branch_eq(&timeout),
        abi::label(&wait_timed),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_FULL_OFFSET),
        abi::move_register(abi::ARG[1], "%v9"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), TIMESPEC_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_timedwait",
    )?;
    instructions.extend([
        abi::compare_immediate(abi::RET[0], "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        abi::label(&enqueue),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate("%v12", "%v10", 3),
        abi::add_registers("%v11", "%v11", "%v12"),
        abi::load_u64("%v12", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("%v12", "%v11", 0),
        abi::add_immediate("%v10", "%v10", 1),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_lt(&tail_wrap),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&tail_wrap),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::add_immediate("%v10", "%v10", 1),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_signal",
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&interrupted),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&timeout),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_TIMEOUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    let skip_orphan_push = format!("{symbol}_skip_orphan_push");
    instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        // bug-147.5b: a failed send (tag != Ok) leaves the message copy orphaned in
        // the DESTINATION arena. Still holding the queue mutex, push it onto the
        // queue's pending-free list — reusing the dead block's own first two words as
        // `{next, size}` — so the destination reclaims it (in its own arena) on its
        // next read. `DATA_OFFSET` still holds the copy pointer here (it is reused as
        // a result-register spill slot only below). The result registers are live, so
        // scratch stays in %v8-%v11.
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&skip_orphan_push),
        // A size of 0 means the caller did not hand us a reclaimable block (a scalar
        // message with no copy, or a type whose exact copy size we do not compute) —
        // skip the push and let it leak (bounded, reclaimed at worker teardown)
        // rather than risk a wrong-size `arena_free`.
        abi::load_u64("%v10", abi::stack_pointer(), DATA_SIZE_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&skip_orphan_push),
        abi::load_u64("%v8", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v9", abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64("%v11", "%v8", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::store_u64("%v11", "%v9", 0),
        abi::store_u64("%v10", "%v9", 8),
        abi::store_u64("%v9", "%v8", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::label(&skip_orphan_push),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
        abi::load_u64(abi::ARG[0], abi::stack_pointer(), QUEUE_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
        abi::label(&done),
    ]);
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

/// How a queue-read helper treats its caller. The read machinery is shared by the
/// data plane (`receive`/`read`) and the resource plane
/// (`acceptResource`/`readResource`); the only difference is whether the caller's
/// `x0` is its own control block, so the helper may re-establish the current-thread
/// register `x20` and consult the worker's cancellation flag (`WorkerSelf`); a
/// parent caller must do neither because `x0` is the *worker's* block and clobbering
/// `x20` would corrupt the parent thread. A parent caller instead checks the
/// worker's run state for termination.
///
/// bug-181: both modes are waitable. The no-arg `receive`/`accept` overload passes
/// the block sentinel (`THREAD_RECEIVE_BLOCK_SENTINEL`, i64::MIN) and waits
/// indefinitely; any other negative `timeoutMs` is rejected with
/// `ErrInvalidArgument`. A parent's indefinite wait is terminated when the worker
/// completes or closes the queue (the trampoline broadcasts the queue's condvar on
/// exit), so it never deadlocks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ThreadReadMode {
    /// Worker reading its own queue (`receive`, `acceptResource`): re-establish
    /// `x20` and check the worker cancel flag.
    WorkerSelf,
    /// Parent reading a worker queue (`read`, `readResource`): no `x20` touch,
    /// check the worker's run state for termination.
    Parent,
}

pub(super) fn thread_queue_read_helper(
    symbol: &str,
    queue_offset: usize,
    mode: ThreadReadMode,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    // `WorkerSelf` callers pass their own control block, so the helper restores
    // `x20` and reads the worker cancel flag; parent callers do neither.
    let worker_self = mode == ThreadReadMode::WorkerSelf;
    const FRAME_SIZE: usize = 80;
    const HANDLE_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    const QUEUE_OFFSET: usize = 24;
    const VALUE_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const ERROR_OFFSET: usize = 48;
    const TIMESPEC_OFFSET: usize = 56;

    let invalid = format!("{symbol}_invalid");
    let found = format!("{symbol}_found");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_timed = format!("{symbol}_wait_timed");
    let wait_indefinite = format!("{symbol}_wait_indefinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let not_found = format!("{symbol}_not_found");
    let interrupted = format!("{symbol}_interrupted");
    let closed = format!("{symbol}_closed");
    let timeout = format!("{symbol}_timeout");
    let head_wrap = format!("{symbol}_head_wrap");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::ARG[0], abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    if worker_self {
        // The caller's `x0` is this worker's own control block (the handle is
        // unforgeable in type-correct code). Re-establish the current-thread
        // register `x20` from it rather than asserting equality: arbitrary
        // generated code between worker ops (e.g. arena allocation) may clobber
        // `x20`, so we restore the invariant here instead of failing on it.
        instructions.push(abi::move_register(abi::CURRENT_THREAD, abi::ARG[0]));
    }
    // bug-181: the no-arg `receive`/`accept` overload passes the block sentinel
    // (i64::MIN) to wait indefinitely; a non-negative `timeoutMs` is a real timeout
    // (0 = poll, N = wait N ms). Any other negative value is an explicit user
    // timeout below zero and is rejected with `ErrInvalidArgument`.
    instructions.extend([
        abi::compare_immediate(abi::ARG[1], "0"),
        abi::branch_ge(&timeout_ok),
        abi::move_immediate("%v9", "Integer", THREAD_RECEIVE_BLOCK_SENTINEL),
        abi::compare_registers(abi::ARG[1], "%v9"),
        abi::branch_ne(&invalid),
        abi::label(&timeout_ok),
    ]);
    emit_thread_deadline(
        symbol,
        platform_imports,
        platform,
        TIMEOUT_OFFSET,
        TIMESPEC_OFFSET,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
        abi::load_u64("%v9", "%v8", queue_offset),
        abi::store_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::move_register(abi::ARG[0], "%v9"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_lock",
    )?;
    // bug-147.5b: drain the queue's pending-free list — the message copies a failed
    // send orphaned in THIS thread's arena. We hold the queue mutex and run in the
    // owning thread's arena (x19), so `arena_free` here reclaims each copy on the
    // owning thread with no cross-thread race. Each node stores `{next, size}` in its
    // own first two words; the queue pointer is reloaded from its frame slot every
    // iteration because `arena_free` clobbers caller-saved registers.
    let drain_loop = format!("{symbol}_pending_free_drain");
    let drain_done = format!("{symbol}_pending_free_done");
    instructions.extend([
        abi::label(&drain_loop),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&drain_done),
        abi::load_u64("%v11", "%v10", 0),
        abi::store_u64("%v11", "%v9", THREAD_QUEUE_PENDING_FREE_OFFSET),
        abi::load_u64(abi::ARG[1], "%v10", 8),
        abi::move_register(abi::ARG[0], "%v10"),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    instructions.extend([abi::branch(&drain_loop), abi::label(&drain_done)]);
    instructions.extend([
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_gt(&found),
    ]);
    if worker_self {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v10", "%v8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("%v10", "0"),
            abi::branch_ne(&interrupted),
        ]);
    }
    instructions.extend([
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&not_found),
    ]);
    if !worker_self {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("%v10", "%v8", THREAD_OFFSET_STATE),
            abi::compare_immediate("%v10", THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate("%v10", THREAD_STATE_COMPLETED),
            abi::branch_eq(&not_found),
        ]);
    }
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&not_found),
        abi::branch_lt(&wait_indefinite),
        abi::label(&wait_timed),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_register(abi::ARG[1], "%v9"),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), TIMESPEC_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_timedwait",
    )?;
    instructions.extend([
        abi::compare_immediate(abi::RET[0], "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        abi::label(&wait_indefinite),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_register(abi::ARG[1], "%v9"),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_wait",
    )?;
    instructions.extend([
        abi::branch(&wait_loop),
        abi::label(&found),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate("%v12", "%v10", 3),
        abi::add_registers("%v11", "%v11", "%v12"),
        abi::load_u64(RESULT_VALUE_REGISTER, "%v11", 0),
        abi::add_immediate("%v10", "%v10", 1),
        abi::load_u64("%v11", "%v9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_lt(&head_wrap),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&head_wrap),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::subtract_immediate("%v10", "%v10", 1),
        abi::store_u64("%v10", "%v9", THREAD_QUEUE_COUNT_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::add_immediate(abi::ARG[0], "%v9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_cond_signal",
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_NOT_FOUND_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&interrupted),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&timeout),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_TIMEOUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::load_u64(abi::ARG[0], abi::stack_pointer(), QUEUE_OFFSET),
    ]);
    emit_thread_external_call(
        &mut EmitCtx {
            symbol: symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "pthread_mutex_unlock",
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::label(&done),
    ]);
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
    Ok((frame, instructions, relocations, stack_slots))
}

pub(super) fn thread_is_cancelled_helper() -> HelperBody {
    // Reads the worker's pinned current-thread register `x20` (the thread control
    // block); reserve it so the allocator never colors the `%v9` scratch onto it.
    let cancelled = "_mfb_rt_thread_is_cancelled_true";
    let done = "_mfb_rt_thread_is_cancelled_done";
    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("%v9", abi::CURRENT_THREAD, THREAD_OFFSET_CANCELLED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(cancelled),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(cancelled),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(done),
        abi::return_(),
    ];
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 0);
    (frame, instructions, Vec::new(), stack_slots)
}

/// Close and broadcast both **resource-plane** queues of the thread whose handle
/// lives at `[sp + handle_offset]`, so anyone parked on them re-checks
/// CANCELLED/CLOSED and unblocks.
///
/// `thread::cancel`/`thread::drop` closed and broadcast only the two data-plane
/// queues, so a worker parked in a blocking `acceptResource` waited on the
/// resource-inbound `not_empty` condvar that was never broadcast — it never woke to
/// observe CANCELLED, hanging permanently (and leaking a detached thread on drop).
/// The data-plane `receive` was woken correctly, and the trampoline exit already
/// closes both resource queues "to wake any parent/worker blocked", which is the
/// contract cancel/drop violated (bug-205).
fn emit_close_resource_queues(
    symbol: &str,
    handle_offset: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    for resource_queue_offset in [
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
        THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
    ] {
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64("%v10", "%v8", resource_queue_offset),
            abi::move_register(abi::ARG[0], "%v10"),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: symbol,
                platform_imports,
                platform,
                instructions: instructions,
                relocations: relocations,
            },
            "pthread_mutex_lock",
        )?;
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64("%v10", "%v8", resource_queue_offset),
            abi::move_immediate("%v9", "Integer", "1"),
            abi::store_u64("%v9", "%v10", THREAD_QUEUE_CLOSED_OFFSET),
            abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: symbol,
                platform_imports,
                platform,
                instructions: instructions,
                relocations: relocations,
            },
            "pthread_cond_broadcast",
        )?;
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64("%v10", "%v8", resource_queue_offset),
            abi::add_immediate(abi::ARG[0], "%v10", THREAD_QUEUE_NOT_FULL_OFFSET),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: symbol,
                platform_imports,
                platform,
                instructions: instructions,
                relocations: relocations,
            },
            "pthread_cond_broadcast",
        )?;
        instructions.extend([
            abi::load_u64("%v8", abi::stack_pointer(), handle_offset),
            abi::load_u64(abi::ARG[0], "%v8", resource_queue_offset),
        ]);
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: symbol,
                platform_imports,
                platform,
                instructions: instructions,
                relocations: relocations,
            },
            "pthread_mutex_unlock",
        )?;
    }
    Ok(())
}
