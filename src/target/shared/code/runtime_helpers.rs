use super::*;

pub(super) const THREAD_BLOCK_SIZE: usize = 120;
pub(super) const THREAD_OFFSET_STATE: usize = 0;
pub(super) const THREAD_OFFSET_CANCELLED: usize = 8;
pub(super) const THREAD_OFFSET_RESULT_TAG: usize = 16;
pub(super) const THREAD_OFFSET_RESULT_VALUE: usize = 24;
pub(super) const THREAD_OFFSET_RESULT_ERROR: usize = 32;
pub(super) const THREAD_OFFSET_INBOUND_QUEUE: usize = 40;
pub(super) const THREAD_OFFSET_OUTBOUND_QUEUE: usize = 48;
pub(super) const THREAD_OFFSET_OS_HANDLE: usize = 56;
pub(super) const THREAD_OFFSET_ENTRY: usize = 64;
pub(super) const THREAD_OFFSET_DATA: usize = 72;
pub(super) const THREAD_OFFSET_ARENA_STATE: usize = 80;
pub(super) const THREAD_OFFSET_PARENT_ARENA_STATE: usize = 88;
// Origin `ErrorLoc` pointer of a worker's terminal error, captured by the
// trampoline so `thread::waitFor` can recover the worker's source location.
pub(super) const THREAD_OFFSET_RESULT_SOURCE: usize = 96;
// Resource plane (§7): two dedicated queues for `thread::transfer`/
// `thread::accept`, independent of the data-channel inbound/outbound queues so a
// thread can carry both planes at once. The resource plane mirrors the data
// plane's split: the inbound queue carries parent→worker transfers (parent
// `transfer`, worker `accept`); the outbound queue carries worker→parent
// transfers (worker `transfer`, parent `accept`). Two queues keep the directions
// isolated so a thread's own transfer is never re-read by its own accept.
pub(super) const THREAD_OFFSET_RESOURCE_INBOUND_QUEUE: usize = 104;
pub(super) const THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE: usize = 112;
pub(super) const THREAD_STATE_RUNNING: &str = "0";
pub(super) const THREAD_STATE_COMPLETED: &str = "1";
pub(super) const THREAD_STATE_CLOSED: &str = "2";

pub(super) const THREAD_QUEUE_NOT_EMPTY_OFFSET: usize = 64;
pub(super) const THREAD_QUEUE_NOT_FULL_OFFSET: usize = 128;
pub(super) const THREAD_QUEUE_CAPACITY_OFFSET: usize = 192;
pub(super) const THREAD_QUEUE_COUNT_OFFSET: usize = 200;
pub(super) const THREAD_QUEUE_HEAD_OFFSET: usize = 208;
pub(super) const THREAD_QUEUE_TAIL_OFFSET: usize = 216;
pub(super) const THREAD_QUEUE_CLOSED_OFFSET: usize = 224;
pub(super) const THREAD_QUEUE_VALUES_OFFSET: usize = 232;
pub(super) const THREAD_QUEUE_BLOCK_SIZE: usize = 240;

pub(super) fn thread_symbol(platform: &dyn CodegenPlatform, name: &str) -> String {
    if platform.target() == "macos-aarch64" {
        format!("_{name}")
    } else {
        name.to_string()
    }
}

pub(super) fn emit_thread_external_call(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    name: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let symbol = thread_symbol(platform, name);
    instructions.push(abi::branch_link(&symbol));
    relocations.push(external_branch(from, &symbol, platform_imports)?);
    Ok(())
}

pub(super) fn emit_thread_queue_alloc(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    limit_stack_offset: usize,
    cb_stack_offset: usize,
    queue_stack_offset: usize,
    cb_queue_offset: usize,
    done_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let alloc_queue_ok = format!("{symbol}_queue_{cb_queue_offset}_alloc_ok");
    let alloc_values_ok = format!("{symbol}_queue_{cb_queue_offset}_values_ok");
    let init_error = format!("{symbol}_queue_{cb_queue_offset}_init_error");
    let init_done = format!("{symbol}_queue_{cb_queue_offset}_init_done");

    instructions.extend([
        abi::move_immediate("x0", "Integer", &THREAD_QUEUE_BLOCK_SIZE.to_string()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_queue_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done_label),
        abi::label(&alloc_queue_ok),
        abi::store_u64("x1", abi::stack_pointer(), queue_stack_offset),
        abi::load_u64("x9", abi::stack_pointer(), cb_stack_offset),
        abi::store_u64("x1", "x9", cb_queue_offset),
        abi::load_u64("x10", abi::stack_pointer(), limit_stack_offset),
        abi::store_u64("x10", "x1", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_COUNT_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_HEAD_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_TAIL_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_CLOSED_OFFSET),
        abi::move_immediate("x11", "Integer", "8"),
        abi::multiply_registers("x0", "x10", "x11"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_values_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done_label),
        abi::label(&alloc_values_ok),
        abi::load_u64("x9", abi::stack_pointer(), queue_stack_offset),
        abi::store_u64("x1", "x9", THREAD_QUEUE_VALUES_OFFSET),
        abi::move_register("x0", "x9"),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_mutex_init",
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&init_error),
        abi::load_u64("x9", abi::stack_pointer(), queue_stack_offset),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_init",
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&init_error),
        abi::load_u64("x9", abi::stack_pointer(), queue_stack_offset),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_init",
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&init_error),
        abi::branch(&init_done),
        abi::label(&init_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_INTERRUPTED_SYMBOL, instructions, relocations);
    instructions.push(abi::branch(done_label));
    instructions.push(abi::label(&init_done));
    Ok(())
}

