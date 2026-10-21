//! plan-176-U: `lower_value_inner`, the per-`NirValue` lowering. The entry points and shared
//! types live in `engine/value/builder_values/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_value_inner(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let scratch9_reg = self.temporary_vreg();
        let scratch10_reg = self.temporary_vreg();
        let scratch9 = &scratch9_reg;
        let scratch10 = &scratch10_reg;
        if let Some(string_value) = self.static_string_value(value) {
            let register = self.load_string_constant(&string_value)?;
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::String,
                location: Operand::from(register),
                text: format!("String({string_value})"),
            });
        }
        match value {
            NirValue::Const { type_, value } => {
                // A Const String is always intercepted by `static_string_value`
                // above (`static_string_value`, builder_value_semantics/static_eval.rs, returns Some for it), so this
                // arm only reaches non-String scalar constants. bug-175 C: the dead
                // `*type_ == ParameterType::String` branch was removed.
                let register = self.allocate_register();
                let immediate = native_immediate_value(&type_, value)?;
                self.emit(abi::move_immediate(
                    &register,
                    &abi::immediate_class(&type_),
                    &immediate,
                ));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("{type_}({value})"),
                })
            }
            NirValue::Local(name) => {
                if self
                    .type_model
                    .union_variants
                    .contains_key(&ParameterType::declared(name))
                {
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::named("VariantTag"),
                        location: Operand::from(name.clone()),
                        text: name.clone(),
                    });
                }
                // A promoted small-vector local lives in its lanes, not a slot
                // (plan-01-vector): reconstruct a register-native view from them.
                // The lanes are shared register values (vectors are immutable), so
                // every read and any later materialization sees the same value.
                if let Some((type_, lanes)) = self.promoted_vector_locals.get(name).cloned() {
                    return Ok(self.make_vector_native(&type_, lanes));
                }
                // A loop-promoted float local lives in an FP register, not its
                // slot (plan-03 Stage D part 2). Its FP register *is* the value's
                // home under the `d`-native value model, so return it directly —
                // no GPR materialization (plan-01 float-dnative).
                if let Some(d) = self.promoted_float_locals.get(name).cloned() {
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::Float,
                        location: Operand::from(d),
                        text: name.clone(),
                    });
                }
                let local = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("native code local '{name}' does not resolve"))?;
                let type_ = local.type_.clone();
                let type_name = type_.clone();
                let stack_offset = local.stack_offset;
                let by_ref = local.by_ref;
                // A non-aliased `Float` local loads straight into an FP register
                // (`ldr d`) under the `d`-native model, so it feeds float
                // arithmetic with no `ldr x` + `fmov` shuttle (plan-01
                // float-dnative). A `by_ref` local needs a pointer deref first, so
                // it stays on the GPR path.
                if matches!(type_, ParameterType::Float) && !by_ref {
                    let d = self.allocate_fp_register();
                    self.emit(abi::load_double(&d, abi::stack_pointer(), stack_offset));
                    return Ok(ValueResult {
                        origin: None,
                        type_: type_name.clone(),
                        location: Operand::from(d),
                        text: name.clone(),
                    });
                }
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, abi::stack_pointer(), stack_offset));
                if by_ref {
                    // A reference local's slot holds a pointer to the parent
                    // binding's slot; deref it to read the live value/block
                    // pointer. For a scalar this yields the value; for a block it
                    // yields the block pointer (an alias into the block).
                    self.emit(abi::load_u64(&register, &register, 0));
                }
                Ok(ValueResult {
                    origin: None,
                    type_: type_name.clone(),
                    location: Operand::from(register),
                    text: name.clone(),
                })
            }
            NirValue::LocalRef { name, type_ } => {
                // The address of the binding's slot (a reference to the slot), used to
                // seed a non-escaping callback's env so the callback observes and
                // updates the live binding. The callback may
                // change the binding through this reference, so any folded constant the
                // outer scope held for it is now stale and must be cleared, else a
                // later read folds to the pre-call value.
                let local = self
                    .locals
                    .get_mut(name)
                    .ok_or_else(|| format!("native code local ref '{name}' does not resolve"))?;
                let stack_offset = local.stack_offset;
                local.constant = None;
                let register = self.allocate_register();
                self.emit(abi::add_immediate(
                    &register,
                    abi::stack_pointer(),
                    stack_offset,
                ));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("&{name}"),
                })
            }
            NirValue::Global { name, .. } => {
                // A `Global` node carries no type of its own (`nir::lower` gives every
                // one `named("")`); the declaration has it.
                let global = self.global_value(name)?;
                let value_type = global.type_.clone();
                let address = self.load_global_address(name)?;
                // A `Float` global loads straight into an FP register under the
                // `d`-native model (plan-01 float-dnative), mirroring the local
                // load path.
                if matches!(value_type, ParameterType::Float) {
                    let d = self.allocate_fp_register();
                    self.emit(abi::load_double(&d, &address, 0));
                    return Ok(ValueResult {
                        origin: None,
                        type_: value_type.clone(),
                        location: Operand::from(d),
                        text: name.clone(),
                    });
                }
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, &address, 0));
                Ok(ValueResult {
                    origin: None,
                    type_: value_type.clone(),
                    location: Operand::from(register),
                    text: name.clone(),
                })
            }
            NirValue::FunctionRef { name, type_ } => {
                // A no-capture function value is the address of a STATIC closure
                // descriptor (`{code = &func, env = 0}`) — one per function, in BSS,
                // populated once at startup (see `collect_function_value_refs` +
                // the entry). Load its address instead of arena-allocating a fresh
                // descriptor on every evaluation, so a lambda in a loop no longer
                // grows the arena (bug-78). All indirect-call/env-access consumers
                // read `{code, env}` off this pointer exactly as before.
                let symbol = builtin_function_symbol_for_type(name, &type_)
                    .or_else(|| self.function_symbols.get(name).cloned())
                    .unwrap_or_else(|| name.clone());
                let desc_symbol = closure_descriptor_symbol(&symbol);
                let closure_register = self.allocate_register();
                self.emit(abi::load_page_address(&closure_register, &desc_symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: desc_symbol.clone(),
                    kind: RelocIntent::DataAddrHi,
                    binding: "data".to_string(),
                    library: None,
                });
                self.emit(abi::add_page_offset(
                    &closure_register,
                    &closure_register,
                    &desc_symbol,
                ));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: desc_symbol,
                    kind: RelocIntent::DataAddrLo,
                    binding: "data".to_string(),
                    library: None,
                });
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(closure_register),
                    text: name.clone(),
                })
            }
            NirValue::Closure {
                name,
                type_,
                captures,
            } if captures.is_empty() => {
                // bug-781: a capture-less closure owns nothing, so it is the static
                // descriptor a `FunctionRef` loads — `env = 0` then marks exactly
                // the function values no holder frees or copies.
                self.lower_value_inner(&NirValue::FunctionRef {
                    name: name.clone(),
                    type_: type_.clone(),
                })
            }
            NirValue::Closure {
                name,
                type_,
                captures,
            } => self.lower_closure_value(name, type_, captures),
            NirValue::Capture { index, type_, .. } => {
                // Load the env slot's raw word. For a by-value capture this is the
                // value/block pointer; for a by-ref capture (`by_ref`) it is the
                // pointer to the parent binding's slot, which `Bind` installs into
                // a reference local that derefs on read/write.
                let register = self.allocate_register();
                self.emit(abi::load_u64(
                    &register,
                    CLOSURE_ENV_REGISTER,
                    closure_env_word_offset(*index),
                ));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("capture[{index}]"),
                })
            }
            NirValue::Call { target, args, loc } => {
                self.lower_call_value(value, target, args, *loc)
            }
            NirValue::CallResult { target, args, .. } => {
                self.lower_call_result_value(value, target, args, scratch9)
            }
            NirValue::RuntimeCall {
                helper,
                target,
                args,
                ..
            } => {
                // plan-116-J: a consuming parameter takes over closing the resources
                // reachable from its argument, so this scope stops closing them.
                //
                // THIS is where a `Body::abi_function` member's call site arrives, and
                // finding that took three wrong guesses worth recording: not
                // `lower_value`'s `Call` arm, not its `CallResult` arm, and not
                // `emit_call`. `canvas::setGroup` lowers to
                // `{"kind": "runtimeCall", "helper": "canvas", "target":
                // "canvas.setGroup"}` — a NIR node of its own, which none of those three
                // see. The `--nir` dump is what settled it; a trace of 798 lowered calls
                // through `emit_call` did not contain the one call the program makes.
                self.deactivate_consumed_cleanups(target, args);
                // The predicate intrinsics (`isEven`, `isEmpty`, ...) never arrive here:
                // `runtime::is_native_direct_call` lists them, so `nir::lower` builds a
                // `NirValue::Call` for them, which `lower_call_value` lowers.
                //
                // `typeName` does arrive here (`helper_for_call` classes it `General`),
                // but every one whose argument type resolves was already folded to a
                // string constant by `static_string_value` at the top of
                // `lower_value_inner`, through the same `static_type_name_for_fold`.
                // What is left is a `typeName` that resolver cannot type, and there
                // is no runtime helper to fall back to.
                if target == "typeName" && args.len() == 1 {
                    return Err("native code cannot determine typeName argument type".to_string());
                }
                self.lower_runtime_helper_call(*helper, target, args, false)
            }
            NirValue::Constructor { type_, args } => {
                // plan-01-vector: a small Float vector is constructed register-native
                // — its lanes stay in per-lane scalar-Float carriers with no
                // arena_alloc, materializing to the block only at a storage boundary
                // (`vector_value_as_block`). Each lane is finiteness-observed exactly
                // as the record-field boundary would (plan-17), so behavior is
                // bit-identical to the heap-record constructor.
                if let Some(count) = vector_field_count(type_) {
                    if args.len() == count {
                        let mut lanes = Vec::with_capacity(count);
                        for arg in args {
                            let value = self.lower_value(arg)?;
                            self.observe_float(arg, &value)?;
                            lanes.push(value);
                        }
                        return Ok(self.make_vector_native(type_, lanes));
                    }
                }
                // A fresh nested owned block passed as a field (e.g. the `ErrorLoc`
                // inside `error(...)`) is registered as a pending temp while lowering
                // the arg, then byte-INLINED (copied) into this record — so the
                // standalone arg block is dead the moment the record is built. On a
                // normal statement it is reclaimed by the statement-scope drop, but a
                // `FAIL` (and other control transfers) CLEARS pending temps instead of
                // freeing them, orphaning it (a per-caught-error leak). Free those
                // consumed arg temps right here so the record is self-contained on
                // every path (plan-25 comment: a record temp is a single arena_free).
                // plan-118-D: the per-TYPE census behind the synthesis threshold.
                // `val:Constructor` in the expansion tally is the aggregate; this
                // splits it by record/variant type so "which types are constructed
                // often enough for a shared function to amortize" is a measurement
                // rather than an assumption. Inclusive of the arguments' own
                // lowering (the aggregate row is the exclusive one), which is the
                // right reading here: the question is what a construction site
                // costs in total. Counted under `-vv` only.
                let census_start = self.instructions.len();
                let arg_temp_watermark = self.pending_temp_frees.len();
                let mut arg_values = Vec::new();
                let mut arg_slots = Vec::new();
                for arg in args {
                    // plan-134-E: a recursive argument is stored into the new record.
                    let value = self.lower_value_stored_field(arg)?;
                    // Observation boundary: a `Float` record/union field must be
                    // finite (plan-17).
                    self.observe_float(arg, &value)?;
                    // A register-native vector field or a `d`-native float is
                    // materialized before the field-payload spill (plan-01).
                    let value = self.materialize_value(value)?;
                    let slot = self.allocate_stack_object("constructor_arg", 8);
                    self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
                    arg_values.push(value);
                    arg_slots.push(slot);
                }
                if self.type_model.record_fields.contains_key(type_) {
                    // A record inlines its `String` fields into a trailing data
                    // region (the slot holds a block-relative offset); scalar and
                    // pointer fields stay inline at `8*index` (plan-02 §4.2).
                    //
                    // plan-118-D: for a type constructed often enough to amortize
                    // it, all of that — the allocation, the failure block, the
                    // field stores, a byte-copy loop per String field — is in
                    // `construct.T` and this site marshals and calls. The helper
                    // is built by replaying THIS emitter, so the layout law is
                    // not forked.
                    let register = if self.synthesized_constructors.contains(type_) {
                        self.emit_construct_helper_call(type_, &arg_slots)?
                    } else {
                        self.emit_build_inlined_record(type_, &arg_slots)?
                    };
                    // The record now owns byte-inlined copies of every field, so the
                    // consumed nested arg blocks are dead — free them (the record
                    // register is live across these frees and preserved by the vreg
                    // allocator). Spill/reload the record pointer so the `arena_free`
                    // calls cannot clobber it.
                    let result_slot = self.allocate_stack_object("constructor_result", 8);
                    self.emit(abi::store_u64(&register, abi::stack_pointer(), result_slot));
                    self.drop_arg_block_temps_to(arg_temp_watermark)?;
                    let result = self.temporary_vreg();
                    self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
                    crate::trace::count_tally(
                        "constructor type",
                        || format!("record {type_}"),
                        (self.instructions.len() - census_start) as u64,
                    );
                    return Ok(ValueResult {
                        origin: None,
                        type_: type_.clone(),
                        location: Operand::from(result),
                        text: format!("construct {type_}({})", join_texts(&arg_values)),
                    });
                }
                // Not a record, and there is nothing else a `Constructor` can
                // name. A union is all-data or all-resource
                // (`TYPE_MIXED_RESOURCE_UNION`): every variant of an all-data
                // union is a `TYPE ... END TYPE` record and is therefore in
                // `record_fields` above, and a resource has no constructor
                // syntax at all — it comes from a function. So a `Constructor`
                // whose type is missing from `record_fields` is a lowering bug,
                // and this fails CLOSED rather than guessing a layout for it.
                //
                // bug-548: ~85 lines of union-variant construction stood here,
                // building a tagged block the way `UnionWrap` does. Nothing
                // reached it. Four sweeps agreed — every unit test, every one of
                // ~700 committed projects, and then, with the instrumentation
                // still in, an imported package's variant (`json::JsonNum[1.5]`),
                // a LOCAL union over imported record variants, and a mixed
                // resource/data union, which the type checker refuses outright.
                // That refusal is what turned "no program reaches it" into "no
                // program can": it removes the one variant kind that is not a
                // record.
                Err(format!(
                    "native code constructor type '{type_}' is not a record; a \
                     union variant is either a record (constructed above) or a \
                     resource (which has no constructor)"
                ))
            }
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => self.lower_union_wrap_value(union_type, member_type, value, scratch9, scratch10),
            NirValue::UnionExtract { type_, value } => {
                // A resource-union variant's payload is the resource pointer
                // itself (offset 8): extracting it yields that pointer directly.
                if self.is_resource_nominal(type_) {
                    let source = self.lower_value(value)?;
                    let register = self.allocate_register();
                    self.emit(abi::load_u64(&register, &source.location, 8));
                    return Ok(ValueResult {
                        origin: None,
                        type_: type_.clone(),
                        location: Operand::from(register),
                        text: format!("extract {type_} from {}", source.text),
                    });
                }
                // A data union inlines the active variant's flat record block at
                // +16 (plan-02 §4.3); the extracted record is an alias into the
                // union at that offset.
                let source = self.lower_value(value)?;
                let register = self.allocate_register();
                self.emit(abi::add_immediate(&register, &source.location, 16));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: format!("extract {type_} from {}", source.text),
                })
            }
            NirValue::ResultIsOk { value } => {
                if let Some(result) = self.lower_unboxed_result_is_ok(value) {
                    return result;
                }
                let result = self.lower_value(value)?;
                let register = self.allocate_register();
                let ok_label = self.label("result_is_ok_true");
                let end_label = self.label("result_is_ok_end");
                self.emit(abi::load_u64(scratch9, &result.location, 0));
                self.emit(abi::compare_immediate(scratch9, RESULT_OK_TAG));
                self.emit(abi::branch_eq(&ok_label));
                self.emit(abi::move_immediate(&register, "Boolean", "0"));
                self.emit(abi::branch(&end_label));
                self.emit(abi::label(&ok_label));
                self.emit(abi::move_immediate(&register, "Boolean", "1"));
                self.emit(abi::label(&end_label));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::Boolean,
                    location: Operand::from(register),
                    text: "resultIsOk".to_string(),
                })
            }
            NirValue::ResultValue { value } => {
                if let Some(result) = self.lower_unboxed_result_value(value) {
                    return result;
                }
                let result = self.lower_value(value)?;
                let crate::types::ParameterType::ResultOf(payload) = &result.type_ else {
                    return Err(format!(
                        "native RESULT_VALUE requires raw Result input, got `{}`",
                        result.type_
                    ));
                };
                let type_ = (**payload).clone();
                // The payload is inlined at +16 (plan-02 §4.3): a block payload
                // yields an alias pointer into the Result; a scalar payload is the
                // 8-byte value.
                let register = self.allocate_register();
                if type_ == ParameterType::Nothing {
                    // bug-744: a `Nothing` callee leaves `RESULT_VALUE_REGISTER`
                    // unset, so the payload `materialize_current_result` boxed from
                    // it is whatever that register last held. The one `Nothing`
                    // value is `NOTHING` (0); produce it rather than read the slot.
                    self.emit(abi::move_immediate(&register, "Integer", "0"));
                } else if self.result_payload_is_block(&type_) {
                    self.emit(abi::add_immediate(&register, &result.location, 16));
                } else {
                    self.emit(abi::load_u64(&register, &result.location, 16));
                }
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register),
                    text: "resultValue".to_string(),
                })
            }
            NirValue::ResultError { value } => {
                if let Some(result) = self.lower_unboxed_result_error(value) {
                    return result;
                }
                let result = self.lower_value(value)?;
                // The error payload (a flat Error block) is inlined at +16.
                let register = self.allocate_register();
                self.emit(abi::add_immediate(&register, &result.location, 16));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::named("Error"),
                    location: Operand::from(register),
                    text: "resultError".to_string(),
                })
            }
            NirValue::Checked {
                type_,
                value,
                observe,
            } => self.lower_checked_value(&type_, &value, *observe),
            NirValue::WithUpdate {
                type_,
                target,
                updates,
            } => {
                // bug-766 B: the item a `set`/`append` self-update copies, built in
                // the record-temp scratch.
                let node = value as *const NirValue as usize;
                if self.record_temps.with_item_armed == Some(node) {
                    self.record_temps.with_item_armed = None;
                    let result = self.lower_with_update_in_record_temp(type_, target, updates)?;
                    self.record_temps.placed = Some(node);
                    return Ok(result);
                }
                self.lower_with_update(&type_, target, updates)
            }
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                // `.result` is an ordinary record field: the thread `t.result` read
                // is removed (`ir::verify` refuses it, TYPE_THREAD_RESULT_REMOVED).
                _ if member == "result" => self.lower_field_access(target, member),
                NirValue::Local(type_name) => {
                    if let Some(ordinal) = self
                        .type_model
                        .enum_members
                        .get(&(ParameterType::declared(type_name), member.clone()))
                        .copied()
                    {
                        let register = self.allocate_register();
                        self.emit(abi::move_immediate(
                            &register,
                            abi::IMMEDIATE_CLASS_ENUM_ORDINAL,
                            &ordinal.to_string(),
                        ));
                        return Ok(ValueResult {
                            origin: None,
                            type_: ParameterType::declared(type_name),
                            location: Operand::from(register),
                            text: format!("{type_name}.{member}"),
                        });
                    }
                    self.lower_field_access(target, member)
                }
                _ => self.lower_field_access(target, member),
            },
            NirValue::Binary {
                op, left, right, ..
            } => {
                if *op == BinaryOp::Concat {
                    // String concat / rope fusion (Level 3): `&` is
                    // left-associative, so `a & b & c` arrives as
                    // `(a & b) & c` and the pairwise lowering would allocate
                    // and fill a whole intermediate for `a & b` only to copy
                    // it again. Flatten the left spine and lower the chain
                    // into one pre-sized allocation instead. Two operands are
                    // left to the pairwise path verbatim, so ordinary `a & b`
                    // is untouched at every level.
                    let mut parts: Vec<&NirValue> = Vec::new();
                    flatten_concat_spine(left, &mut parts);
                    parts.push(right);
                    if parts.len() > 2 && crate::optimizer::level_enabled(3) {
                        let fused = parts.len() as u64 - 1;
                        let result = self.lower_string_concat_chain(&parts)?;
                        crate::optimizer::stats::count_string_concats_fused(fused);
                        return Ok(result);
                    }
                    return self.lower_string_concat(left, right);
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor) {
                    return self.lower_boolean_binary(*op, left, right);
                }
                if op.is_comparison() {
                    return self.lower_comparison_binary(*op, left, right);
                }
                self.lower_arithmetic_binary(*op, left, right)
            }
            NirValue::Unary { op, operand, .. } => {
                let operand = self.lower_value(operand)?;
                if *op == UnaryOp::Not && operand.type_ == ParameterType::Boolean {
                    let register = self.allocate_register();
                    let true_label = self.label("bool_not_true");
                    let done_label = self.label("bool_not_done");
                    self.emit(abi::compare_immediate(&operand.location, "0"));
                    self.emit(abi::branch_eq(&true_label));
                    self.emit(abi::move_immediate(&register, "Boolean", "false"));
                    self.emit(abi::branch(&done_label));
                    self.emit(abi::label(&true_label));
                    self.emit(abi::move_immediate(&register, "Boolean", "true"));
                    self.emit(abi::label(&done_label));
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::Boolean,
                        location: Operand::from(register),
                        text: format!("(NOT {})", operand.text),
                    });
                }
                if *op == UnaryOp::Negate
                    && matches!(
                        operand.type_.name().as_ref(),
                        "Byte" | "Integer" | "Fixed" | "Float" | "Money"
                    )
                {
                    return self.lower_numeric_unary_negation(operand);
                }
                // `NOT` on a non-Boolean and `-` on a non-numeric are both
                // rejected by `ir::verify` before lowering, and `SIZEOF` folds
                // during LINK lowering; a node reaching here is malformed IR,
                // not an unlowered operator.
                Err(format!(
                    "native code plan does not lower unary operator '{}' for {} yet while lowering native function '{}'",
                    op.name(),
                    operand.type_,
                    self.current_symbol
                ))
            }
            NirValue::ListLiteral { type_, values } => self.lower_list_literal(&type_, values),
            NirValue::SetLiteral { type_, values } => self.lower_set_literal(&type_, values),
            NirValue::MapLiteral { type_, entries } => self.lower_map_literal(&type_, entries),
        }
    }
}
