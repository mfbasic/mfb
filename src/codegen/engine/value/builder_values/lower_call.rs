//! plan-176-U: `lower_call_value` and `lower_call_result_value`, the `Call` and `CallResult` arms
//! of the value lowering. Called from `lower_value_inner` in
//! `engine/value/builder_values/lower_inner.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Lower a direct `NirValue::Call`: inline vector ops, ABI-inline and fast-path calls, then the
    /// ordinary call.
    pub(super) fn lower_call_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let NirValue::Call { target, args, loc } = value else {
            unreachable!("lower_call_value is called from the NirValue::Call arm only")
        };
        // plan-01-vector: inline the pure-arithmetic `vector::` ops
        // (`scale`, `dot`) over Float2/3/4 as their equivalent constructor
        // / sum expression, eliminating the out-of-line FUNC call. The
        // rewrite reproduces the `vector_package.mfb` body's exact
        // expression tree, so the result and its finiteness observation are
        // bit-identical; a non-simple (re-evaluation-unsafe) operand or any
        // un-inlined op falls back to the package FUNC call below.
        if let Some(result) = self.try_inline_vector_op(target, args, *loc)? {
            return Ok(result);
        }
        // plan-39 A4: lower the internal `#collections_slice$T` helper
        // (window/chunks) as a native contiguous-range copy instead of the
        // element-by-element FUNC. Non-list / unsupported types fall back.
        if let Some(result) = self.try_inline_slice_op(target, args)? {
            return Ok(result);
        }
        // zip migrated to Implementation::Mfb.fast_path (func_zip.rs),
        // consulted by try_mfb_fast_path below.
        if let Some(local) = self.locals.get(target).cloned() {
            // Any callable, `ISOLATED` included: an isolated function value
            // is the same capture-free code pointer, and it is called the
            // same way. Matching only the plain form sent a call through an
            // `ISOLATED FUNC` local down the direct-call path, which emitted
            // a relocation to a function named like the local (found with
            // bug-822).
            if matches!(local.type_, ParameterType::Func(..)) {
                let return_type_typed = typed_callable_return_type(&local.type_)
                    .expect("guarded by the Func match above")
                    .clone();
                let return_type = return_type_typed.clone();
                let callable = ValueResult {
                    origin: None,
                    type_: local.type_.clone(),
                    location: {
                        let register = self.allocate_register();
                        self.emit(abi::load_u64(
                            &register,
                            abi::stack_pointer(),
                            local.stack_offset,
                        ));
                        Operand::from(register)
                    },
                    text: target.clone(),
                };
                return self.emit_function_value_call(
                    target,
                    &callable,
                    args,
                    Some(&return_type.name()),
                );
            }
        }
        // A top-level (global) binding holding a function value is called
        // indirectly too (bug-198): load the function pointer from the
        // global's arena slot, mirroring the local FUNC-value path above.
        if let Some(global) = self.globals.get(target).cloned() {
            if matches!(global.type_, ParameterType::Func(..)) {
                let return_type = typed_callable_return_type(&global.type_)
                    .expect("guarded by the Func match above")
                    .name()
                    .into_owned();
                let address = self.load_global_address(target)?;
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, &address, 0));
                let callable = ValueResult {
                    origin: None,
                    type_: global.type_.clone(),
                    location: Operand::from(register),
                    text: target.clone(),
                };
                return self.emit_function_value_call(target, &callable, args, Some(&return_type));
            }
        }
        // `strings::`/`astrings::` native members migrated to the clean-room
        // registry, reached below through the inline dual-path (plan-99 PART
        // B/C). `strings::` lowers per member in its own `func_*.rs` (the
        // `builder_strings_*` carrier + `lower_strings_package_call` dispatcher
        // were collapsed to the func_/gen_ shape); `astrings::` members are
        // `Body::abi_inline`, reached through `try_abi_inline_lower`,
        // still delegating to the shared `lower_astrings_package_call` carrier
        // in `gen_astrings.rs`.
        // Migrated `collections::`/`strings::` members arrive with their
        // qualified, dot-containing target (`collections.get`,
        // `strings.find`, ...). `native_builtin_target` maps these to the
        // shared bare lowering name and returns `None` for bare names, so a
        // user `FUNC get` is never hijacked by the native lowering
        // (plan-01-functions.md §5).
        // plan-95: prefer a migrated function's own `abi_inline` lowering
        // over the legacy ladder below.
        if let Some(result) = self.try_abi_inline_lower(target, args) {
            return result;
        }
        let native = crate::codegen::builtins::native_builtin_target(target);
        if native == Some("find") && (args.len() == 2 || args.len() == 3) {
            return self.lower_find(args);
        }
        if target == "len" && args.len() == 1 {
            return self.lower_len(&args[0]);
        }
        if native == Some("mid") && args.len() == 3 {
            return self.lower_mid(args);
        }
        if native == Some("replace") && args.len() == 3 {
            return self.lower_replace(args);
        }
        // Mfb dual-path: a migrated source-generic member's native fast path
        // (`Implementation::Mfb.fast_path`, owned by its `func_*.rs`) is
        // consulted here. It self-gates on the monomorph instantiation and
        // either lowers (returns here) or declines, falling through to the
        // un-migrated per-member arms below and then the monomorphized `.mfb`
        // body. As each member migrates, its per-member arm below is deleted.
        if let Some(result) = self.try_mfb_fast_path(target, args) {
            return result;
        }
        // findLastIndex migrated to Implementation::Mfb.fast_path
        // (func_find_last_index.rs), consulted by try_mfb_fast_path above.
        if target == "toString" && (args.len() == 1 || args.len() == 2) {
            return self.lower_to_string(args);
        }
        if target == "typeName" && args.len() == 1 {
            let type_name = self
                .static_type_name_for_fold(&args[0])
                .ok_or_else(|| "native code cannot determine typeName argument type".to_string())?;
            let register = self.load_string_constant(&type_name.name())?;
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::String,
                location: Operand::from(register),
                text: format!("typeName({type_name})"),
            });
        }
        if target == "toInt" && (1..=2).contains(&args.len()) {
            return self.lower_to_int(args);
        }
        if target == "toFloat" && args.len() == 1 {
            return self.lower_to_float(&args[0]);
        }
        if target == "toFixed" && args.len() == 1 {
            return self.lower_to_fixed(&args[0]);
        }
        if target == "toByte" && args.len() == 1 {
            return self.lower_to_byte(&args[0]);
        }
        if target == "toMoney" && args.len() == 1 {
            return self.lower_to_money(&args[0]);
        }
        if target == "toScalar" && args.len() == 1 {
            return self.lower_to_scalar(&args[0]);
        }
        if target == "isNumeric" && args.len() == 1 {
            return self.lower_is_numeric(&args[0]);
        }
        // `math.*` migrated to the clean-room registry (`Body::abi_inline`),
        // reached above through `try_abi_inline_lower` — no `math.` name
        // predicate here. The shared `lower_math_call` carrier stays; each
        // member's `func_*.rs` shim calls it, and `builder_vector_inline` reaches
        // the scalar `math.sqrt`/`math.clamp` it emits as `NirValue::Call` through
        // the same self-lowering dual-path.
        // `money.*` migrated to the clean-room registry (`Body::abi_inline`),
        // reached on the `Call` path through `try_abi_inline_lower`; the
        // explicit intrinsics below cover this `RuntimeCall` arm.
        if target == "isEven" && args.len() == 1 {
            return self.lower_integer_parity_predicate("isEven", &args[0], false);
        }
        if target == "isOdd" && args.len() == 1 {
            return self.lower_integer_parity_predicate("isOdd", &args[0], true);
        }
        if matches!(target.as_str(), "isPositive" | "isNegative" | "isZero") && args.len() == 1 {
            return self.lower_numeric_filter_predicate(target, &args[0]);
        }
        if matches!(target.as_str(), "isEmpty" | "isNotEmpty") && args.len() == 1 {
            return self.lower_empty_filter_predicate(target, &args[0]);
        }
        let symbol = self
            .function_symbols
            .get(target)
            .cloned()
            .unwrap_or_else(|| target.to_string());
        // plan-147-D: name the call node so `emit_call` can ask `handover`
        // whether this SITE hands anything over. Restored afterwards so a
        // nested call cannot inherit an outer site's answer.
        let previous_call = self
            .current_call_key
            .replace(crate::codegen::engine::analysis::handover::call_key(value));
        let result = self.emit_call(target, &symbol, args, None);
        self.current_call_key = previous_call;
        result
    }

    /// Lower a `NirValue::CallResult` (a call whose `Result` is kept): local and global `FUNC`
    /// values, inline conversions and builtins, then the trapped call. `scratch9` is
    /// `lower_value_inner`'s first scratch vreg.
    pub(super) fn lower_call_result_value(
        &mut self,
        value: &NirValue,
        scratch9: &VirtualRegister,
    ) -> Result<ValueResult, String> {
        let NirValue::CallResult { target, args, .. } = value else {
            unreachable!("lower_call_result_value is called from the NirValue::CallResult arm only")
        };
        if let Some(local) = self.locals.get(target).cloned() {
            if matches!(local.type_, ParameterType::Func(..)) {
                let return_type_typed = typed_callable_return_type(&local.type_)
                    .expect("guarded by the Func match above")
                    .clone();
                let return_type = return_type_typed.clone();
                let callable = ValueResult {
                    origin: None,
                    type_: local.type_.clone(),
                    location: {
                        let register = self.allocate_register();
                        self.emit(abi::load_u64(
                            &register,
                            abi::stack_pointer(),
                            local.stack_offset,
                        ));
                        Operand::from(register)
                    },
                    text: target.clone(),
                };
                // bug-448: the inline-TRAP machinery consumes a boxed
                // `Result` (tag/value/error), so materialize one from the
                // call's standard result registers exactly as the direct
                // user-function raw path below does — the previous code
                // returned the raw success *value*, which the machinery
                // then dereferenced as a `Result`-object pointer and
                // segfaulted.
                let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
                let value_slot = self.allocate_stack_object("raw_result_value", 8);
                let message_slot = self.allocate_stack_object("raw_result_message", 8);
                let source_slot = self.allocate_stack_object("raw_result_source", 8);
                let unboxed = self.unboxed_trap_active.take();
                self.emit_function_value_call_raw(&callable, args)?;
                self.emit(abi::store_u64(
                    RESULT_TAG_REGISTER,
                    abi::stack_pointer(),
                    tag_slot,
                ));
                self.emit(abi::store_u64(
                    RESULT_VALUE_REGISTER,
                    abi::stack_pointer(),
                    value_slot,
                ));
                self.emit(abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    message_slot,
                ));
                self.emit(abi::store_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    source_slot,
                ));
                // bug-841: `EXIT PROGRAM` is not the handler's to catch.
                self.emit_inline_trap_program_exit_passthrough(tag_slot, value_slot)?;
                // bug-778: an unboxed temporary keeps the callee's block —
                // an owning `Bind` of the untrapped call owns it too (a
                // `FUNC` value is never a param borrow, bug-569).
                if let Some(unboxed) = unboxed {
                    return self.finish_unboxed_trap_result(
                        unboxed,
                        tag_slot,
                        value_slot,
                        message_slot,
                        source_slot,
                        UnboxedPayloadSource::Owned,
                        TrappedErrorSource::CalleeRegister,
                        format!("callResult {target}"),
                    );
                }
                let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
                let wrap_error_label = self.label("result_wrap_error");
                let have_payload_label = self.label("result_have_payload");
                let result_slot = self.allocate_stack_object("raw_result", 8);
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), tag_slot));
                self.emit(abi::compare_immediate(scratch9, RESULT_OK_TAG));
                self.emit(abi::branch_ne(&wrap_error_label));
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), value_slot));
                self.emit(abi::store_u64(scratch9, abi::stack_pointer(), payload_slot));
                let ok_result =
                    self.emit_build_result_inline(tag_slot, &return_type, payload_slot)?;
                self.emit(abi::store_u64(
                    &ok_result,
                    abi::stack_pointer(),
                    result_slot,
                ));
                self.emit(abi::branch(&have_payload_label));
                self.emit(abi::label(&wrap_error_label));
                // bug-565: adopt a parked `ERR_BLOCK` instead of orphaning
                // it, and free the payload the `Result` copied. Shared with
                // the direct-callee path below and with
                // `materialize_current_result`.
                self.emit_trapped_error_result(
                    scratch9,
                    tag_slot,
                    value_slot,
                    message_slot,
                    source_slot,
                    payload_slot,
                    result_slot,
                    TrappedErrorSource::CalleeRegister,
                )?;
                self.emit(abi::label(&have_payload_label));
                return Ok(self.fresh_trapped_result_value(
                    result_slot,
                    return_type_typed.clone(),
                    format!("callResult {target}"),
                ));
            }
        }
        // An inline `TRAP` on an inline-lowered conversion built-in
        // (`toInt`, `toFloat`, `toFixed`, `toByte`) traps the raw
        // `Result`: lower the conversion inline but capture its error
        // instead of auto-propagating, then materialize the `Result`.
        if matches!(
            target.as_str(),
            "toInt" | "toFloat" | "toFixed" | "toByte" | "toMoney" | "toScalar"
        ) && (args.len() == 1 || (target == "toInt" && args.len() == 2))
        {
            return self.lower_inline_conversion_raw(target, args);
        }
        // bug-486: the fallibility census is name-keyed except for the
        // overloads whose argument type decides it — today only
        // `toString(<List OF Byte>)`, whose UTF-8 decode raises
        // `ErrEncoding`. `static_type_name_for_fold` is the widest arg-type
        // oracle the builder has (it falls back to the registry's typed
        // return-type resolver for a nested call); this is a read-only
        // question with none of the fast-path coupling that keeps
        // `static_type_name` itself narrow. An argument it cannot type
        // answers `Unknown`, which lands on the name-keyed verdict.
        let inline_arg_types: Vec<ParameterType> =
            if crate::codegen::builtins::inline_builtin_fallibility_depends_on_args(target) {
                args.iter()
                    .map(|arg| {
                        self.static_type_name_for_fold(arg)
                            .unwrap_or(ParameterType::Unknown)
                    })
                    .collect()
            } else {
                Vec::new()
            };
        // An inline `TRAP` on a fallible inline member (`collections::get`,
        // `strings::mid`, …) traps the raw `Result` the same way (plan-21-B):
        // run the member's normal inline lowering under a raw capture so its
        // domain error redirects to the capture point instead of propagating,
        // then materialize the `Result OF <success>`.
        if crate::codegen::builtins::inline_builtin_raw_supported(target, &inline_arg_types) {
            return self.lower_inline_builtin_raw(target, args);
        }
        // An inline `TRAP` on a provably-infallible inline built-in
        // (`len`, `toString`, `typeName`, `bits::*`, the pure-query/growth
        // collection members) is uniform surface (plan-26-A): the member
        // emits no error exit, so lower it normally and wrap the success as
        // an always-`Ok` `Result` for the inline-TRAP machinery. The handler
        // is dead code (front-end warns `TYPE_INLINE_TRAP_DEAD_HANDLER`).
        // bug-679: `NoTypeKinds` deliberately, and unreachable for the one
        // name that would care. Every `toInt` form returned above through
        // `lower_inline_conversion_raw`, so the enum overload never
        // arrives here; and codegen holds no declaration table anyway, so
        // the oracle's `false` is both correct and the over-approximating
        // side for anything that did.
        if crate::codegen::builtins::inline_builtin_is_infallible(
            target,
            &inline_arg_types,
            &crate::codegen::builtins::NoTypeKinds,
        ) {
            return self.lower_inline_infallible_raw(target, args);
        }
        // An inline `TRAP` on a helper-backed built-in (`thread::waitFor`,
        // `fs::*`, …) traps the raw `Result`. The runtime helper leaves
        // that `Result` in the standard tag/value/error registers just
        // like a user-function call, so we invoke the helper without the
        // auto-propagate branch and materialize the raw `Result`.
        if let Some(helper) = runtime::helper_for_call(target) {
            return self.lower_runtime_helper_call(helper, target, args, true);
        }
        // Future-proofing backstop: an inline-lowered builtin has no
        // standalone symbol, so the generic raw path below would emit
        // `bl <target>` to a non-existent symbol. After plan-26 every inline
        // builtin is either raw-supported or infallible (both handled above),
        // so this never fires today; it fails loudly if a *future* inline
        // builtin is added to `native_builtin_target` without a raw or
        // infallible lowering, instead of miscompiling.
        if crate::codegen::builtins::inline_trap_unsupported(
            target,
            &inline_arg_types,
            &crate::codegen::builtins::NoTypeKinds,
        ) {
            return Err(format!(
                "internal: inline TRAP reached inline-lowered builtin '{target}' \
                         without a raw or infallible lowering; add one to \
                         lower_inline_builtin_raw / lower_inline_infallible_raw"
            ));
        }
        let symbol = self
            .function_symbols
            .get(target)
            .cloned()
            .unwrap_or_else(|| target.to_string());
        let success_type_typed = self
            .functions
            .get(target)
            .map(|function| function.returns.clone())
            .or_else(|| self.package_return_types.get(target).cloned())
            .or_else(|| {
                // A static descriptor name — a registry-literal boundary.
                builtins::call_return_type(target)
            })
            .ok_or_else(|| format!("native raw result call '{target}' has no return type"))?;
        let success_type = success_type_typed.clone();
        let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
        let value_slot = self.allocate_stack_object("raw_result_value", 8);
        let message_slot = self.allocate_stack_object("raw_result_message", 8);
        let source_slot = self.allocate_stack_object("raw_result_source", 8);
        let unboxed = self.unboxed_trap_active.take();
        // plan-147-D: the trapped call form is an approved site like any other.
        let previous_call = self
            .current_call_key
            .replace(crate::codegen::engine::analysis::handover::call_key(value));
        let call = self.emit_call(target, &symbol, args, Some(&success_type.name()));
        self.current_call_key = previous_call;
        call?;
        self.emit(abi::store_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            tag_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            value_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        // Preserve the callee's error origin (x3) so an inline-trapped
        // error keeps its original source location.
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_slot,
        ));
        // bug-841: `EXIT PROGRAM` is not the handler's to catch.
        self.emit_inline_trap_program_exit_passthrough(tag_slot, value_slot)?;
        // bug-778: an unboxed temporary owns the callee's block exactly when
        // an owning `Bind` of the untrapped call would (`lower_value_owned`):
        // a param borrow or a rodata `String` is copied, anything else is
        // taken. Nothing is registered for the statement-scope free — the
        // temporary, then its reader's binding, is the owner.
        if let Some(unboxed) = unboxed {
            let owner = if self.value_needs_owning_copy(value) {
                UnboxedPayloadSource::Copy
            } else {
                UnboxedPayloadSource::Owned
            };
            return self.finish_unboxed_trap_result(
                unboxed,
                tag_slot,
                value_slot,
                message_slot,
                source_slot,
                owner,
                TrappedErrorSource::CalleeRegister,
                format!("callResult {target}"),
            );
        }
        let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
        let wrap_error_label = self.label("result_wrap_error");
        let have_payload_label = self.label("result_have_payload");
        let result_slot = self.allocate_stack_object("raw_result", 8);
        self.emit(abi::load_u64(scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(scratch9, RESULT_OK_TAG));
        self.emit(abi::branch_ne(&wrap_error_label));
        self.emit(abi::load_u64(scratch9, abi::stack_pointer(), value_slot));
        self.emit(abi::store_u64(scratch9, abi::stack_pointer(), payload_slot));
        let ok_result = self.emit_build_result_inline(tag_slot, &success_type, payload_slot)?;
        self.emit(abi::store_u64(
            &ok_result,
            abi::stack_pointer(),
            result_slot,
        ));
        // bug-561: the `Result` above owns a COPY of the callee's block;
        // the callee's own block is dead from here. Ok path only —
        // `value_slot` holds an error code on the other branch.
        self.register_call_result_payload_temp(value, &success_type, payload_slot);
        self.emit(abi::branch(&have_payload_label));
        self.emit(abi::label(&wrap_error_label));
        // bug-565: adopt a parked `ERR_BLOCK` instead of orphaning it, and
        // free the payload block the `Result` copied.
        self.emit_trapped_error_result(
            scratch9,
            tag_slot,
            value_slot,
            message_slot,
            source_slot,
            payload_slot,
            result_slot,
            TrappedErrorSource::CalleeRegister,
        )?;
        self.emit(abi::label(&have_payload_label));
        Ok(self.fresh_trapped_result_value(
            result_slot,
            success_type_typed.clone(),
            format!("callResult {target}"),
        ))
    }
}