pub(super) fn lower_thread_helper(
    symbol: &str,
    call: &str,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    match call {
        "thread.start" => lower_thread_start_helper(symbol, uses_rng, platform_imports, platform),
        "thread.isRunning" => simple_thread_handle_helper(
            symbol,
            ThreadSimpleOp::IsRunning,
            platform_imports,
            platform,
        ),
        "thread.waitFor" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::WaitFor, platform_imports, platform)
        }
        "thread.cancel" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Cancel, platform_imports, platform)
        }
        "thread.drop" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Drop, platform_imports, platform)
        }
        "thread.send" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_INBOUND_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        "thread.poll" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Poll, platform_imports, platform)
        }
        "thread.read" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_OUTBOUND_QUEUE,
            ThreadReadMode::ParentBounded,
            platform_imports,
            platform,
        ),
        "thread.receive" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_INBOUND_QUEUE,
            ThreadReadMode::WorkerSelf,
            platform_imports,
            platform,
        ),
        "thread.emit" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_OUTBOUND_QUEUE,
            false,
            platform_imports,
            platform,
        ),
        // Resource plane: transfer/accept mirror send/receive, split by direction
        // across two queues (like the data plane's send/emit and receive/read).
        // Parent→worker uses the inbound resource queue; worker→parent uses the
        // outbound resource queue, so a thread's own transfer is never re-read by
        // its own accept.
        //
        // transferResource: parent writes the inbound resource queue (mirrors send).
        "thread.transferResource" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        // emitResource: worker writes the outbound resource queue (mirrors emit).
        "thread.emitResource" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
            false,
            platform_imports,
            platform,
        ),
        // acceptResource: worker reads the inbound resource queue (mirrors receive,
        // and like accept allows an indefinite wait).
        "thread.acceptResource" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
            ThreadReadMode::WorkerSelf,
            platform_imports,
            platform,
        ),
        // readResource: parent reads the outbound resource queue (mirrors read, but
        // unlike read it permits an indefinite wait — see thread::accept docs).
        "thread.readResource" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
            ThreadReadMode::ParentWaitable,
            platform_imports,
            platform,
        ),
        "thread.isCancelled" => Ok(thread_is_cancelled_helper()),
        _ => Err(format!("native thread helper does not implement {call}")),
    }
}

