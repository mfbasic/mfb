//! Per-member `Body::abi_function` lowerings for the `thread` package.
//!
//! Each member owns a thin body here that calls its shared runtime-thread emitter
//! (in `codegen::runtime::thread`, which emits its own fallible ABI — the result
//! value in `RESULT_VALUE_REGISTER` + the tag, each error path returning), branching
//! the worker/parent + resource-plane split off [`AbiCtx::call`]. The body returns a
//! `void`-location result, so the `abi_function` wrapper (`lower_abi_function_helper`)
//! seeds the `entry` label and finalizes without adding an epilogue. The internal
//! `emit`/`read`/`sleepWorker`/`*Resource`/`drop` runtime-call names are the members'
//! `os_aliases` (see `mod.rs`), routed to the owning body by `abi_function_lower`.

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::operand::Operand;
use crate::codegen::registry::AbiCtx;
use crate::codegen::runtime::thread::{
    lower_thread_sleep_helper, lower_thread_sleep_worker_helper, lower_thread_start_helper,
    lower_thread_stdin_subscription_helper, simple_thread_handle_helper,
    thread_is_cancelled_helper, thread_queue_read_helper, thread_queue_write_helper,
    ThreadBodyParts, ThreadReadMode, ThreadSimpleOp, THREAD_OFFSET_INBOUND_QUEUE,
    THREAD_OFFSET_OUTBOUND_QUEUE, THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
    THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
};
use crate::types::ParameterType;

/// Extend `builder` with an emitter's un-finalized parts + record the frame size, and
/// return the `void` result the `abi_function` wrapper recognizes (no epilogue). `text`
/// carries the runtime-call name; `location` is `void`.
fn finish(builder: &mut CodeBuilder, parts: ThreadBodyParts, call: &str) -> ValueResult {
    let (instructions, relocations, stack_size) = parts;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: call.to_string(),
    }
}

/// `thread::start` — spawn a worker. Reads `AbiCtx::arena_global_slots`/`uses_rng` to
/// size and seed the worker's arena block (bug-369, plan-01 §6).
pub(crate) fn lower_start(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let parts = lower_thread_start_helper(
        &symbol,
        ctx.uses_rng,
        ctx.arena_global_slots,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::isRunning`.
pub(crate) fn lower_is_running(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let parts = simple_thread_handle_helper(
        &symbol,
        ThreadSimpleOp::IsRunning,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::waitFor`.
pub(crate) fn lower_wait_for(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let parts = simple_thread_handle_helper(
        &symbol,
        ThreadSimpleOp::WaitFor,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::cancel` — and its internal `thread.drop` scope-cleanup code form (both
/// ride `simple_thread_handle_helper`, selected off `AbiCtx::call`).
pub(crate) fn lower_cancel(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let op = if ctx.call == "thread.drop" {
        ThreadSimpleOp::Drop
    } else {
        ThreadSimpleOp::Cancel
    };
    let parts = simple_thread_handle_helper(&symbol, op, ctx.platform_imports, ctx.platform)?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::poll`.
pub(crate) fn lower_poll(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let parts = simple_thread_handle_helper(
        &symbol,
        ThreadSimpleOp::Poll,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::isCancelled` (worker-side; infallible emitter).
pub(crate) fn lower_is_cancelled(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let parts = thread_is_cancelled_helper();
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::send` — parent→worker inbound-queue write — and its worker-side `thread.emit`
/// code form (worker→parent outbound-queue write), selected off `AbiCtx::call`.
pub(crate) fn lower_send(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (queue_offset, parent_send) = if ctx.call == "thread.emit" {
        (THREAD_OFFSET_OUTBOUND_QUEUE, false)
    } else {
        (THREAD_OFFSET_INBOUND_QUEUE, true)
    };
    let parts = thread_queue_write_helper(
        &symbol,
        queue_offset,
        parent_send,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::receive` — worker-side inbound-queue read — and its parent-side `thread.read`
/// code form (outbound-queue read), selected off `AbiCtx::call`.
pub(crate) fn lower_receive(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (queue_offset, mode) = if ctx.call == "thread.read" {
        (THREAD_OFFSET_OUTBOUND_QUEUE, ThreadReadMode::Parent)
    } else {
        (THREAD_OFFSET_INBOUND_QUEUE, ThreadReadMode::WorkerSelf)
    };
    let parts = thread_queue_read_helper(
        &symbol,
        queue_offset,
        mode,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::sleep` — parent-side plain `nanosleep` — and its worker-side
/// cancellation-aware `thread.sleepWorker` code form, selected off `AbiCtx::call`.
pub(crate) fn lower_sleep(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let parts = if ctx.call == "thread.sleepWorker" {
        lower_thread_sleep_worker_helper(&symbol, ctx.platform_imports, ctx.platform)?
    } else {
        lower_thread_sleep_helper(&symbol, ctx.platform_imports, ctx.platform)?
    };
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::transfer` (resource plane) — its `thread.transferResource` (parent→worker,
/// resource inbound-queue write) and `thread.emitResource` (worker→parent, resource
/// outbound-queue write) code forms, selected off `AbiCtx::call`.
pub(crate) fn lower_transfer(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (queue_offset, parent_send) = if ctx.call == "thread.emitResource" {
        (THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE, false)
    } else {
        (THREAD_OFFSET_RESOURCE_INBOUND_QUEUE, true)
    };
    let parts = thread_queue_write_helper(
        &symbol,
        queue_offset,
        parent_send,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::accept` (resource plane) — its `thread.acceptResource` (worker-side,
/// resource inbound-queue read) and `thread.readResource` (parent-side, resource
/// outbound-queue read) code forms, selected off `AbiCtx::call`.
pub(crate) fn lower_accept(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (queue_offset, mode) = if ctx.call == "thread.readResource" {
        (
            THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE,
            ThreadReadMode::Parent,
        )
    } else {
        (
            THREAD_OFFSET_RESOURCE_INBOUND_QUEUE,
            ThreadReadMode::WorkerSelf,
        )
    };
    let parts = thread_queue_read_helper(
        &symbol,
        queue_offset,
        mode,
        ctx.platform_imports,
        ctx.platform,
    )?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::openStdIn` — subscribe to the stdin broadcast.
pub(crate) fn lower_open_std_in(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let parts = lower_thread_stdin_subscription_helper(&symbol, true)?;
    Ok(finish(builder, parts, ctx.call))
}

/// `thread::closeStdIn` — unsubscribe from the stdin broadcast.
pub(crate) fn lower_close_std_in(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let parts = lower_thread_stdin_subscription_helper(&symbol, false)?;
    Ok(finish(builder, parts, ctx.call))
}
