// --- codegen tier imports (migration) ---
use crate::codegen::builtins;
use crate::codegen::builtins::vector::vector_field_count;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::function::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::target::shared::runtime;
use crate::types::ParameterType;

impl<'a> CodeBuilder<'a> {
    /// Build an [`AbiCtx`] from the builder's own platform/imports/build-mode for an
    /// inline (`abi_inline`) lowering. The fields are `&'a`/`Copy`,
    /// so the returned `AbiCtx` borrows the underlying build data, not `self` — leaving
    /// `self` free to pass mutably to the lowering. The inline path has no OS-seam arena
    /// offsets (that is an `abi_function` concern), so both are `None`.
    pub(crate) fn inline_abi_ctx(&self) -> crate::codegen::registry::AbiCtx<'a> {
        crate::codegen::registry::AbiCtx {
            platform_imports: self.platform_imports,
            platform: self.platform,
            build_mode: self.build_mode,
            // The inline path takes no resource-path member, so no module identity
            // is needed (that is an `abi_function` concern).
            module_name: "",
            term_state_offset: None,
            presentation_mode_offset: None,
            // The inline path lowers per call site and hands the body its raw args +
            // target directly; no `abi_function` runtime-call name applies.
            call: "",
            // Worker-spawn build state (`thread.start`) is an `abi_function` concern;
            // the inline path never lowers `thread.start`.
            arena_global_slots: 0,
            uses_rng: false,
        }
    }
}

/// Flatten the left-leaning spine of a `&` chain into its operands, in source
/// order. `a & b & c` parses as `(a & b) & c`, so the chain is the left
/// descendants; the caller appends the right operand itself.
///
/// The spine stops at the first non-`&` value, so a parenthesized subchain on
/// the right (`a & (b & c)`) keeps its own grouping and is lowered as its own
/// fused chain — the operand order the source wrote is preserved either way.
pub(super) fn flatten_concat_spine<'a>(value: &'a NirValue, out: &mut Vec<&'a NirValue>) {
    if let NirValue::Binary {
        op, left, right, ..
    } = value
    {
        if *op == BinaryOp::Concat {
            flatten_concat_spine(left, out);
            out.push(right);
            return;
        }
    }
    out.push(value);
}