fn lower_thread_start_helper(
    symbol: &str,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const ENTRY_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 16;
    const IN_LIMIT_OFFSET: usize = 24;
    const OUT_LIMIT_OFFSET: usize = 32;
    const CB_OFFSET: usize = 40;
    const QUEUE_OFFSET: usize = 48;

    let invalid_limit = format!("{symbol}_invalid_limit");
    let alloc_block_ok = format!("{symbol}_alloc_block_ok");
    let alloc_worker_arena_ok = format!("{symbol}_alloc_worker_arena_ok");
    let spawn_error = format!("{symbol}_spawn_error");
    let parent_done = format!("{symbol}_parent_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();

    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x0", abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x2", abi::stack_pointer(), IN_LIMIT_OFFSET),
        abi::store_u64("x3", abi::stack_pointer(), OUT_LIMIT_OFFSET),
        abi::compare_immediate("x2", "1"),
        abi::branch_lt(&invalid_limit),
        abi::compare_immediate("x3", "1"),
        abi::branch_lt(&invalid_limit),
        abi::move_immediate("x0", "Integer", &THREAD_BLOCK_SIZE.to_string()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_block_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&alloc_block_ok),
        abi::store_u64("x1", abi::stack_pointer(), CB_OFFSET),
        abi::move_register("x9", "x1"),
        abi::store_u64("x31", "x9", THREAD_OFFSET_STATE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_CANCELLED),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_TAG),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_VALUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_ERROR),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_SOURCE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_INBOUND_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESOURCE_INBOUND_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_OS_HANDLE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_PARENT_ARENA_STATE),
        abi::load_u64("x10", abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64("x10", "x9", THREAD_OFFSET_ENTRY),
        abi::load_u64("x10", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x10", "x9", THREAD_OFFSET_DATA),
        abi::store_u64(ARENA_STATE_REGISTER, "x9", THREAD_OFFSET_PARENT_ARENA_STATE),
        abi::move_immediate("x0", "Integer", &ARENA_STATE_SIZE.to_string()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_worker_arena_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&alloc_worker_arena_ok),
        abi::store_u64("x31", "x1", 0),
        abi::store_u64("x31", "x1", 8),
        abi::store_u64("x31", "x1", 16),
        abi::store_u64("x31", "x1", 24),
        abi::store_u64("x31", "x1", 32),
        abi::store_u64("x31", "x1", 40),
        abi::store_u64("x31", "x1", 48),
        abi::store_u64("x31", "x1", 56),
        abi::store_u64("x31", "x1", ARENA_CLEANUP_FAILURE_COUNT_OFFSET),
        abi::store_u64("x31", "x1", ARENA_CLEANUP_FAILURE_CODE_OFFSET),
        abi::store_u64("x31", "x1", ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
        abi::store_u64("x1", "x9", THREAD_OFFSET_ARENA_STATE),
    ]);

    if uses_rng {
        // Give the new thread its own PCG64 stream by drawing a 64-bit seed from
        // the spawning thread's generator (runs in the parent, so `x19` is the
        // parent arena and the draw is race-free). Reload the child arena from
        // the control block afterwards because the draw clobbers x0-x18.
        instructions.push(abi::branch_link(RNG_NEXT_SYMBOL));
        relocations.push(internal_branch(symbol, RNG_NEXT_SYMBOL));
        instructions.extend([
            abi::move_register("x1", abi::return_register()),
            abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
            abi::load_u64(abi::return_register(), "x9", THREAD_OFFSET_ARENA_STATE),
        ]);
        instructions.push(abi::branch_link(RNG_SEED_SYMBOL));
        relocations.push(internal_branch(symbol, RNG_SEED_SYMBOL));
    }

    // Seed the worker's dedicated memory-fill RNG (always on, plan-01 §6). Draw an
    // entropy-derived 64-bit value from the parent's fill stream — race-free since
    // this runs in the parent (`x19` is the parent arena) — and XOR the worker
    // arena address so each worker poisons with a distinct stream. The worker's
    // fill RNG (offsets 16/24) is separate from its `math::rand` stream, exactly
    // as on the main thread. (`arenaStartTime` at offset 40 stays 0 for workers;
    // it is a main-thread diagnostic and not needed for the seed's distinctness.)
    instructions.push(abi::branch_link(ARENA_FILL_NEXT_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FILL_NEXT_SYMBOL));
    instructions.extend([
        abi::move_register("x1", abi::return_register()),
        abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(abi::return_register(), "x9", THREAD_OFFSET_ARENA_STATE),
        abi::exclusive_or_registers("x1", "x1", abi::return_register()),
    ]);
    instructions.push(abi::branch_link(ARENA_FILL_SEED_SYMBOL));
    relocations.push(internal_branch(symbol, ARENA_FILL_SEED_SYMBOL));

    emit_thread_queue_alloc(
        symbol,
        platform_imports,
        platform,
        IN_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_INBOUND_QUEUE,
        &parent_done,
        &mut instructions,
        &mut relocations,
    )?;
    emit_thread_queue_alloc(
        symbol,
        platform_imports,
        platform,
        OUT_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_OUTBOUND_QUEUE,
        &parent_done,
        &mut instructions,
        &mut relocations,
    )?;
    // Resource plane queues (§7): inbound (parent→worker) bounded like the
    // inbound data queue, outbound (worker→parent) bounded like the outbound
    // data queue.
    emit_thread_queue_alloc(
        symbol,
        platform_imports,
        platform,
        IN_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
        &parent_done,
        &mut instructions,
        &mut relocations,
    )?;
    emit_thread_queue_alloc(
        symbol,
        platform_imports,
        platform,
        OUT_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
        &parent_done,
        &mut instructions,
        &mut relocations,
    )?;

    let pthread_create_symbol = if platform.target() == "macos-aarch64" {
        "_pthread_create"
    } else {
        "pthread_create"
    };
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
        abi::add_immediate("x0", "x9", THREAD_OFFSET_OS_HANDLE),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    instructions.push(abi::load_page_address("x2", THREAD_TRAMPOLINE_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        kind: RelocIntent::DataAddrHi,
        binding: "data".to_string(),
        library: None,
    });
    instructions.push(abi::add_page_offset("x2", "x2", THREAD_TRAMPOLINE_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        kind: RelocIntent::DataAddrLo,
        binding: "data".to_string(),
        library: None,
    });
    instructions.extend([
        abi::move_register("x3", "x9"),
        abi::branch_link(pthread_create_symbol),
    ]);
    relocations.push(external_branch(
        symbol,
        pthread_create_symbol,
        platform_imports,
    )?);
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&spawn_error),
        abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
        abi::move_register(RESULT_VALUE_REGISTER, "x9"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&parent_done),
    ]);

    instructions.extend([
        abi::label(&invalid_limit),
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
        abi::branch(&parent_done),
        abi::label(&spawn_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&parent_done));
    instructions.extend([
        abi::label(&parent_done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

pub(super) fn lower_thread_trampoline(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const ARENA_OFFSET: usize = 8;
    const X20_OFFSET: usize = 16;
    const CLOSURE_OFFSET: usize = 24;
    const CB_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const VALUE_OFFSET: usize = 48;
    const ERROR_OFFSET: usize = 56;
    const SOURCE_OFFSET: usize = 64;
    let result_closed = format!("{THREAD_TRAMPOLINE_SYMBOL}_result_closed");

    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::store_u64("x20", abi::stack_pointer(), X20_OFFSET),
        abi::store_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
        abi::move_register("x20", "x0"),
        abi::store_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(ARENA_STATE_REGISTER, "x20", THREAD_OFFSET_ARENA_STATE),
        abi::load_u64("x9", "x20", THREAD_OFFSET_ENTRY),
        abi::load_u64(CLOSURE_ENV_REGISTER, "x9", CLOSURE_OFFSET_ENV),
        abi::load_u64("x9", "x9", CLOSURE_OFFSET_CODE),
        abi::load_u64("x1", "x20", THREAD_OFFSET_DATA),
        abi::move_register("x0", "x20"),
        abi::branch_link_register("x9"),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            SOURCE_OFFSET,
        ),
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_INBOUND_QUEUE),
        abi::move_register("x0", "x9"),
    ];
    let mut relocations = Vec::new();
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_lock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_INBOUND_QUEUE),
        abi::move_immediate("x10", "Integer", "1"),
        abi::store_u64("x10", "x9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_INBOUND_QUEUE),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x0", "x20", THREAD_OFFSET_INBOUND_QUEUE),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_unlock",
        &mut instructions,
        &mut relocations,
    )?;
    // Close both resource-plane queues on worker exit, mirroring the data
    // queues: wake any parent blocked in `thread::transfer` (writing the inbound
    // resource queue) or `thread::accept` (reading the outbound resource queue).
    for resource_queue_offset in [
        THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
        THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
    ] {
        instructions.extend([
            abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
            abi::load_u64("x9", "x20", resource_queue_offset),
            abi::move_register("x0", "x9"),
        ]);
        emit_thread_external_call(
            THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            "pthread_mutex_lock",
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
            abi::load_u64("x9", "x20", resource_queue_offset),
            abi::move_immediate("x10", "Integer", "1"),
            abi::store_u64("x10", "x9", THREAD_QUEUE_CLOSED_OFFSET),
            abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        ]);
        emit_thread_external_call(
            THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            "pthread_cond_broadcast",
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
            abi::load_u64("x9", "x20", resource_queue_offset),
            abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
        ]);
        emit_thread_external_call(
            THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            "pthread_cond_broadcast",
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
            abi::load_u64("x0", "x20", resource_queue_offset),
        ]);
        emit_thread_external_call(
            THREAD_TRAMPOLINE_SYMBOL,
            platform_imports,
            platform,
            "pthread_mutex_unlock",
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::move_register("x0", "x9"),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_lock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_STATE),
        abi::compare_immediate("x9", THREAD_STATE_CLOSED),
        abi::branch_eq(&result_closed),
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_TAG),
        abi::load_u64("x9", abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_VALUE),
        abi::load_u64("x9", abi::stack_pointer(), ERROR_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_ERROR),
        abi::load_u64("x9", abi::stack_pointer(), SOURCE_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_SOURCE),
        abi::move_immediate("x10", "Integer", THREAD_STATE_COMPLETED),
        abi::store_u64("x10", "x20", THREAD_OFFSET_STATE),
        abi::load_u64("x9", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&result_closed),
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x0", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_unlock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate("x0", "Integer", "0"),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::load_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
        abi::load_u64("x20", abi::stack_pointer(), X20_OFFSET),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.thread.trampoline".to_string(),
        symbol: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        params: vec![CodeParam {
            name: "controlBlock".to_string(),
            type_: "ThreadControlBlock".to_string(),
            location: "x0".to_string(),
        }],
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string(), "x20".to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

