// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::builder_arena_transfer::RawSuccessBlock;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::target::shared::runtime;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Whether a type is a PARENT-side thread handle (`Thread OF …`).
    ///
    /// plan-106-E: the handle and its side are a variant and its `worker` flag,
    /// not a prefix on a spelling.
    pub(crate) fn is_thread_type(type_: &ParameterType) -> bool {
        matches!(type_, ParameterType::ThreadHandle { worker: false, .. })
    }

    pub(crate) fn thread_drop_symbol() -> String {
        runtime::symbol_for_call(runtime::RuntimeHelper::Thread, "thread.drop")
    }

    pub(crate) fn deactivate_thread_cleanup(&mut self, name: &str) {
        if let Some(index) = self.active_cleanups.iter().rposition(
            |cleanup| matches!(cleanup, ActiveCleanup::Thread(thread) if thread.name == name),
        ) {
            self.active_cleanups.remove(index);
        }
    }

    /// A thread handle passed to a callee: the callee's parameter now closes it, so this
    /// binding no longer closes it at scope exit.
    ///
    /// bug-622: it still RELEASES it. The language lets the caller read the handle after
    /// the call (the op answers `ErrResourceClosed` once the callee closed it), so this
    /// binding keeps its owner count and the control block outlives it too; the callee's
    /// parameter took an owner count of its own on entry. (A `RETURN` is different — the
    /// callee frame is gone, so `deactivate_thread_cleanup` hands its count to the caller.)
    pub(crate) fn maybe_deactivate_moved_thread_local(&mut self, value: &NirValue) {
        let NirValue::Local(name) = value else {
            return;
        };
        if !self
            .locals
            .get(name)
            .is_some_and(|local| Self::is_thread_type(&local.type_))
        {
            return;
        }
        if let Some(ActiveCleanup::Thread(thread)) = self.active_cleanups.iter_mut().rev().find(
            |cleanup| matches!(cleanup, ActiveCleanup::Thread(thread) if thread.name == *name),
        ) {
            thread.close = false;
        }
    }

    /// bug-622: whether a thread-typed store takes a handle no other binding holds — a
    /// `thread::start` (or any call) result, a trapped `Result`'s payload, or the
    /// not-yet-assigned null of an inline `TRAP`'s value binding. Such a store inherits
    /// the handle's existing owner count. Anything else reads a handle another binding
    /// still holds and must add an owner; an unrecognized shape is treated that way, so a
    /// misclassification leaks a control block instead of freeing one that is still read.
    pub(crate) fn thread_value_is_fresh_handle(value: Option<&NirValue>) -> bool {
        matches!(
            value,
            None | Some(
                NirValue::Call { .. }
                    | NirValue::CallResult { .. }
                    | NirValue::RuntimeCall { .. }
                    | NirValue::ResultValue { .. }
            )
        )
    }

    /// bug-622: add one owner to the thread handle stored in the stack slot at `slot`
    /// (`CONTROL_OFFSET_OWNERS`). A null slot is left alone. Every handle is counted —
    /// including the zeroed CLOSED handle an inline `TRAP` on `thread::start` binds,
    /// whose last release frees its block — so an alias of it cannot free it twice.
    /// Call-free.
    pub(crate) fn emit_thread_owner_increment(&mut self, slot: usize) {
        use crate::codegen::runtime::thread::CONTROL_OFFSET_OWNERS;
        let skip = self.label("thread_owner_increment_skip");
        let handle = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&handle, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&handle, "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit(abi::load_u64(&count, &handle, CONTROL_OFFSET_OWNERS));
        self.emit(abi::add_immediate(&count, &count, 1));
        self.emit(abi::store_u64(&count, &handle, CONTROL_OFFSET_OWNERS));
        self.emit(abi::label(&skip));
    }

    /// A thread `transferResource`/`emitResource` moves its resource argument
    /// (`args[1]`) across the thread boundary. If that argument was a fresh heap
    /// temporary, claim it so the statement-scope free never reclaims a record whose
    /// handle has moved. A `Local` argument is an aliasing source that was never
    /// registered, so this is a no-op for it (plan-25).
    ///
    /// `thread.start`, `thread.send` and `thread.emit` are NOT in the list: their
    /// helpers deep-copy the value into the thread's block arena (plan-156-A) and
    /// hand the COPY across, so the original never crosses and its owner frees it as
    /// usual (bug-629 Part A / bug-649 for the sends; before plan-156-A the seed was
    /// handed over uncopied and had to be claimed).
    pub(crate) fn claim_moved_thread_arg_temp(&mut self, target: &str, arg_values: &[ValueResult]) {
        if matches!(target, "thread.transferResource" | "thread.emitResource") {
            if let Some(arg) = arg_values.get(1) {
                self.claim_pending_temp(arg);
            }
        }
    }

    pub(crate) fn deactivate_moved_thread_arguments(&mut self, target: &str, args: &[NirValue]) {
        match target {
            "thread.start"
            | "thread.send"
            | "thread.emit"
            | "thread.transferResource"
            | "thread.emitResource" => {
                if let Some(arg) = args.get(1) {
                    self.maybe_deactivate_moved_thread_local(arg);
                }
            }
            target if !target.starts_with("thread.") => {
                for arg in args {
                    self.maybe_deactivate_moved_thread_local(arg);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn emit_thread_send_runtime_helper_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        result_type: &ParameterType,
        raw: bool,
    ) -> Result<ValueResult, String> {
        if args.len() < 2 {
            return Err(format!(
                "native runtime call '{target}' expects a handle and message"
            ));
        }
        let scratch9 = self.temporary_vreg();
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        self.reset_temporary_registers();
        for arg in args {
            let value = self.lower_value(arg)?;
            // Observation boundary: a `Float` sent across a thread boundary is
            // observable on the other side and must be finite (plan-17).
            self.observe_float(arg, &value)?;
            // Materialize a `d`-native float before marshalling it across the
            // thread boundary (plan-01 float-dnative).
            let value = self.materialize_float(value)?;
            let slot = self.allocate_stack_object("runtime_thread_send_arg", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
            self.reset_temporary_registers();
        }
        // The resource targets keep the claim — see `claim_moved_thread_arg_temp`.
        // A data message is copied by the helper, so its original stays owned here and
        // the statement-scope temp cleanup frees it (bug-629 Part A / bug-649).
        self.claim_moved_thread_arg_temp(target, &arg_values);

        // plan-156-A: the helper deep-copies the message into the thread's block arena
        // under the block lock, and only once the enqueue is certain (the queue has
        // room and is open), so a failed send leaves nothing behind and the sender's
        // value is untouched. Arg 3 is the message type's copy function (0 for a value
        // that crosses as its word). A thread-sendable resource's copy flags its source
        // `moved|closed` as it copies (`copy_resource_to_current_arena`) — which now
        // happens exactly when the transfer succeeds, the property bug-425 had to defer
        // the flag to the enqueue-success branch to get.
        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), *slot));
            self.emit(abi::move_register(
                &abi::argument_register(index)?,
                &scratch9,
            ));
        }
        let msg_type = arg_values[1].type_.clone();
        self.emit_thread_value_fn_address(
            crate::codegen::runtime::thread::ThreadValueFn::Copy,
            &msg_type,
            abi::argument_register(3)?,
        );
        // Arg 4: the message type's drop function, which the helper records on the
        // queue so the thread's final release drops (and, for a resource, closes) a
        // message that is never received.
        self.emit_thread_value_fn_address(
            crate::codegen::runtime::thread::ThreadValueFn::Drop,
            &msg_type,
            abi::argument_register(4)?,
        );
        self.emit_symbol_call(symbol);

        // An inline `TRAP` traps the raw send `Result`. On failure the sent value
        // remains owned by the caller (the former source checker restores the binding
        // into the handler scope); the success continuation treats it as moved.
        if raw {
            self.deactivate_moved_thread_arguments(target, args);
            self.deactivate_moved_resource_arguments(target, args);
            // thread.send/emit errors originate at this call site, not a worker.
            // bug-566: `thread.send`/`emit`/`transferResource`/`emitResource` all
            // return `Nothing` — there is no success block at all — and every
            // `thread.*` target is foreign-arena anyway.
            return self.materialize_current_result(
                result_type,
                format!("callResult {target}"),
                false,
                RawSuccessBlock::OwnedElsewhere,
            );
        }

        let ok_label = self.label("runtime_thread_send_ok");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit_stamp_current_error_source()?;
        self.emit_call_error_exit()?;
        self.emit(abi::label(&ok_label));
        self.deactivate_moved_thread_arguments(target, args);
        self.deactivate_moved_resource_arguments(target, args);

        if *result_type != ParameterType::Nothing {
            return Err(format!(
                "native runtime call '{target}' expected Nothing result, got '{result_type}'"
            ));
        }
        Ok(ValueResult {
            origin: None,
            type_: result_type.clone(),
            location: Operand::from("void"),
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }
}