impl CodeBuilder<'_> {
    pub(crate) fn lower_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        // Track the source location of the node being lowered so that any error
        // freshly created while lowering it (overflow, divide-by-zero, helper
        // failure, conversion failure) stamps a real `ErrorLoc`. The save/restore
        // ensures that after recursively lowering operands/arguments the outer
        // node's location is back in place before its own fallible emit runs.
        let saved_loc = self.current_loc;
        if let Some(loc) = value_loc(value) {
            self.current_loc = loc;
        }
        // plan-118-A: attribute the instructions this expression emits to its
        // kind (and call target). Exclusive — an operand's own frame subtracts
        // itself from this one — so the report partitions the module.
        crate::codegen::engine::expansion::enter(
            || crate::codegen::engine::expansion::value_key(value),
            self.instructions.len(),
        );
        // bug-496: note which of this node's operands must be snapshotted before
        // a later sibling's call can reassign the storage they point into.
        let snapshot_mark = self.push_operand_snapshot_frame(value);
        let result = self.lower_value_inner(value);
        self.operand_snapshot_wanted.truncate(snapshot_mark);
        crate::codegen::engine::expansion::exit(self.instructions.len());
        self.current_loc = saved_loc;
        if let Ok(result) = &result {
            self.register_pending_temp(value, result);
        }
        // bug-496: if THIS value is such an operand of the enclosing node, hand
        // its consumer a statement-scope deep copy instead of the live pointer.
        let mut result = result.and_then(|result| self.snapshot_aliased_operand(value, result));
        // Stamp the source `NirValue` so a pre-lowered `abi_inline` body can run any
        // NIR-structural analysis (bounds-check elision, float-finiteness, folding)
        // off the `ValueResult` — the value is pre-lowered, but the details are kept.
        if let Ok(vr) = &mut result {
            if vr.origin.is_none() {
                vr.origin = Some(value.clone());
            }
        }
        result
    }

    /// Register a freshly produced, freeable-flat heap value as a statement-scope
    /// temporary to be freed unless an owner claims it (plan-25 temp-lifetime
    /// fix). Only *fresh arena blocks* qualify — exactly the values copy-insertion
    /// treats as ownable without a copy: not an aliasing source / static string
    /// (`value_needs_owning_copy`), not runtime-managed (thread-owned), and a
    /// freeable-flat type. The block pointer is spilled to a fresh slot so the
    /// eventual `arena_free` survives the intervening register clobbers; the live
    /// register in `result` is left untouched for the immediate consumer.
    fn register_pending_temp(&mut self, value: &NirValue, result: &ValueResult) {
        // plan-86 E: a read-only `get`-borrow returns an ALIAS into the container's
        // data region (not a fresh block), so it must NOT be registered for the
        // statement-scope free — freeing it would `arena_free` INTO the container and
        // corrupt the free list. The `borrow_get_result` flag is set only while
        // lowering such a borrow's initializer.
        if self.borrow_get_result {
            return;
        }
        // A register-native vector has no arena block yet; it is registered as a
        // temp only when materialized (`vector_value_as_block`), so skip it here
        // (its marker location is not a real block pointer to spill/free).
        if Self::is_vector_native(result) {
            return;
        }
        if !self.is_freeable_flat_value(&result.type_)
            || self.value_needs_owning_copy(value)
            || Self::value_is_runtime_managed(value)
        {
            return;
        }
        // A bare `String` result is conservatively NOT freed here (plan-25). A
        // record/union/Result/collection temp is a self-contained fresh arena
        // block (a nested `String` field is byte-inlined, so one `arena_free`
        // reclaims it), but a *standalone* `String` produced by a call may be a
        // shared rodata constant NOT loaded through the tracked static-string
        // path, or a non-owned view into an argument — indistinguishable from a
        // fresh block at this point, and freeing one is a wild `arena_free` that
        // corrupts the arena. String temps therefore leak until scope exit as
        // they did pre-plan-25; the benchmark's poison is large *list* temps,
        // which are freed.
        if result.type_ == ParameterType::String {
            return;
        }
        let slot = self.allocate_stack_object("pending_temp", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
        self.pending_temp_frees.push(PendingTemp {
            type_: result.type_.clone(),
            slot,
            location: result.location.clone(),
        });
    }

    /// Exempt the just-produced temporary from the statement-scope free because an
    /// owning consumer (a binding, a `RETURN`, a resource `STATE` store, a
    /// thread-spawn move) now owns its block and will free it exactly once. The
    /// outermost node's temp is always the most recently registered, so matching
    /// the tail entry's origin register is precise.
    pub(crate) fn claim_pending_temp(&mut self, result: &ValueResult) {
        if self
            .pending_temp_frees
            .last()
            .is_some_and(|temp| temp.location == result.location)
        {
            self.pending_temp_frees.pop();
        }
    }

    /// Free every pending temporary registered above `watermark`, most-recent
    /// first (the scope-drop convention). Reuses the owned-value drop (null-guard +
    /// type-sized `arena_free`).
    pub(crate) fn drop_pending_temps_to(&mut self, watermark: usize) -> Result<(), String> {
        while self.pending_temp_frees.len() > watermark {
            let temp = self
                .pending_temp_frees
                .pop()
                .expect("watermark within bounds");
            self.emit_owned_value_drop(&OwnedValueCleanup {
                type_: temp.type_,
                stack_offset: temp.slot,
                closure_captures: None,
            })?;
        }
        Ok(())
    }

    /// Discard pending temporaries above `watermark` WITHOUT freeing them: used on
    /// control-transfer statements (`RETURN`/`EXIT`/`CONTINUE`/`Fail`) where the
    /// statement branches away (a returned temp is moved to the caller; any
    /// interior temp's free would be unreachable dead code after the branch).
    pub(crate) fn clear_pending_temps_to(&mut self, watermark: usize) {
        self.pending_temp_frees.truncate(watermark);
    }

    /// Lower a value that is being stored into a longer-lived or independently
    /// freed location (a `LET`/`MUT` binding, a global, a closure env, a returned
    /// value). plan-02 made every non-resource value a flat, pointer-free block,
    /// so a `memcpy` is a sound deep copy; this routine inserts that copy whenever
    /// the source is an **aliasing source** (a node that yields a pointer to an
    /// existing block rather than a fresh allocation). After copy-insertion every
    /// owned local owns an independent block, so plan-01 Phase 5 / plan-02 Phase 8
    /// can free each one exactly once at scope-drop with no double-free.
    ///
    /// Fresh-producing nodes (`Call`, `Constructor`, literals, `Binary`, …) and
    /// non-freeable types (scalars, resources, threads) are returned unchanged.
    pub(crate) fn lower_value_owned(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let result = self.lower_value(value)?;
        // A register-native vector reaching an owner boundary (a binding, global,
        // return, closure env) materializes to its block here — the block is
        // registered as a temp by `vector_value_as_block` and claimed just below,
        // so the owner owns it exactly as it would an eager `Constructor` block.
        if Self::is_vector_native(&result) {
            let block = self.vector_value_as_block(result)?;
            self.claim_pending_temp(&block);
            return Ok(block);
        }
        if self.value_needs_owning_copy(value) && self.is_freeable_flat_value(&result.type_) {
            let copied = self.copy_flat_block(&result.type_, &result.location)?;
            return Ok(ValueResult {
                origin: None,
                type_: result.type_,
                location: Operand::from(copied.render()),
                text: result.text,
            });
        }
        // A fresh value returned unchanged becomes this owner's block; claim its
        // pending-temp registration so the statement-scope free never double-frees
        // what scope-drop (or the consuming store) now owns (plan-25).
        self.claim_pending_temp(&result);
        Ok(result)
    }

    /// Whether lowering `value` yields a pointer this scope does **not** own — an
    /// alias into another value, or a *static* `String` constant in rodata
    /// (`static_string_value`). Either must be deep-copied into the arena before a
    /// binding/global/return can own it, so the eventual scope-drop `arena_free`
    /// reclaims a real arena block and never an aliased or static one.
    pub(crate) fn value_needs_owning_copy(&self, value: &NirValue) -> bool {
        Self::value_is_aliasing_source(value)
            || self.static_string_value(value).is_some()
            || Self::call_returns_rodata_string(value)
            || self.call_returns_param_borrow(value)
    }

    /// plan-118-B: whether `value` is a call to one of the internal Unicode
    /// property lookups, whose result is a pointer INTO A RODATA name table
    /// rather than a fresh arena block.
    ///
    /// `static_string_value` cannot see these: it classifies by knowing the
    /// string at compile time, and the category of a runtime scalar is only
    /// known at runtime. But the ownership consequence is identical to a string
    /// literal's — an owning store must deep-copy, and a bare temp must not be
    /// freed — and getting it wrong is the same crash a `typeName` fold mismatch
    /// causes a few lines below: scope-drop `arena_free`s a read-only constant,
    /// which writes the free list into rodata and takes SIGBUS. Observed exactly
    /// that on `LET cat AS String = regex::genCat(cp)` before this arm existed.
    fn call_returns_rodata_string(value: &NirValue) -> bool {
        matches!(
            value,
            NirValue::Call { target, .. } | NirValue::CallResult { target, .. }
                if matches!(target.as_str(), "regex.genCat" | "regex.scriptOf" | "strings.genCat")
        )
    }

    /// plan-86 K1: whether `value` is a call to a user function that returns a borrow
    /// of one of its parameters (`function_returns_param_borrow`). The result aliases
    /// the caller's argument block rather than a fresh allocation, so it is
    /// classified as an aliasing source exactly like a `Local`: `register_pending_temp`
    /// leaves it unfreed (the argument's owner frees it), and `lower_value_owned`
    /// deep-copies it at an owning store. This is the caller half of the elision — the
    /// callee (`lower_returned_value`, gated on the SAME predicate via
    /// `current_returns_param_borrow`) returns the argument pointer uncopied, so the
    /// single deep-copy lands here at the ownership boundary and value semantics is
    /// unchanged.
    fn call_returns_param_borrow(&self, value: &NirValue) -> bool {
        let target = match value {
            NirValue::Call { target, .. } | NirValue::CallResult { target, .. } => target.as_str(),
            _ => return false,
        };
        self.functions
            .get(target)
            .is_some_and(|f| function_returns_param_borrow(f, &self.callback_referenced_functions))
    }

    /// Whether lowering `value` yields a value whose lifetime is managed by the
    /// thread runtime, not by this scope: the result of a cross-thread data call
    /// (`thread::receive`/`read`/`waitFor`/`result`). Such a value lives in the
    /// thread's message plumbing and the worker arena that the runtime bulk-frees
    /// at teardown; scope-drop must not `arena_free` it (it may be a non-owning
    /// view, or already reclaimed on a cancel/timeout path), so its binding is not
    /// registered for an owned-value free — same exclusion principle as resources.
    pub(crate) fn value_is_runtime_managed(value: &NirValue) -> bool {
        let target = match value {
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => target.as_str(),
            NirValue::MemberAccess { member, .. } if member == "result" => return true,
            _ => return false,
        };
        target.starts_with("thread.") || target.starts_with("thread::")
    }

    /// Whether a `RES` bind's initializer merely names an **already-live**
    /// resource rather than producing one, making the bind an *alias* that
    /// registers no close obligation (§15.6: "a `RES` binding, a `RES`
    /// parameter, and a collection slot all hold a copy of the one handle
    /// pointer … none of these close the resource; the owning scope closes it
    /// exactly once on exit"). bug-375: classifying such a bind as an owner
    /// closed the caller's resource at the callee's exit, and the caller's next
    /// use failed with `7-703-0004`.
    ///
    /// Two shapes alias, and the boundary against *producing* is what keeps
    /// bug-374's leak fixed:
    ///
    /// * `Local` — naming an existing binding, `RES` parameter, or `FOR EACH`
    ///   loop variable. A producing bind never reaches here as a bare local at
    ///   the source level; where `TRAP` desugaring routes one through a temp
    ///   (`RES f = <fallible> TRAP` lowers to `bind f = local $trap_valN`), that
    ///   temp is itself a resource bind in the *same* scope and carries the
    ///   close obligation, so the resource is still closed exactly once.
    /// * `collections.get`/`getOr` — reading a resource element out of a
    ///   collection. Per §15.6 these "yield a pointer to the one resource",
    ///   never a transfer: the collection's owning scope closes it. Every other
    ///   call — `fs::openFile`, a user `FUNC … AS RES File` — *does* transfer
    ///   ownership to this binding and must keep its cleanup.
    /// * `tcp.poll`/`udp.poll`/`tls.poll` over a `List OF RES Socket`
    ///   (plan-76-A) — the readiness multiplex returns a pointer to the first
    ///   ready list element, the exact borrowed-element shape as
    ///   `collections::get`: the list still owns and closes it.
    pub(crate) fn value_aliases_live_resource(value: &NirValue) -> bool {
        match value {
            NirValue::Local(_) => true,
            // plan-114-C: `RES g = h.handle` reads a handle back OUT of a record
            // field. The record's scope owns that resource, so this bind must
            // register no close obligation of its own — registering one would
            // release the record's handle at this scope's exit, which is bug-375
            // one container-kind over.
            //
            // Safe to state unconditionally here because the only caller ANDs
            // this with "the bind is resource-typed"
            // (`builder_control.rs:697-699`: `resource_cleanup_symbol(type_)` or
            // `resource_union_cleanup(type_)`). A `MemberAccess` reading an
            // ordinary field is not resource-typed, so it never reaches the
            // alias branch and still takes the owned-value path below it.
            NirValue::MemberAccess { .. } => true,
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => {
                matches!(
                    crate::codegen::registry::native_bare_target(target),
                    Some("get" | "getOr")
                ) || matches!(target.as_str(), "tcp.poll" | "udp.poll" | "tls.poll")
            }
            _ => false,
        }
    }

    /// A NIR value node that yields a pointer to a **pre-existing** arena block
    /// (an alias) rather than a freshly allocated one. Storing such a
    /// value into an owned slot without copying would alias another owner, so
    /// [`lower_value_owned`](Self::lower_value_owned) deep-copies these.
    pub(crate) fn value_is_aliasing_source(value: &NirValue) -> bool {
        matches!(
            value,
            NirValue::Local(_)
                | NirValue::Global { .. }
                | NirValue::Capture { .. }
                | NirValue::MemberAccess { .. }
                | NirValue::UnionExtract { .. }
                | NirValue::ResultValue { .. }
                | NirValue::ResultError { .. }
        )
    }

    /// Whether `type_` is a flat, arena-allocated value block that scope-drop
    /// frees own and `arena_free` reclaims in one call — `String`, a flat record,
    /// a flat data union, a flat collection, or a flat `Result`. Scalars (stored
    /// inline by value), resources, threads, and recursive/non-flat composites are
    /// excluded: they are never freed by the generic owned-value path.
    pub(crate) fn is_freeable_flat_value(&self, type_: &ParameterType) -> bool {
        self.type_is_memcpy_copyable(type_)
            && (*type_ == ParameterType::String
                || typed_is_collection_type(type_)
                || matches!(type_, ParameterType::ResultOf(_))
                || self.type_model.record_fields.contains_key(type_)
                || self.union_is_data(type_))
    }

    /// plan-77 M6: the static type of a closure capture, used by the closure
    /// scope-drop to decide whether the env slot holds a freeable owned block.
    /// ONLY a `Local` capture qualifies: it is deep-copied (`lower_value_owned`)
    /// into its own arena block, so freeing it is sound. Everything else returns
    /// `""` so the drop LEAVES the slot — a by-ref capture (`LocalRef` / a
    /// `by_ref` `Capture`) holds a pointer to another binding's slot, NOT an owned
    /// block, and a by-value scalar/float is stored inline; freeing either would
    /// be a wild free. Leaving them is a safe (bounded, arena-reclaimed) leak.
    pub(crate) fn capture_free_type(&self, capture: &NirValue) -> ParameterType {
        match capture {
            NirValue::Local(name) => self
                .locals
                .get(name)
                .map(|local| local.type_.clone())
                // The EMPTY nominal is this tree's "no declared type" marker
                // (`type_utils::is_unset_type`); it was `String::new()` here.
                .unwrap_or_else(|| ParameterType::named("")),
            _ => ParameterType::named(""),
        }
    }

    fn lower_value_inner(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let scratch9_reg = self.temporary_vreg();
        let scratch10_reg = self.temporary_vreg();
        // A third scratch vreg was consumed by the union-wrap data-variant
        // field-copy loop (now removed as unreachable — every data variant
        // returns early via `emit_wrap_record_in_union`). The allocation is kept
        // so `next_vreg` — and therefore the byte-identical codegen it drives —
        // is unchanged.
        let _ = self.temporary_vreg();
        let scratch9 = &scratch9_reg;
        let scratch10 = &scratch10_reg;
        if let Some(string_value) = self.static_string_value(value) {
            let register = self.load_string_constant(&string_value)?;
            return Ok(ValueResult {
                origin: None,
                type_: ParameterType::String,
                location: Operand::from(register.render()),
                text: format!("String({string_value})"),
            });
        }
        match value {
            NirValue::Const { type_, value } => {
                // A Const String is always intercepted by `static_string_value`
                // above (builder_value_semantics.rs:562 returns Some for it), so this
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
                    location: Operand::from(register.render()),
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
                        location: Operand::from(d.render()),
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
                    location: Operand::from(register.render()),
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
                    location: Operand::from(register.render()),
                    text: format!("&{name}"),
                })
            }
            NirValue::Global { name, type_ } => {
                let global = self.global_value(name)?;
                let value_type = if type_.name().is_empty() {
                    global.type_.clone()
                } else {
                    type_.clone()
                };
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
                        location: Operand::from(d.render()),
                        text: name.clone(),
                    });
                }
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, &address, 0));
                Ok(ValueResult {
                    origin: None,
                    type_: value_type.clone(),
                    location: Operand::from(register.render()),
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
                    location: Operand::from(closure_register.render()),
                    text: name.clone(),
                })
            }
            NirValue::Closure {
                name,
                type_,
                captures,
            } => {
                let symbol = self
                    .function_symbols
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                let function_register = self.allocate_register();
                self.emit(abi::load_page_address(&function_register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol.clone(),
                    kind: RelocIntent::DataAddrHi,
                    binding: "data".to_string(),
                    library: None,
                });
                self.emit(abi::add_page_offset(
                    &function_register,
                    &function_register,
                    &symbol,
                ));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol,
                    kind: RelocIntent::DataAddrLo,
                    binding: "data".to_string(),
                    library: None,
                });
                let function_slot = self.allocate_stack_object("closure_code", 8);
                self.emit(abi::store_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                let env_slot = if captures.is_empty() {
                    None
                } else {
                    let env_register = self.allocate_register();
                    let env_slot = self.allocate_stack_object("closure_env", 8);
                    let alloc_ok = self.label("closure_env_alloc_ok");
                    let env_size = (captures.len() * 8).to_string();
                    self.emit(abi::move_immediate(
                        abi::return_register(),
                        "Integer",
                        &env_size,
                    ));
                    self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                    self.emit_arena_alloc_call();
                    self.emit(abi::branch_eq(&alloc_ok));
                    self.raise_error_bare("ErrOutOfMemory")?;
                    self.emit(abi::label(&alloc_ok));
                    self.emit(abi::move_register(&env_register, abi::mfb_return(1)));
                    self.emit(abi::store_u64(
                        &env_register,
                        abi::stack_pointer(),
                        env_slot,
                    ));
                    for (index, capture) in captures.iter().enumerate() {
                        // The closure env outlives the capturing scope, so it must
                        // own each captured flat value independently (plan-02
                        // Phase 8). `lower_value_owned` deep-copies an aliasing
                        // source; its `arena_alloc` clobbers caller-saved scratch
                        // (incl. `env_register`), so reload the env from its slot.
                        let value = self.lower_value_owned(capture)?;
                        // Observation boundary: a `Float` captured into the
                        // closure env is read back when the closure runs, so it
                        // must be finite (plan-17).
                        self.observe_float(capture, &value)?;
                        // Materialize a `d`-native float before storing it into
                        // the closure env (plan-01 float-dnative).
                        let value = self.materialize_float(value)?;
                        let env_register = self.allocate_register();
                        self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                        self.emit(abi::store_u64(&value.location, &env_register, index * 8));
                    }
                    Some(env_slot)
                };
                let closure_register = self.allocate_register();
                let alloc_ok = self.label("closure_alloc_ok");
                // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
                self.emit(abi::move_immediate(
                    abi::c_arg(0),
                    "Integer",
                    &CLOSURE_OBJECT_SIZE.to_string(),
                ));
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                self.emit_arena_alloc_call();
                self.emit(abi::branch_eq(&alloc_ok));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&alloc_ok));
                self.emit(abi::load_u64(
                    &function_register,
                    abi::stack_pointer(),
                    function_slot,
                ));
                self.emit(abi::store_u64(
                    &function_register,
                    abi::mfb_return(1),
                    CLOSURE_OFFSET_CODE,
                ));
                if let Some(env_slot) = env_slot {
                    let env_register = self.allocate_register();
                    self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
                    self.emit(abi::store_u64(
                        &env_register,
                        abi::mfb_return(1),
                        CLOSURE_OFFSET_ENV,
                    ));
                } else {
                    self.emit(abi::store_u64(
                        abi::ZERO,
                        abi::mfb_return(1),
                        CLOSURE_OFFSET_ENV,
                    ));
                }
                self.emit(abi::move_register(&closure_register, abi::mfb_return(1)));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(closure_register.render()),
                    text: name.clone(),
                })
            }
            NirValue::Capture { index, type_, .. } => {
                // Load the env slot's raw word. For a by-value capture this is the
                // value/block pointer; for a by-ref capture (`by_ref`) it is the
                // pointer to the parent binding's slot, which `Bind` installs into
                // a reference local that derefs on read/write.
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, CLOSURE_ENV_REGISTER, index * 8));
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: format!("capture[{index}]"),
                })
            }
            NirValue::Call { target, args, loc } => {
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
                    if matches!(local.type_, ParameterType::Func(_, _, false)) {
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
                                Operand::from(register.render())
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
                    if matches!(global.type_, ParameterType::Func(_, _, false)) {
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
                            location: Operand::from(register.render()),
                            text: target.clone(),
                        };
                        return self.emit_function_value_call(
                            target,
                            &callable,
                            args,
                            Some(&return_type),
                        );
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
                    let type_name = self.static_type_name_for_fold(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name.name())?;
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::String,
                        location: Operand::from(register.render()),
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
                if matches!(target.as_str(), "isPositive" | "isNegative" | "isZero")
                    && args.len() == 1
                {
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
                self.emit_call(target, &symbol, args, None)
            }
            NirValue::CallResult { target, args, .. } => {
                if let Some(local) = self.locals.get(target).cloned() {
                    if matches!(local.type_, ParameterType::Func(_, _, false)) {
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
                                Operand::from(register.render())
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
                        let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
                        let wrap_error_label = self.label("result_wrap_error");
                        let have_payload_label = self.label("result_have_payload");
                        let result_slot = self.allocate_stack_object("raw_result", 8);
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
                        let error_register =
                            self.emit_build_error_inline(value_slot, message_slot, source_slot)?;
                        self.emit(abi::store_u64(
                            &error_register,
                            abi::stack_pointer(),
                            payload_slot,
                        ));
                        let err_result = self.emit_build_result_inline(
                            tag_slot,
                            &ParameterType::named("Error"),
                            payload_slot,
                        )?;
                        self.emit(abi::store_u64(
                            &err_result,
                            abi::stack_pointer(),
                            result_slot,
                        ));
                        self.emit(abi::label(&have_payload_label));
                        let register = self.allocate_register();
                        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                        return Ok(ValueResult {
                            origin: None,
                            type_: ParameterType::result_of(return_type_typed.clone()),
                            location: Operand::from(register.render()),
                            text: format!("callResult {target}"),
                        });
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
                    if crate::codegen::builtins::inline_builtin_fallibility_depends_on_args(target)
                    {
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
                if crate::codegen::builtins::inline_builtin_raw_supported(target, &inline_arg_types)
                {
                    return self.lower_inline_builtin_raw(target, args);
                }
                // An inline `TRAP` on a provably-infallible inline built-in
                // (`len`, `toString`, `typeName`, `bits::*`, the pure-query/growth
                // collection members) is uniform surface (plan-26-A): the member
                // emits no error exit, so lower it normally and wrap the success as
                // an always-`Ok` `Result` for the inline-TRAP machinery. The handler
                // is dead code (front-end warns `TYPE_INLINE_TRAP_DEAD_HANDLER`).
                if crate::codegen::builtins::inline_builtin_is_infallible(target, &inline_arg_types)
                {
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
                if crate::codegen::builtins::inline_trap_unsupported(target, &inline_arg_types) {
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
                    .ok_or_else(|| {
                        format!("native raw result call '{target}' has no return type")
                    })?;
                let success_type = success_type_typed.clone();
                let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
                let value_slot = self.allocate_stack_object("raw_result_value", 8);
                let message_slot = self.allocate_stack_object("raw_result_message", 8);
                let source_slot = self.allocate_stack_object("raw_result_source", 8);
                let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
                let wrap_error_label = self.label("result_wrap_error");
                let have_payload_label = self.label("result_have_payload");
                let result_slot = self.allocate_stack_object("raw_result", 8);
                self.emit_call(target, &symbol, args, Some(&success_type.name()))?;
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
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), tag_slot));
                self.emit(abi::compare_immediate(scratch9, RESULT_OK_TAG));
                self.emit(abi::branch_ne(&wrap_error_label));
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), value_slot));
                self.emit(abi::store_u64(scratch9, abi::stack_pointer(), payload_slot));
                let ok_result =
                    self.emit_build_result_inline(tag_slot, &success_type, payload_slot)?;
                self.emit(abi::store_u64(
                    &ok_result,
                    abi::stack_pointer(),
                    result_slot,
                ));
                self.emit(abi::branch(&have_payload_label));
                self.emit(abi::label(&wrap_error_label));
                let error_register =
                    self.emit_build_error_inline(value_slot, message_slot, source_slot)?;
                self.emit(abi::store_u64(
                    &error_register,
                    abi::stack_pointer(),
                    payload_slot,
                ));
                let err_result = self.emit_build_result_inline(
                    tag_slot,
                    &ParameterType::named("Error"),
                    payload_slot,
                )?;
                self.emit(abi::store_u64(
                    &err_result,
                    abi::stack_pointer(),
                    result_slot,
                ));
                self.emit(abi::label(&have_payload_label));
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::result_of(success_type_typed.clone()),
                    location: Operand::from(register.render()),
                    text: format!("callResult {target}"),
                })
            }
            NirValue::RuntimeCall {
                helper,
                target,
                args,
                ..
            } => {
                // `strings::`/`astrings::` native members migrated to the clean-room
                // registry are `Body::abi_inline`, so they arrive on the `Call` path
                // (reached through `try_abi_inline_lower` there), not this `RuntimeCall`
                // arm — which handles the explicit intrinsics below (plan-99 PART B/C).
                if target == "isEven" && args.len() == 1 {
                    return self.lower_integer_parity_predicate("isEven", &args[0], false);
                }
                if target == "isOdd" && args.len() == 1 {
                    return self.lower_integer_parity_predicate("isOdd", &args[0], true);
                }
                if matches!(target.as_str(), "isPositive" | "isNegative" | "isZero")
                    && args.len() == 1
                {
                    return self.lower_numeric_filter_predicate(target, &args[0]);
                }
                if matches!(target.as_str(), "isEmpty" | "isNotEmpty") && args.len() == 1 {
                    return self.lower_empty_filter_predicate(target, &args[0]);
                }
                if target == "typeName" && args.len() == 1 {
                    let type_name = self.static_type_name_for_fold(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name.name())?;
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::String,
                        location: Operand::from(register.render()),
                        text: format!("typeName({type_name})"),
                    });
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
                    let value = self.lower_value(arg)?;
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
                    self.drop_pending_temps_to(arg_temp_watermark)?;
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
                        location: Operand::from(result.render()),
                        text: format!("construct {type_}({})", join_texts(&arg_values)),
                    });
                }
                let register = self.allocate_register();
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(type_)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{type_}' does not resolve")
                    })?;
                let union_name = self
                    .type_model
                    .union_variants
                    .get(type_)
                    .cloned()
                    .unwrap_or_else(|| type_.clone());
                // bug-175 C: size the union block the same way the `UnionWrap` path
                // does — a resource variant occupies one word (its handle pointer)
                // rather than being skipped, so a union mixing resource and data
                // variants allocates an identical block size on both paths.
                let union_size = self
                    .type_model
                    .variants_for_union(&union_name)
                    .map(|variant| {
                        if crate::codegen::builtins::is_resource_type(&variant) {
                            1
                        } else {
                            self.type_model
                                .union_variant_fields
                                .get(variant)
                                .map(Vec::len)
                                .unwrap_or(0)
                        }
                    })
                    .max()
                    .map(|max_payload| 8 * (1 + max_payload.max(1)))
                    .unwrap_or(8 * (arg_values.len() + 1));
                let result_slot = self.allocate_stack_object("union_result", 8);
                let alloc_ok = self.label("union_construct_alloc_ok");
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &union_size.to_string(),
                ));
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                self.emit_arena_alloc_call();
                self.emit(abi::branch_eq(&alloc_ok));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&alloc_ok));
                self.emit(abi::store_u64(
                    abi::mfb_return(1),
                    abi::stack_pointer(),
                    result_slot,
                ));
                let zero_register = self.allocate_register();
                self.emit(abi::move_immediate(&zero_register, "Integer", "0"));
                for offset in (0..union_size).step_by(8) {
                    self.emit(abi::store_u64(&zero_register, abi::mfb_return(1), offset));
                }
                let tag_register = self.allocate_register();
                self.emit(abi::move_immediate(
                    &tag_register,
                    abi::IMMEDIATE_CLASS_UNION_TAG,
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(&tag_register, abi::mfb_return(1), 0));
                for (index, slot) in arg_slots.iter().enumerate() {
                    self.emit(abi::load_u64(scratch9, abi::stack_pointer(), *slot));
                    self.emit(abi::store_u64(
                        scratch9,
                        abi::mfb_return(1),
                        8 * (index + 1),
                    ));
                }
                self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                crate::trace::count_tally(
                    "constructor type",
                    || format!("variant {type_}"),
                    (self.instructions.len() - census_start) as u64,
                );
                Ok(ValueResult {
                    origin: None,
                    type_: union_name.clone(),
                    location: Operand::from(register.render()),
                    text: format!("construct {type_}({})", join_texts(&arg_values)),
                })
            }
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => {
                let wrapped = self.lower_value(value)?;
                let wrapped_slot = self.allocate_stack_object("union_wrap_source", 8);
                self.emit(abi::store_u64(
                    &wrapped.location,
                    abi::stack_pointer(),
                    wrapped_slot,
                ));
                // A resource-union variant is a bare resource whose payload is
                // the resource pointer itself (one word at offset 8), not record
                // fields.
                let is_resource_variant = crate::codegen::builtins::is_resource_type(&member_type);
                let fields = if is_resource_variant {
                    Vec::new()
                } else {
                    self.type_model
                        .record_fields
                        .get(member_type)
                        .cloned()
                        .ok_or_else(|| {
                            format!("native code union wrap member '{member_type}' is not a record")
                        })?
                };
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(member_type)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{member_type}' does not resolve")
                    })?;
                // Data variant: build a flat `{tag, size, variant-record-block}`
                // union, inlining the wrapped variant record at +16 (plan-02
                // §4.3). Resource variants fall through to the fixed
                // `{tag, resource-ptr}` layout below.
                if !is_resource_variant {
                    let _ = &fields;
                    let register =
                        self.emit_wrap_record_in_union(member_type, tag, wrapped_slot)?;
                    return Ok(ValueResult {
                        origin: None,
                        type_: union_type.clone(),
                        location: Operand::from(register.render()),
                        text: format!("wrap {member_type} as {union_type}"),
                    });
                }
                // Payload words across all variants: a resource variant occupies
                // one word (the handle pointer); a record variant occupies its
                // field count.
                let max_payload = self
                    .type_model
                    .variants_for_union(union_type)
                    .map(|variant| {
                        if crate::codegen::builtins::is_resource_type(&variant) {
                            1
                        } else {
                            self.type_model
                                .union_variant_fields
                                .get(variant)
                                .map(Vec::len)
                                .unwrap_or(0)
                        }
                    })
                    .max()
                    .unwrap_or(if is_resource_variant { 1 } else { fields.len() });
                let union_size = 8 * (1 + max_payload.max(1));
                let result_slot = self.allocate_stack_object("union_result", 8);
                let alloc_ok = self.label("union_construct_alloc_ok");
                self.emit(abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &union_size.to_string(),
                ));
                self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
                self.emit_arena_alloc_call();
                self.emit(abi::branch_eq(&alloc_ok));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&alloc_ok));
                self.emit(abi::store_u64(
                    abi::mfb_return(1),
                    abi::stack_pointer(),
                    result_slot,
                ));
                let zero_register = self.allocate_register();
                self.emit(abi::move_immediate(&zero_register, "Integer", "0"));
                for offset in (0..union_size).step_by(8) {
                    self.emit(abi::store_u64(&zero_register, abi::mfb_return(1), offset));
                }
                let tag_register = self.allocate_register();
                self.emit(abi::move_immediate(
                    &tag_register,
                    abi::IMMEDIATE_CLASS_UNION_TAG,
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(&tag_register, abi::mfb_return(1), 0));
                // Only resource variants reach here — every data variant returned
                // early above via `emit_wrap_record_in_union`. A resource variant
                // stores its handle pointer as a single word at +8 (plan-02 §4.2).
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), wrapped_slot));
                self.emit(abi::load_u64(scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::store_u64(scratch9, scratch10, 8));
                let register = self.allocate_register();
                self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
                Ok(ValueResult {
                    origin: None,
                    type_: union_type.clone(),
                    location: Operand::from(register.render()),
                    text: format!("wrap {member_type} as {union_type}"),
                })
            }
            NirValue::UnionExtract { type_, value } => {
                // A resource-union variant's payload is the resource pointer
                // itself (offset 8): extracting it yields that pointer directly.
                if crate::codegen::builtins::is_resource_type(&type_) {
                    let source = self.lower_value(value)?;
                    let register = self.allocate_register();
                    self.emit(abi::load_u64(&register, &source.location, 8));
                    return Ok(ValueResult {
                        origin: None,
                        type_: type_.clone(),
                        location: Operand::from(register.render()),
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
                    location: Operand::from(register.render()),
                    text: format!("extract {type_} from {}", source.text),
                })
            }
            NirValue::ResultIsOk { value } => {
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
                    location: Operand::from(register.render()),
                    text: "resultIsOk".to_string(),
                })
            }
            NirValue::ResultValue { value } => {
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
                if self.result_payload_is_block(&type_) {
                    self.emit(abi::add_immediate(&register, &result.location, 16));
                } else {
                    self.emit(abi::load_u64(&register, &result.location, 16));
                }
                Ok(ValueResult {
                    origin: None,
                    type_: type_.clone(),
                    location: Operand::from(register.render()),
                    text: "resultValue".to_string(),
                })
            }
            NirValue::ResultError { value } => {
                let result = self.lower_value(value)?;
                // The error payload (a flat Error block) is inlined at +16.
                let register = self.allocate_register();
                self.emit(abi::add_immediate(&register, &result.location, 16));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::named("Error"),
                    location: Operand::from(register.render()),
                    text: "resultError".to_string(),
                })
            }
            NirValue::Checked { type_, value } => self.lower_checked_value(&type_, &value),
            NirValue::WithUpdate {
                type_,
                target,
                updates,
            } => self.lower_with_update(&type_, target, updates),
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                _ if member == "result" => {
                    if let Some(output_type) =
                        self.static_type_name(target).and_then(|type_| match type_ {
                            ParameterType::ThreadHandle {
                                worker: false, out, ..
                            } => Some((*out).clone()),
                            _ => None,
                        })
                    {
                        self.emit_raw_call(
                            &runtime::symbol_for_call(
                                runtime::RuntimeHelper::Thread,
                                "thread.waitFor",
                            ),
                            std::slice::from_ref(target.as_ref()),
                            "thread_result_arg",
                        )?;
                        return self.materialize_current_result(
                            &output_type,
                            "thread.result".to_string(),
                            true,
                        );
                    }
                    self.lower_field_access(target, member)
                }
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
                            location: Operand::from(register.render()),
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
                        location: Operand::from(register.render()),
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

    /// Lower a `Checked` node (bug-471): run `value`'s ordinary lowering under a
    /// `raw_result_capture` so every domain error it raises joins the capture
    /// point instead of the function's error exit, then tag the success
    /// fall-through `Ok` and materialize a `Result OF <success_type>`.
    ///
    /// This is the member-agnostic sibling of `lower_inline_conversion_raw` /
    /// `lower_inline_builtin_raw`: those wrap ONE built-in's inline lowering,
    /// this wraps an arbitrary raising *expression* — the arithmetic operators
    /// `ir::lower`'s inline-`TRAP` desugar lifts out of a trapped expression.
    /// Every one of them raises through `emit_error_register_return`, whose
    /// `raw_result_capture` branch is what makes this work; the `Float`
    /// observation-boundary check (`observe_float` → `emit_float_result_check`)
    /// funnels through the same tail.
    ///
    /// `value` never contains a call: a callee's error return does not pass
    /// through `emit_error_register_return` in this frame, so it would slip past
    /// the capture. The desugar lifts every call out ahead of the checked value
    /// and `ir::verify::check_checked_has_no_call` rejects the shape on the
    /// decoded-package path.
    fn lower_checked_value(
        &mut self,
        success_type: &ParameterType,
        value: &NirValue,
    ) -> Result<ValueResult, String> {
        let capture = self.label("raw_checked_done");
        let previous = self.raw_result_capture.take();
        self.raw_result_capture = Some(capture.clone());
        // A `Float` arithmetic node raises at plan-17's *observation boundary*,
        // not at the operator, and the boundary is wherever the value is first
        // consumed. Checking it here makes the `Checked` node that boundary: the
        // lifted `z * z` no longer flows into a `Bind`/argument that would have
        // observed it (it flows into `ResultValue`, which is not an arithmetic
        // node), so without this an overflow to infinity would be delivered as a
        // finite-looking `Ok` instead of running the handler.
        let lowered = self
            .lower_value(value)
            .and_then(|success| self.observe_float(value, &success).map(|()| success));
        self.raw_result_capture = previous;
        let success = lowered?;
        // Success fall-through: tag the produced value as the `Ok` result. A
        // `d`-native `Float` still lives in its FP register (plan-01), so its bit
        // pattern is moved across rather than pushed through a GP `mov` — the
        // same distinction `store_value_at` draws when spilling one.
        if Self::float_is_dnative(&success) {
            self.emit(abi::float_move_x_from_d(
                RESULT_VALUE_REGISTER,
                &success.location,
            ));
        } else {
            self.emit(abi::move_register(RESULT_VALUE_REGISTER, &success.location));
        }
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        self.emit(abi::label(&capture));
        self.materialize_current_result(success_type, "checked".to_string(), false)
    }

    /// Lower an inline conversion built-in (`toInt`/`toFloat`/`toFixed`/`toByte`)
    /// for an inline `TRAP`: emit the normal inline conversion but capture its
    /// error return (which would otherwise auto-propagate) so the raw `Result`
    /// is left in the standard registers, then materialize it as a value.
    fn lower_inline_conversion_raw(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let success_type = builtins::call_return_type(target)
            .ok_or_else(|| format!("native raw conversion '{target}' has no return type"))?;
        let capture = self.label("raw_conversion_done");
        let previous = self.raw_result_capture.take();
        self.raw_result_capture = Some(capture.clone());
        let lowered = match target {
            "toInt" => self.lower_to_int(args),
            "toFloat" => self.lower_to_float(&args[0]),
            "toFixed" => self.lower_to_fixed(&args[0]),
            "toByte" => self.lower_to_byte(&args[0]),
            "toMoney" => self.lower_to_money(&args[0]),
            "toScalar" => self.lower_to_scalar(&args[0]),
            other => Err(format!("native raw conversion '{other}' is not supported")),
        };
        self.raw_result_capture = previous;
        let success = lowered?;
        // Success fall-through: tag the converted value as the `Ok` result.
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &success.location));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        self.emit(abi::label(&capture));
        self.materialize_current_result(&success_type, format!("callResult {target}"), false)
    }

    /// Inline `TRAP` on a fallible inline member (plan-21-B): the member-agnostic
    /// generalization of `lower_inline_conversion_raw`. Run the member's normal
    /// inline lowering under a `raw_result_capture` so its domain-error exit
    /// branches to the capture point instead of propagating, then on the success
    /// fall-through tag the produced value `Ok` and materialize a `Result OF
    /// <success>`. Two failure seams reach the capture: the index/range members
    /// (`get`/`set`/`insert`/`removeAt`/`find`/`mid`) route their domain error
    /// through `emit_error_register_return`; the callback loop members
    /// (`transform`/`filter`/`reduce`/`forEach`, plan-26-B) route a failing user
    /// callback through `emit_callback_failure_exit` (which also frees each
    /// member's loop-scoped intermediates before joining the capture). The success
    /// value is a single register for every member except `forEach`, which yields
    /// `Nothing` (`void`) and takes the no-value fall-through below. Only the
    /// members `inline_builtin_raw_supported` allows reach here.
    ///
    /// The experimental `AbiInline` dual-path: if `target` names an `abi_inline`
    /// member, lower each `NirValue` arg to a `ValueResult` here (the dispatch owns
    /// arg acquisition — "pre-lowered `ValueResult`"), bundle an [`AbiCtx`] from the
    /// builder's own platform/imports/build-mode, and run the body inline.
    fn try_abi_inline_lower(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Option<Result<ValueResult, String>> {
        let lower = crate::codegen::registry::abi_inline_lower(target)?;
        let arg_values = match self.lower_abi_inline_args(args) {
            Ok(values) => values,
            Err(err) => return Some(Err(err)),
        };
        let ctx = self.inline_abi_ctx();
        // `-vv`: attribute inline-lowering time to the builtin being lowered.
        // A registry body is free to do arbitrary work per call site — build a
        // Unicode table, emit a kernel — and no span tree keyed by *stage* can
        // show that one builtin is responsible. Timed around the body only: arg
        // lowering above recurses into other builtins, and including it would
        // charge them to whichever call happened to enclose them.
        crate::trace::timed_tally(
            "abi_inline builtin",
            || target.to_string(),
            || Some(lower(self, &arg_values, &ctx)),
        )
    }

    /// Pre-lower each `NirValue` arg to a `ValueResult` for an `AbiInline` body
    /// ("pre-lowered `ValueResult`"). A single arg is consumed immediately and needs
    /// no stabilization. With two or more args, lowering a later arg can reset
    /// temporaries and clobber an earlier arg's register, so each is spilled to a
    /// fresh stack slot and, after the reset, reloaded into a persistent register —
    /// the exact spill/reset/reload the former `bits::gen_two_integers` did by hand,
    /// now owned by the dispatch so every multi-arg `AbiInline` body gets
    /// non-aliasing operands (and byte-identically to the pre-migration bodies).
    fn lower_abi_inline_args(&mut self, args: &[NirValue]) -> Result<Vec<ValueResult>, String> {
        if args.len() <= 1 {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(self.lower_value(arg)?);
            }
            return Ok(values);
        }
        // Spill each lowered arg to its own stack slot (arg0 spilled before arg1 is
        // lowered, so a temporary reset cannot clobber it).
        let mut spilled = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object(&format!("abi_inline_arg{index}"), 8);
            // Type-aware spill: a `d`-native `Float` stores from its FP register
            // (`str d`) so its bit pattern — including the sign of a `-0.0` — survives
            // the round-trip. A plain `store_u64` would push the FP vreg through a GP
            // store and drop the sign (bug: `pow(-0.0, 3)` → `+0.0`). Integer/GP args
            // store via `str x` exactly as before, so `bits`/collections stay identical.
            self.store_value_at(&value, abi::stack_pointer(), slot);
            spilled.push((slot, value.type_, value.text, value.origin));
        }
        self.reset_temporary_registers();
        // Allocate every result register first, then reload — matching the former
        // helper's instruction order.
        let mut registers = Vec::with_capacity(spilled.len());
        for _ in &spilled {
            registers.push(self.allocate_register());
        }
        let mut arg_values = Vec::with_capacity(spilled.len());
        for ((slot, type_, text, origin), register) in spilled.into_iter().zip(registers) {
            self.emit(abi::load_u64(&register, abi::stack_pointer(), slot));
            arg_values.push(ValueResult {
                origin,
                type_,
                location: Operand::from(register.render()),
                text,
            });
        }
        Ok(arg_values)
    }

    /// Mfb fast-path dual-path dispatch: for a `#collections_<name>$<TypeArgs>`
    /// monomorph target, consult the member's `Implementation::Mfb.fast_path`. If
    /// it lowers the call natively, return it; if it declines (`Ok(None)`), return
    /// `None` so the caller falls through to monomorphizing the `.mfb` body. The
    /// member name is parsed from the target and resolved to its qualified
    /// `collections.<name>` descriptor. Migrated members' per-instantiation gating
    /// lives in their `fast_path` fn, not here.
    fn try_mfb_fast_path(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Option<Result<ValueResult, String>> {
        let fast_path = crate::codegen::registry::mfb_fast_path(target)?;
        fast_path(self, target, args).transpose()
    }

    fn lower_inline_builtin_raw(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let capture = self.label("raw_builtin_done");
        let previous = self.raw_result_capture.take();
        self.raw_result_capture = Some(capture.clone());
        // The migrated variable-shift `bits.` ops (`sl`/`sr`/`sra`) route their
        // out-of-range `ErrInvalidArgument` through `emit_error_register_return`,
        // whose `raw_result_capture` branch (set above) redirects to the capture
        // point; the total `bits.` ops never reach here (they are infallible). Those
        // shift ops are `Body::abi_inline` intrinsics, reached through
        // `try_abi_inline_lower`, which runs inside this raw-capture wrapper so a
        // fallible body's domain error is captured rather than returned.
        let lowered = if let Some(result) = self.try_abi_inline_lower(target, args) {
            // An `abi_inline` member (the migrated collections/strings natives, e.g.
            // fallible `get`/`set`/`insert`, and the fallible `bits.sl`/`sr`/`sra`
            // variable shifts) trapped by an inline `TRAP`: its domain error routes
            // through the `raw_result_capture` branch set above.
            result
        } else if target == "toString" && (args.len() == 1 || args.len() == 2) {
            // bug-486: `toString(<List OF Byte>)`. Only the byte-list overload
            // reaches here (`inline_builtin_raw_supported` gates on the argument
            // type); `lower_to_string`'s `List OF Byte` arm raises `ErrEncoding`
            // through the same `emit_error_register_return` tail as the index
            // members above, so the `raw_result_capture` set here redirects it to
            // the capture point instead of auto-propagating past the handler.
            self.lower_to_string(args)
        } else {
            match crate::codegen::builtins::native_builtin_target(target) {
                Some("find") => self.lower_find(args),
                Some("mid") => self.lower_mid(args),
                other => Err(format!(
                    "native raw inline builtin '{target}' ({other:?}) is not supported"
                )),
            }
        };
        self.raw_result_capture = previous;
        let success = lowered?;
        // Success fall-through: tag the produced value as the `Ok` result.
        // `forEach` produces `Nothing` (a `void` location) — there is no value
        // register to carry, so set a benign 0 and materialize `Result OF Nothing`.
        if success.location == "void" {
            self.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
        } else {
            self.emit(abi::move_register(RESULT_VALUE_REGISTER, &success.location));
        }
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        self.emit(abi::label(&capture));
        let success_type = success.type_.clone();
        self.materialize_current_result(&success_type, format!("callResult {target}"), false)
    }

    /// Inline `TRAP` on a provably-infallible inline built-in (plan-26-A). Unlike
    /// [`lower_inline_builtin_raw`](Self::lower_inline_builtin_raw) there is no
    /// error exit to capture — the member cannot fail — so we lower it exactly as
    /// the normal call path would, then tag the single-register success `Ok` and
    /// materialize an always-`Ok` `Result OF <success>`. No capture label is
    /// needed (nothing ever branches to the error tail). The `TRAP`'s handler is
    /// therefore dead code, which the front-end flags with the advisory
    /// `TYPE_INLINE_TRAP_DEAD_HANDLER` warning.
    fn lower_inline_infallible_raw(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let success = self.lower_infallible_member(target, args)?;
        let success_type = success.type_.clone();
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &success.location));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        self.materialize_current_result(&success_type, format!("callResult {target}"), false)
    }

    /// Dispatch a provably-infallible inline built-in to its normal lowering. The
    /// enabled set matches `builtins::inline_builtin_is_infallible`: `len`,
    /// `toString`, `typeName`, every `bits::*` op, and the pure-query / growth /
    /// default-returning collection members. Each yields a single-register value,
    /// so one success shape covers them all. Shared only by
    /// [`lower_inline_infallible_raw`](Self::lower_inline_infallible_raw); the
    /// non-trapped call path keeps its own inline arms so its codegen is unchanged.
    fn lower_infallible_member(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        // plan-95/96: prefer a migrated member's own `abi_inline` lowering over the
        // infallible-inline ladder below (the third dispatch site).
        if let Some(result) = self.try_abi_inline_lower(target, args) {
            return result;
        }
        if target == "len" && args.len() == 1 {
            return self.lower_len(&args[0]);
        }
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
                location: Operand::from(register.render()),
                text: format!("typeName({type_name})"),
            });
        }
        // An infallible `abi_inline` intrinsic (the total `bits::*` ops — the
        // rotates `rl*`/`rr*`, `popCount`, `clz`/`ctz`, the byte swaps, and
        // `band`/`bor`/`bxor`/`bnot`) trapped by an inline TRAP: lower it inline
        // exactly as the non-trapped path would. It cannot fail, so — unlike the
        // fallible variable shifts handled by `lower_inline_builtin_raw` — there is
        // no error exit to capture. The fallible raw path grew a `try_abi_inline_lower`
        // arm when `bits` migrated onto `Body::abi_inline`; this infallible path was
        // overlooked, so an inline TRAP on a total `bits` op failed to lower.
        if let Some(result) = self.try_abi_inline_lower(target, args) {
            return result;
        }
        match crate::codegen::builtins::native_builtin_target(target) {
            Some("replace") if args.len() == 3 => self.lower_replace(args),
            other => Err(format!(
                "native infallible inline builtin '{target}' ({other:?}) is not supported"
            )),
        }
    }

    /// Lower a runtime-helper-backed call (`thread::*`, `fs::*`, `io::*`, …).
    /// With `raw = false` the call auto-unwraps/auto-propagates (the normal call
    /// site). With `raw = true` the call traps the raw `Result` for an inline
    /// `TRAP`: the helper outcome is materialized as a `Result OF <success>`
    /// value instead of propagating on error.
    /// `net::connectTcp` is overloaded on its first argument. The single-argument
    /// call is unambiguously the `Address` overload (the former source checker rejects a lone
    /// host); the two-argument call is the `Address` overload only when the first
    /// argument is statically an `Address` (otherwise it is `host, port`).
    fn net_connect_is_address_form(&self, args: &[NirValue]) -> bool {
        match args.len() {
            1 => true,
            2 => args
                .first()
                .and_then(|arg| self.overload_arg_type(arg))
                .is_some_and(|type_| type_.is_builtin_named("net", "Address")),
            _ => false,
        }
    }

    /// plan-76-A: `net::poll`'s first argument is a `List OF RES Socket` (the
    /// multiplex overload) rather than a scalar `Socket` (the readiness query). The
    /// two overloads share the `net.poll` NIR name; this selects the `net.pollList`
    /// helper by the receiver's static type.
    fn net_poll_is_list_form(&self, args: &[NirValue]) -> bool {
        args.first()
            .and_then(|arg| self.overload_arg_type(arg))
            .is_some_and(|ty| matches!(ty, crate::types::ParameterType::ListOf(_)))
    }

    /// bug-497: the code form of a bytes-or-text write (`tcp::write`,
    /// `udp::send`, `tls::write`) from its payload's static type — and nothing
    /// else. The two forms read the payload block with different layouts: a
    /// `String` is `[u64 len][bytes]`, a `List OF Byte` is a collection block
    /// whose count is at +8 and whose bytes start past the header. Choosing
    /// wrong is therefore not a wrong answer but an out-of-bounds read whose
    /// LENGTH the payload's own first bytes dictate; over a socket those bytes
    /// are the peer's, which is how OS-50 turned an echo server into a remote
    /// memory-disclosure oracle (a 22-byte request read back 1024 bytes of
    /// process memory).
    ///
    /// The old `is_some_and(String) … else bytes` shape failed OPEN: any payload
    /// the builder could not type took the byte form. This fails CLOSED: a
    /// `String` is text, a `List OF _` is bytes (the front end has already
    /// checked the element type, and the literal `[]` arrives as
    /// `List OF Unknown`), and anything else is a codegen error naming the
    /// member — a payload shape the resolver does not know is a build failure,
    /// never a guess. The byte-form sink checks the block header as well
    /// (`push_write_payload_view`), so the two guards are independent.
    fn net_write_payload_form(
        &self,
        payload: Option<&NirValue>,
        byte_form: &'static str,
        text_form: &'static str,
    ) -> Result<&'static str, String> {
        let payload = payload
            .ok_or_else(|| format!("native runtime {byte_form} is missing its payload argument"))?;
        match self.overload_arg_type(payload) {
            Some(ParameterType::String) => Ok(text_form),
            Some(ParameterType::ListOf(_)) => Ok(byte_form),
            other => Err(format!(
                "native runtime {byte_form}: payload static type {} is neither String nor \
                 List OF Byte; refusing to select a lowering (bug-497)",
                other.map_or_else(
                    || "<unresolved>".to_string(),
                    |type_| type_.name().to_string()
                )
            )),
        }
    }

    pub(crate) fn lower_runtime_helper_call(
        &mut self,
        helper: runtime::RuntimeHelper,
        target: &str,
        args: &[NirValue],
        raw: bool,
    ) -> Result<ValueResult, String> {
        let mut helper_args = args.to_vec();
        // plan-89-A: `io::print`/`io::write` accept an `AttributedString` and emit
        // its visible text. Rewrite the argument to `toString(a)` — the `toString`
        // overload deep-copies the inlined text into an owned String — so the rest
        // of the writer path is byte-identical to a String argument.
        if matches!(target, "io.print" | "io.write") {
            if let Some(arg) = helper_args.first() {
                if self
                    .overload_arg_type(arg)
                    .is_some_and(|type_| type_.is_named("AttributedString"))
                {
                    let inner = helper_args[0].clone();
                    helper_args[0] = NirValue::Call {
                        target: "toString".to_string(),
                        args: vec![inner],
                        loc: NirSourceLoc::default(),
                    };
                }
            }
        }
        if target == "io.input" && helper_args.is_empty() {
            helper_args.push(NirValue::Const {
                type_: ParameterType::String,
                value: String::new(),
            });
        } else if target == "io.pollInput" && helper_args.is_empty() {
            // plan-73-F: the no-timeout `io::pollInput()` form BLOCKS until input is
            // ready (the timeout convention's omit=unbounded rule) — pad the missing
            // `timeoutMs` with the block sentinel (i64::MIN); the poll helper routes
            // it to a poll(2) -1 (block-forever) timeout and rejects every other
            // negative value. Before plan-73 this padded `0` (a non-blocking check)
            // and a negative meant "block", the exact POSIX inversion the convention
            // removes.
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: TIMEOUT_UNBOUNDED_SENTINEL.to_string(),
            });
        } else if target == "thread.start" {
            while helper_args.len() < 4 {
                helper_args.push(NirValue::Const {
                    type_: ParameterType::Integer,
                    value: "64".to_string(),
                });
            }
        } else if matches!(target, "thread.send" | "thread.transferResource")
            && helper_args.len() == 2
        {
            // plan-73-A: the no-timeout `thread::send(t, x)` / `thread::transfer(t, r)`
            // form BLOCKS until the queue has space (the timeout convention's
            // omit=unbounded rule). Pad the missing `timeoutMs` with the block
            // sentinel (i64::MIN); the queue-write helper waits indefinitely on it and
            // rejects every other negative timeout. Before plan-73 `send` padded `0`
            // (immediate `ErrTimeout` when full) and `transferResource` was NOT padded
            // at all — its timeout arg was uninitialised (plan-73-A Corrections C2).
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: TIMEOUT_UNBOUNDED_SENTINEL.to_string(),
            });
        } else if matches!(target, "thread.receive" | "thread.acceptResource")
            && helper_args.len() == 1
        {
            // bug-181 / plan-73-A: the no-timeout `thread::receive(t)` /
            // `thread::accept(t)` overload blocks. Pad the missing `timeoutMs` with the
            // unbounded sentinel (i64::MIN); the queue-read helper waits indefinitely
            // on it and rejects every other negative timeout. (`accept` had no padding
            // before, so its no-arg blocking form is enabled here.)
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: TIMEOUT_UNBOUNDED_SENTINEL.to_string(),
            });
        } else if matches!(target, "thread.openStdIn" | "thread.closeStdIn")
            && helper_args.is_empty()
        {
            // No-arg self form: pass a null handle sentinel; the helper subscribes
            // the calling thread when the handle is 0 (plan-15 §4.5).
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: "0".to_string(),
            });
        } else if target == "net.lookup" && helper_args.len() == 1 {
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: "0".to_string(),
            });
        } else if target == "net.ping" {
            // plan-110-A: `ping(host|address, timeoutMs?, ttl?, size?)`. Both
            // overloads take the same three optional trailing arguments, so the
            // padding is shape-independent — unlike `connectTcp`, whose Address form
            // has a different positional layout. An omitted `timeoutMs` follows the
            // timeout convention (unbounded); `ttl`/`size` take the documented
            // defaults, which are public contract (plan-110-A §C3).
            const PING_DEFAULT_TTL: &str = "64";
            const PING_DEFAULT_SIZE: &str = "56";
            for value in [
                TIMEOUT_UNBOUNDED_SENTINEL,
                PING_DEFAULT_TTL,
                PING_DEFAULT_SIZE,
            ]
            .into_iter()
            .skip(helper_args.len().saturating_sub(1))
            {
                helper_args.push(NirValue::Const {
                    type_: ParameterType::Integer,
                    value: value.to_string(),
                });
            }
        } else if target == "tcp.connect" {
            // plan-110-B: the same padding `net.connectTcp` takes. The Address form
            // keeps its single record argument (2 total), the host/port form is 3;
            // the only ever-missing argument is the trailing `timeoutMs`, and an
            // omitted connect timeout BLOCKS per the convention.
            let is_address = self.net_connect_is_address_form(args);
            let target_args = if is_address { 2 } else { 3 };
            while helper_args.len() < target_args {
                helper_args.push(NirValue::Const {
                    type_: ParameterType::Integer,
                    value: TIMEOUT_UNBOUNDED_SENTINEL.to_string(),
                });
            }
        } else if target == "tcp.listen" && helper_args.len() == 2 {
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: "128".to_string(),
            });
        } else if target == "udp.poll" && helper_args.len() == 1 {
            // plan-110-C: an omitted readiness timeout blocks, as everywhere else.
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: TIMEOUT_UNBOUNDED_SENTINEL.to_string(),
            });
        } else if matches!(target, "tcp.poll" | "tcp.accept") && helper_args.len() == 1 {
            // An omitted readiness/accept timeout blocks, exactly as the `net`
            // originals do.
            helper_args.push(NirValue::Const {
                type_: ParameterType::Integer,
                value: TIMEOUT_UNBOUNDED_SENTINEL.to_string(),
            });
        }
        let result_type = self
            .thread_runtime_return_type(target, &helper_args)
            // plan-110-B: `tcp.poll` is return-type-overloaded the same way
            // `net.poll` is — a scalar socket answers `Boolean`, a list answers
            // with the first ready `Socket`.
            .or_else(|| {
                (target == "tcp.poll").then(|| {
                    if self.net_poll_is_list_form(&helper_args) {
                        ParameterType::named(crate::codegen::builtins::tcp::SOCKET_TYPE_ID)
                    } else {
                        ParameterType::Boolean
                    }
                })
            })
            // plan-110-C: `udp.poll` is return-type-overloaded the same way.
            .or_else(|| {
                (target == "udp.poll").then(|| {
                    if self.net_poll_is_list_form(&helper_args) {
                        ParameterType::named(crate::codegen::builtins::udp::SOCKET_TYPE_ID)
                    } else {
                        ParameterType::Boolean
                    }
                })
            })
            // plan-76-C: `tls.poll` is likewise return-type-overloaded — the list
            // form yields a borrowed `tls::Socket`, the scalar a `Boolean`.
            //
            // All four of these name the resource with its PACKAGE-QUALIFIED id,
            // not the bare `SOCKET_TYPE`. A bare resource spelling is invisible
            // to the resource classification, so an inline-TRAP'd list-form poll
            // treated the returned handle as an ordinary value and tried to
            // flat-copy it: "native inlined field size not available for type
            // 'Socket'", at build time, before plan-110-D. It is the same defect
            // the audio device-overload opens hit (see
            // `registry::alias_call_return_type`), reached by a different route.
            .or_else(|| {
                (target == "tls.poll").then(|| {
                    if self.net_poll_is_list_form(&helper_args) {
                        ParameterType::named(crate::codegen::builtins::tls::TLS_SOCKET_TYPE_ID)
                    } else {
                        ParameterType::Boolean
                    }
                })
            })
            .or_else(|| builtins::call_return_type(target))
            // A runtime-call `os_alias` reached directly (the IR-level overload
            // rewrites: `audio.openOutputDevice`/`openInputDevice`/…) is not a
            // registry member, so `call_return_type_name` declines it. Resolve the
            // aliased implementation's own return type — package-qualified, so a
            // resource return (`audio.AudioOutput`) keeps its resource
            // classification (the derived spec fallback below bares it, which
            // broke the inline-TRAP'd device-overload opens).
            .or_else(|| {
                crate::codegen::registry::alias_call_return_type(target)
                    .map(|name| ParameterType::declared(&name))
            })
            // A migrated package's code-form/scope-drop close op (`audio.closeInput`,
            // `audio.closeOutput`, `tls.closeListener`) is an `os_alias`, not a
            // registry member, so `call_return_type_name` declines it; its return type
            // is the catalogued runtime spec's (derived by `registry::runtime_specs`).
            .or_else(|| {
                runtime::spec_for_call(target).map(|spec| ParameterType::declared(spec.abi.returns))
            })
            .ok_or_else(|| format!("native runtime call '{target}' has no return type"))?;
        let runtime_target = match target {
            "thread.send" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.send missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.send handle has unknown type".to_string()
                    })?;
                if crate::types::is_worker_thread_handle(&handle) {
                    "thread.emit"
                } else {
                    "thread.send"
                }
            }
            "thread.receive" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.receive missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.receive handle has unknown type".to_string()
                    })?;
                if crate::types::is_worker_thread_handle(&handle) {
                    "thread.receive"
                } else {
                    "thread.read"
                }
            }
            // Resource plane, split by direction like the data plane above. A
            // `thread::transfer` (lowered to `transferResource`) on a worker handle
            // writes the outbound resource queue (`emitResource`); on a parent
            // handle it writes the inbound queue (`transferResource`). A
            // `thread::accept` (lowered to `acceptResource`) on a worker handle
            // reads the inbound queue (`acceptResource`); on a parent handle it
            // reads the outbound queue (`readResource`).
            "thread.transferResource" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.transfer missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.transfer handle has unknown type".to_string()
                    })?;
                if crate::types::is_worker_thread_handle(&handle) {
                    "thread.emitResource"
                } else {
                    "thread.transferResource"
                }
            }
            "thread.acceptResource" => {
                let handle = self
                    .static_type_name(helper_args.first().ok_or_else(|| {
                        "native runtime thread.accept missing handle argument".to_string()
                    })?)
                    .ok_or_else(|| {
                        "native runtime thread.accept handle has unknown type".to_string()
                    })?;
                if crate::types::is_worker_thread_handle(&handle) {
                    "thread.acceptResource"
                } else {
                    "thread.readResource"
                }
            }
            // plan-110-A: `ping(Address, …)` lowers through `net.pingAddr`, which
            // reads the host out of the record. Unlike `connectTcp`, the two ping
            // overloads share a positional layout, so the first argument's type
            // decides on its own at every arity.
            "net.ping" => {
                if args
                    .first()
                    .and_then(|arg| self.overload_arg_type(arg))
                    .is_some_and(|type_| type_.is_builtin_named("net", "Address"))
                {
                    "net.pingAddr"
                } else {
                    "net.ping"
                }
            }
            // plan-110-B: tcp's three overload-split code forms. `connect`/`poll`
            // split exactly as their `net` originals do; `write` is new — `net` had
            // separate `write`/`writeText` members, and collapsing them into one
            // overloaded member means the *lowering* has to be selected here, by
            // the payload's static type, instead of by the member name.
            "tcp.connect" => {
                if self.net_connect_is_address_form(args) {
                    "tcp.connectAddr"
                } else {
                    "tcp.connect"
                }
            }
            "tcp.poll" => {
                if self.net_poll_is_list_form(args) {
                    "tcp.pollList"
                } else {
                    "tcp.poll"
                }
            }
            "tcp.write" => {
                self.net_write_payload_form(args.get(1), "tcp.write", "tcp.writeText")?
            }
            // plan-110-C: udp's two code forms. `send`'s payload is argument 2
            // (socket, address, payload), not argument 1 as in `tcp::write`.
            "udp.poll" => {
                if self.net_poll_is_list_form(args) {
                    "udp.pollList"
                } else {
                    "udp.poll"
                }
            }
            "udp.send" => self.net_write_payload_form(args.get(2), "udp.send", "udp.sendText")?,
            // plan-110-D: the `net::Address` connect overload. Selected by the
            // first argument's static type rather than by arity, because tls's
            // optional `timeoutMs`/`serverName` are `DefaultValue::Fill` — every
            // call reaches here already padded, so the two shapes differ only in
            // whether the endpoint is one record or a host/port pair.
            "tls.connect" => {
                if args
                    .first()
                    .and_then(|arg| self.overload_arg_type(arg))
                    .is_some_and(|type_| type_.is_builtin_named("net", "Address"))
                {
                    "tls.connectAddr"
                } else {
                    "tls.connect"
                }
            }
            // plan-110-D: `tls::writeText` became a String overload of `tls::write`,
            // so the byte-vs-text lowering is selected here by the payload's type
            // rather than by the member name — the same move `tcp::write` made.
            "tls.write" => {
                self.net_write_payload_form(args.get(1), "tls.write", "tls.writeText")?
            }
            // bug-465: `tls::localAddress` spans both handle types, as
            // `tcp::localAddress` always has. The two cannot share one body —
            // macOS reads a `Socket`'s address off its connection path and a
            // `Listener`'s port off `nw_listener_get_port` — so the `Listener`
            // form routes to its own `tls.localAddressListener` code form.
            "tls.localAddress" => {
                if args
                    .first()
                    .and_then(|arg| self.overload_arg_type(arg))
                    .is_some_and(|type_| {
                        type_.is_named(crate::codegen::builtins::tls::TLS_LISTENER_TYPE_ID)
                    })
                {
                    crate::codegen::builtins::tls::LOCAL_ADDRESS_LISTENER
                } else {
                    "tls.localAddress"
                }
            }
            // plan-76-C: `tls::poll(List OF RES tls::Socket)` lowers through the portable
            // `tls.pollList` driver (scans the list via the scalar readiness helper),
            // vs the scalar `tls::poll(tls::Socket) → Boolean`.
            "tls.poll" => {
                if self.net_poll_is_list_form(args) {
                    "tls.pollList"
                } else {
                    "tls.poll"
                }
            }
            // plan-90-A: `spawn(args)` (argv only) and
            // `spawn(args, cwd, env, envReplace)` lower through distinct helpers;
            // the full form carries the working directory and environment map.
            "process.spawn" => {
                if args.len() >= 4 {
                    "process.spawnEnv"
                } else {
                    "process.spawn"
                }
            }
            // plan-90-B: the `timeoutMs` overloads route to distinct helpers that
            // poll for writability before each blocking write.
            "process.send" => {
                if args.len() >= 3 {
                    "process.sendTimeout"
                } else {
                    "process.send"
                }
            }
            "process.sendBytes" => {
                if args.len() >= 3 {
                    "process.sendBytesTimeout"
                } else {
                    "process.sendBytes"
                }
            }
            // plan-90-B: `poll(p, ms, from AS Stream)` routes to the stderr-capable
            // helper; the 2-arg form always polls stdout.
            "process.poll" => {
                if args.len() >= 3 {
                    "process.pollFrom"
                } else {
                    "process.poll"
                }
            }
            // `receive(p, from AS Stream)` / `receiveBytes(p, from)` route to the
            // stderr-capable helpers.
            "process.receive" => {
                if args.len() >= 2 {
                    "process.receiveFrom"
                } else {
                    "process.receive"
                }
            }
            "process.receiveBytes" => {
                if args.len() >= 2 {
                    "process.receiveBytesFrom"
                } else {
                    "process.receiveBytes"
                }
            }
            // audio's overload splits (named-device `open*`, timed `read`/`poll`,
            // per-direction `close`) are done at IR level (`audio::runtime_overload_name`),
            // so the NIR already carries the rewritten runtime-call name here.
            _ => target,
        };
        self.emit_runtime_helper_call(
            runtime_target,
            &runtime::symbol_for_call(helper, runtime_target),
            &helper_args,
            &result_type,
            raw,
        )
    }
}

/// Extract the source location carried by an error-originating NIR value, if any.
pub(crate) fn value_loc(value: &NirValue) -> Option<NirSourceLoc> {
    match value {
        NirValue::Call { loc, .. }
        | NirValue::CallResult { loc, .. }
        | NirValue::RuntimeCall { loc, .. }
        | NirValue::Binary { loc, .. }
        | NirValue::Unary { loc, .. } => Some(*loc),
        _ => None,
    }
}

#[cfg(test)]
mod concat_spine_tests {
    use super::*;
    use crate::target::shared::nir::NirSourceLoc;
    use crate::types::ParameterType;

    fn text(value: &str) -> NirValue {
        NirValue::Const {
            type_: ParameterType::String,
            value: value.to_string(),
        }
    }

    fn concat(left: NirValue, right: NirValue) -> NirValue {
        NirValue::Binary {
            op: BinaryOp::Concat,
            left: Box::new(left),
            right: Box::new(right),
            loc: NirSourceLoc::default(),
        }
    }

    fn spine(value: &NirValue) -> Vec<String> {
        let mut parts = Vec::new();
        flatten_concat_spine(value, &mut parts);
        parts
            .into_iter()
            .map(|part| match part {
                NirValue::Const { value, .. } => value.clone(),
                _ => "?".to_string(),
            })
            .collect()
    }

    /// `a & b & c` parses left-leaning, so the spine is `a`, `b` and the
    /// caller appends `c`.
    #[test]
    fn a_left_leaning_chain_flattens_in_source_order() {
        let chain = concat(concat(text("a"), text("b")), text("c"));
        let NirValue::Binary { left, .. } = &chain else {
            panic!("expected a concat")
        };
        assert_eq!(spine(left), vec!["a", "b"]);
    }

    /// A four-operand chain keeps going.
    #[test]
    fn a_longer_chain_keeps_its_order() {
        let chain = concat(concat(concat(text("a"), text("b")), text("c")), text("d"));
        let NirValue::Binary { left, .. } = &chain else {
            panic!("expected a concat")
        };
        assert_eq!(spine(left), vec!["a", "b", "c"]);
    }

    /// The spine stops at a non-`&` value: a parenthesized right subchain
    /// keeps its own grouping and is fused as its own chain.
    #[test]
    fn the_spine_stops_at_a_non_concat() {
        let inner = concat(text("b"), text("c"));
        assert_eq!(spine(&inner), vec!["b", "c"]);
        let other = NirValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(text("a")),
            right: Box::new(text("b")),
            loc: NirSourceLoc::default(),
        };
        assert_eq!(spine(&other), vec!["?"]);
    }

    /// A lone operand is a one-element spine, which the caller's arity check
    /// then sends down the pairwise path.
    #[test]
    fn a_lone_operand_is_its_own_spine() {
        assert_eq!(spine(&text("a")), vec!["a"]);
    }
}

#[cfg(test)]
mod borrowed_resource_tests {
    use super::*;
    use crate::codegen::engine::builder::CodeBuilder;
    use crate::target::shared::nir::NirSourceLoc;
    use crate::target::shared::runtime::RuntimeHelper;

    fn runtime_call(target: &str) -> NirValue {
        NirValue::RuntimeCall {
            helper: RuntimeHelper::Tcp,
            target: target.to_string(),
            args: Vec::new(),
            loc: NirSourceLoc::default(),
        }
    }

    fn plain_call(target: &str) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args: Vec::new(),
            loc: NirSourceLoc::default(),
        }
    }

    /// The list form of `poll` returns a pointer to an element the list still
    /// owns, so its `RES` bind must register no close (bug-375).
    ///
    /// The guard is on the **NIR variant**, not just the name: a built-in
    /// package's member lowers to `NirValue::RuntimeCall`, and this predicate
    /// used to match only `Call`/`CallResult`. The `net.poll`/`tls.poll` names it
    /// listed were therefore never reached, and every `poll(List)` bind was
    /// classified as an owner -- closing the borrowed element at the bind's scope
    /// exit and again when the list drained.

    /// plan-114-C: `RES g = h.handle` reads a handle back OUT of a record field.
    /// The record's scope owns it, so the bind registers no close of its own —
    /// the same rule bug-375 established for `poll(List)`, one container-kind
    /// over. Registering a close here would release the record's handle at this
    /// scope's exit and the record's drain would then close it again.
    #[test]
    fn a_record_field_read_aliases_a_live_resource() {
        let read = NirValue::MemberAccess {
            target: Box::new(NirValue::Local("h".to_string())),
            member: "handle".to_string(),
        };
        assert!(
            CodeBuilder::value_aliases_live_resource(&read),
            "a record field read is an alias, not a producer"
        );
    }

    #[test]
    fn poll_list_forms_alias_a_live_resource() {
        for target in ["tcp.poll", "udp.poll", "tls.poll"] {
            assert!(
                CodeBuilder::value_aliases_live_resource(&runtime_call(target)),
                "{target} returns a borrowed list element"
            );
        }
        // A producing member of the same package still transfers ownership.
        assert!(!CodeBuilder::value_aliases_live_resource(&runtime_call(
            "tcp.accept"
        )));
        // `collections::get`/`getOr` keep their own borrow classification, which
        // arrives as a plain call rather than a runtime call.
        assert!(CodeBuilder::value_aliases_live_resource(&plain_call(
            "collections.get"
        )));
    }
}
