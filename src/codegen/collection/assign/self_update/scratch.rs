//! plan-176-Q: self-update scratch — which functions need a scratch block, `SCRATCH_ARMS`, and
//! reserving and publishing the scratch. The arm table lives in
//! `collection/assign/self_update/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// The self-update scratch (plan-142-B Correction B1).
// ---------------------------------------------------------------------------

/// Builtins whose in-place arm keeps per-element state in the function's
/// self-update scratch. A function holding a self-update of one of these gets the
/// scratch slot (`prescan_self_update_scratch`).
pub(crate) const SCRATCH_ARMS: &[&str] = &[
    "filter",
    "distinct",
    "transform",
    "sort",
    "sortBy",
    "union",
    "intersection",
    "difference",
    "symmetricDifference",
    "mapValues",
    "merge",
];

/// Whether `ops` (recursively) hold a self-update `x = f(x, …)` whose call target
/// satisfies `wanted`.
fn ops_hold_self_update(ops: &[NirOp], wanted: &dyn Fn(&str) -> bool) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Assign { name, value } => {
            if with_holds_field_self_update(value, wanted) {
                return true;
            }
            let Some((target, args)) = self_update_call_parts(value) else {
                return false;
            };
            matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name) && wanted(target)
        }
        NirOp::StoreGlobal {
            name,
            value: Some(value),
            ..
        } if self_update_call_parts(value).is_some() => {
            let (target, args) = self_update_call_parts(value).expect("matched above");
            matches!(args.first(), Some(NirValue::Global { name: arg0, .. }) if arg0 == name)
                && wanted(target)
        }
        // plan-145-G: `gR = WITH gR { f := g(gR.f, …) }`.
        NirOp::StoreGlobal {
            value: Some(value), ..
        } => with_holds_field_self_update(value, wanted),
        // plan-145-D: a field self-update `r = WITH r { f := g(r.f, …) }` or
        // `h.state = WITH h.state { f := g(h.state.f, …) }`, one field or mixed.
        NirOp::StateAssign { value, .. } => with_holds_field_self_update(value, wanted),
        // plan-147-B (site S11): `RETURN f(x, …)` on an owned local runs the same
        // arm as `x = f(x, …)`, so a function whose only self-update is at a
        // `RETURN` must still reserve the scratch that arm needs. Without this the
        // arm would find none.
        NirOp::Return { value: Some(value) } => {
            returned_self_update_local(value).is_some_and(|_| {
                self_update_call_parts(value).is_some_and(|(target, _)| wanted(target))
            })
                // plan-147-F: the field form of S11 runs the same arms, so a function
                // whose only self-update is `RETURN WITH r { f := OP(r.f, …) }` must
                // reserve the scratch those arms need.
                || with_holds_field_self_update(value, wanted)
        }
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_hold_self_update(then_body, wanted) || ops_hold_self_update(else_body, wanted),
        NirOp::Match { cases, .. } => cases
            .iter()
            .any(|case| ops_hold_self_update(&case.body, wanted)),
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => ops_hold_self_update(body, wanted),
        _ => false,
    })
}

/// Whether `value` is a `WITH` over its owner (`r` or `h.state`) with a field
/// update `f := g(<owner>.f, …)` whose `g` is `wanted` — a field self-update.
/// The owner is the `WITH` target itself; `ops_hold_self_update`'s caller
/// matched the assignment's left side to it.
pub(super) fn with_holds_field_self_update(
    value: &NirValue,
    wanted: &dyn Fn(&str) -> bool,
) -> bool {
    let NirValue::WithUpdate {
        target: owner,
        updates,
        ..
    } = value
    else {
        return false;
    };
    updates.iter().any(|update| match &update.value {
        NirValue::Call { target, args, .. } => {
            matches!(
                args.first(),
                Some(NirValue::MemberAccess { target: inner, member })
                    if *member == update.field && same_field_owner(inner, owner)
            ) && wanted(target)
        }
        // plan-145-F: `inner := WITH <owner>.inner { f := g(<owner>.inner.f, …) }`.
        NirValue::WithUpdate { target: nested, .. } => {
            matches!(
                nested.as_ref(),
                NirValue::MemberAccess { target: inner, member }
                    if *member == update.field && same_field_owner(inner, owner)
            ) && with_holds_field_self_update(&update.value, wanted)
        }
        _ => false,
    })
}

/// Whether two field owners name the same place: the local `r`, `h.state`, or
/// (plan-145-F) a field path under either, `o.inner`.
fn same_field_owner(a: &NirValue, b: &NirValue) -> bool {
    match (a, b) {
        (NirValue::Local(x), NirValue::Local(y)) => x == y,
        // plan-145-G: a module-level record.
        (NirValue::Global { name: x, .. }, NirValue::Global { name: y, .. }) => x == y,
        (
            NirValue::MemberAccess {
                target: x,
                member: mx,
            },
            NirValue::MemberAccess {
                target: y,
                member: my,
            },
        ) => mx == my && same_field_owner(x, y),
        _ => false,
    }
}

/// Whether `ops` create a closure whose lambda borrows its creator's scratch
/// (`scratch_closure_captures`).
fn ops_create_scratch_closure(builder: &CodeBuilder<'_>, ops: &[NirOp]) -> bool {
    struct Finder<'b, 'a> {
        builder: &'b CodeBuilder<'a>,
        found: bool,
    }
    impl NirVisitor for Finder<'_, '_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Closure { name, captures, .. } = value {
                if !captures.is_empty() && self.builder.function_needs_scratch(name) {
                    self.found = true;
                }
            }
            walk_value(self, value);
        }
    }
    let mut finder = Finder {
        builder,
        found: false,
    };
    finder.visit_ops(ops);
    finder.found
}

/// bug-767 A: whether `ops` hold a copying `collections::sort`/`sortBy` call that
/// the word-merge fast paths decline — the shapes `lower_copying_sort{,_by}`
/// serve by sorting a copy with the in-place arm's merge, which keeps its index
/// buffers in the self-update scratch. Only this function's own scratch: the
/// predicate is not part of `function_needs_scratch`, so no closure borrows a
/// creator's scratch for it (a `keyFn` cannot hold a self-update — only a
/// `forEach` lambda mutates a capture — so the merge never shares the block).
fn ops_hold_copying_sort(ops: &[NirOp]) -> bool {
    struct Finder {
        found: bool,
    }
    impl NirVisitor for Finder {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } =
                value
            {
                let copying_sort = (target.starts_with("#collections_sortBy$")
                    && args.len() == 2
                    && !sort_by_word_path_accepts(target, args))
                    || (target.starts_with("#collections_sort$")
                        && args.len() == 1
                        && !sort_word_path_accepts(target, args));
                if copying_sort {
                    self.found = true;
                }
            }
            walk_value(self, value);
        }
    }
    let mut finder = Finder { found: false };
    finder.visit_ops(ops);
    finder.found
}

/// Whether a self-update of `target` has an arm that keeps state in the
/// function's self-update scratch.
fn target_needs_self_update_scratch(target: &str) -> bool {
    self_update_builtin(target).is_some_and(|bare| SCRATCH_ARMS.contains(&bare))
        // plan-146-D/E: the `String` arms that build their result in the scratch
        // (`os::appResourcePath`'s prefix, the six rewrites), by qualified target.
        || crate::codegen::collection::assign::string_self_update::target_needs_string_scratch(
            target,
        )
        || crate::codegen::collection::assign::builder_inplace_rewrite::math_self_update_function(
            target,
        )
        .is_some()
}

/// plan-146-D: whether the module holds a self-update `x = f(x, …)` of the builtin
/// whose bare name is `bare`.
pub(crate) fn module_self_updates_with(module: &NirModule, bare: &str) -> bool {
    module.functions.iter().any(|function| {
        ops_hold_self_update(&function.body, &|target| {
            self_update_builtin(target) == Some(bare)
        })
    })
}

/// plan-146-D/G: whether `ops` hold `s = os::appResourcePath(s)`.
pub(crate) fn ops_hold_resource_path_self_update(ops: &[NirOp]) -> bool {
    ops_hold_self_update(ops, &|target| {
        self_update_builtin(target) == Some("appResourcePath")
    })
}

/// Whether the module holds a `collections::replace` self-update (plan-142-C).
/// Its arm writes through `lower_list_set_in_place`, whose rebuild path — never
/// taken from that arm, but always emitted — raises `ErrIndexOutOfRange`, so the
/// module needs that message's data object even when it calls no bounds-checked
/// member (`data_objects::string_symbols`).
pub(crate) fn module_self_updates_with_replace(module: &NirModule) -> bool {
    module.functions.iter().any(|function| {
        ops_hold_self_update(&function.body, &|target| {
            self_update_builtin(target) == Some("replace")
        })
    })
}

impl CodeBuilder<'_> {
    /// Whether the module function `name`'s body holds a self-update whose arm
    /// needs the scratch.
    pub(crate) fn function_needs_scratch(&self, name: &str) -> bool {
        self.functions.get(name).is_some_and(|function| {
            ops_hold_self_update(&function.body, &target_needs_self_update_scratch)
        })
    }

    /// plan-142-G: a lambda holding a scratch self-update (the self-update of a
    /// by-ref capture, site S9) runs once per element of the `forEach` that calls
    /// it, so a scratch of its own would be allocated once per element. It
    /// borrows its creator's instead: the closure env carries one word past its
    /// captures — the address of the creator's scratch slot. `Some(index of that
    /// word)` for such a lambda, found from the `Closure` node that creates it.
    pub(crate) fn scratch_closure_captures(&self, lambda: &str) -> Option<usize> {
        if !self.function_needs_scratch(lambda) {
            return None;
        }
        struct Finder<'n> {
            lambda: &'n str,
            captures: Option<usize>,
        }
        impl NirVisitor for Finder<'_> {
            fn visit_value(&mut self, value: &NirValue) {
                if let NirValue::Closure { name, captures, .. } = value {
                    if name == self.lambda && !captures.is_empty() {
                        self.captures = Some(captures.len());
                    }
                }
                walk_value(self, value);
            }
        }
        let mut finder = Finder {
            lambda,
            captures: None,
        };
        for function in self.functions.values() {
            finder.visit_ops(&function.body);
        }
        finder.captures
    }

    /// Give this function a self-update scratch slot if its body holds a
    /// self-update whose arm needs one, or creates a closure that borrows it
    /// (`scratch_closure_captures`). The slot is registered as a function-level
    /// owned `List OF Integer`, so the ordinary scope drop frees it on every exit
    /// (with the null guard and prologue zeroing that drop brings). A lambda that
    /// borrows its creator's scratch gets a working slot and no cleanup. A
    /// function without such a statement is untouched.
    pub(crate) fn prescan_self_update_scratch(&mut self, function: &str, ops: &[NirOp]) {
        if self.self_update_scratch.is_some() {
            return;
        }
        if let Some(index) = self.scratch_closure_captures(function) {
            self.self_update_scratch = Some(self.allocate_stack_object("su_scratch", 8));
            self.self_update_scratch_env = Some(index);
            return;
        }
        if !ops_hold_self_update(ops, &target_needs_self_update_scratch)
            && !ops_create_scratch_closure(self, ops)
            && !ops_hold_copying_sort(ops)
        {
            return;
        }
        let slot = self.allocate_stack_object("su_scratch", 8);
        self.self_update_scratch = Some(slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: ParameterType::list_of(ParameterType::Integer),
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
    }

    /// plan-146-D: give this function a slot for `os::appResourcePath`'s base block
    /// when its body holds `s = os::appResourcePath(s)` — the in-place arm computes the
    /// base at most once per call and reads it from there. Registered as a
    /// function-level owned `String`, so the ordinary scope drop frees it (with the
    /// null guard and prologue zeroing that drop brings), exactly as the self-update
    /// scratch is.
    pub(crate) fn prescan_string_app_resource_base(&mut self, ops: &[NirOp]) {
        if self.string_app_resource_base.is_some() || self.string_app_resource_base_env.is_some() {
            return;
        }
        // plan-146-G: a function that lends the cache to a lambda needs one too.
        let lends = self.closures_created_in(ops).into_iter().any(|lambda| {
            crate::codegen::collection::assign::string_self_update::lambda_shares_resource_base(
                self.functions,
                &lambda,
            )
        });
        if !ops_hold_resource_path_self_update(ops) && !lends {
            return;
        }
        let slot = self.allocate_stack_object("str_resource_base", 8);
        self.string_app_resource_base = Some(slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: ParameterType::String,
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
        // The prologue zeroes it from here: without that, a second call to this
        // function reads the block the first one cached AND freed (a lambda runs
        // once per element, and the third call died on the reused memory).
        self.owned_value_slots.push(slot);
    }

    /// plan-146-G: in a lambda, the closure-environment word holding the address
    /// of the creator's `os::appResourcePath` base-path cache, after the shadow words.
    pub(crate) fn prescan_string_app_resource_base_env(&mut self, function: &str) {
        if !crate::codegen::collection::assign::string_self_update::lambda_shares_resource_base(
            self.functions,
            function,
        ) {
            return;
        }
        let Some(total) = self.closure_capture_count(function) else {
            return;
        };
        let scratch_words = usize::from(self.scratch_closure_captures(function).is_some());
        let shadow_words =
            crate::codegen::collection::assign::string_self_update::string_shadow_captures(
                self.functions,
                function,
            )
            .len();
        self.string_app_resource_base_env = Some(total + scratch_words + shadow_words);
    }

    /// plan-146-G: in a lambda, record which closure-environment word holds the
    /// address of each by-ref-captured `String`'s OWNER shadow slot. The layout is
    /// `[captures…][the borrowed scratch, if any][one word per shadowed capture]`,
    /// and the creator writes the same words in the same order
    /// (`string_shadow_captures`).
    pub(crate) fn prescan_string_shadow_env(&mut self, function: &str) {
        let captures =
            crate::codegen::collection::assign::string_self_update::string_shadow_captures(
                self.functions,
                function,
            );
        if captures.is_empty() {
            return;
        }
        let Some(total) = self.closure_capture_count(function) else {
            return;
        };
        // The same predicate the creator uses to decide whether it lends its
        // scratch (`borrowed_scratch`), NOT `self_update_scratch_env` — that field
        // is set by a later prescan, and reading it here put the shadow word on top
        // of the scratch word (both sides then wrote through word 1: the scratch
        // pointer landed in the owner's shadow and the scratch leaked).
        let scratch_words = usize::from(self.scratch_closure_captures(function).is_some());
        for (position, (local, _)) in captures.into_iter().enumerate() {
            self.string_shadow_env
                .insert(local, total + scratch_words + position);
        }
    }

    /// The number of values the closure creating `lambda` captures, or `None` when
    /// no closure does (or it captures nothing, so there is no environment).
    pub(crate) fn closure_capture_count(&self, lambda: &str) -> Option<usize> {
        struct Finder<'n> {
            lambda: &'n str,
            captures: Option<usize>,
        }
        impl NirVisitor for Finder<'_> {
            fn visit_value(&mut self, value: &NirValue) {
                if let NirValue::Closure { name, captures, .. } = value {
                    if name == self.lambda && !captures.is_empty() {
                        self.captures = Some(captures.len());
                    }
                }
                walk_value(self, value);
            }
        }
        let mut finder = Finder {
            lambda,
            captures: None,
        };
        for function in self.functions.values() {
            finder.visit_ops(&function.body);
        }
        finder.captures
    }

    /// Make the self-update scratch hold at least the byte count in `need_slot`,
    /// and return a fresh frame slot holding the address of its first byte. The
    /// bytes' contents are unspecified. Grows to twice the request (at least 64
    /// bytes), so a loop over similar sizes allocates it once.
    ///
    /// `Err` when the function has no scratch slot: an arm that needs one must
    /// check `self_update_scratch` in its gates, before it emits anything.
    pub(crate) fn emit_reserve_self_update_scratch(
        &mut self,
        need_slot: usize,
    ) -> Result<usize, String> {
        let scratch_slot = self
            .self_update_scratch
            .ok_or("native self-update scratch requested in a function without one")?;
        self.emit_reserve_scratch_block(scratch_slot, self.self_update_scratch_env, need_slot)
    }

    /// [`Self::emit_reserve_self_update_scratch`] over the scratch block whose
    /// frame slot is `scratch_slot` — borrowed through closure-env word `env`
    /// when that is `Some` — shared with bug-766 B's record-temp scratch.
    pub(super) fn emit_reserve_scratch_block(
        &mut self,
        scratch_slot: usize,
        env: Option<usize>,
        need_slot: usize,
    ) -> Result<usize, String> {
        let list_type = ParameterType::list_of(ParameterType::Integer);
        let layout = CollectionTypeLayout::from_type(&list_type)
            .ok_or("native self-update scratch has no List OF Integer layout")?;
        let block = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let need = self.temporary_vreg();
        let size = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let newcap_slot = self.allocate_stack_object("su_scratch_newcap", 8);
        let data_slot = self.allocate_stack_object("su_scratch_data", 8);
        let grow = self.label("su_scratch_grow");
        let no_free = self.label("su_scratch_no_free");
        let alloc_ok = self.label("su_scratch_alloc_ok");
        let ready = self.label("su_scratch_ready");

        // A borrowed scratch: take the creator's current block as the working copy.
        if let Some(index) = env {
            let holder = self.temporary_vreg();
            self.emit(abi::load_u64(
                &holder,
                CLOSURE_ENV_REGISTER,
                closure_env_word_offset(index),
            ));
            self.emit(abi::load_u64(&block, &holder, 0));
            self.emit(abi::store_u64(&block, abi::stack_pointer(), scratch_slot));
        }
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&grow));
        self.emit(abi::load_u64(&cap, &block, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
        self.emit(abi::compare_registers(&need, &cap));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::branch(&ready));

        // newCapacity = align8(2 * need + 64).
        self.emit(abi::label(&grow));
        self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&size, &need, &need));
        self.emit(abi::add_immediate(&size, &size, 64 + 7));
        self.emit(abi::move_immediate(&mask, "Integer", &(!7u64).to_string()));
        self.emit(abi::and_registers(&size, &size, &mask));
        self.emit(abi::store_u64(&size, abi::stack_pointer(), newcap_slot));
        // Free the old block (its contents are scratch), sized as the drop sizes
        // it: HEADER + dataCapacity (a `List OF Integer` has no entry table).
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&no_free));
        self.emit(abi::load_u64(&cap, &block, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::add_immediate(
            abi::c_arg(1),
            &cap,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::move_register(abi::c_arg(0), &block));
        self.emit_arena_free_call();
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            scratch_slot,
        ));
        // The creator must never see the freed block, even if the alloc below
        // raises: its drop would free it again.
        self.emit_publish_borrowed_scratch(scratch_slot, env);
        self.emit(abi::label(&no_free));
        let size = self.temporary_vreg();
        self.emit(abi::load_u64(&size, abi::stack_pointer(), newcap_slot));
        self.emit(abi::add_immediate(
            abi::c_arg(0),
            &size,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            scratch_slot,
        ));
        let nb = self.temporary_vreg();
        let zero = self.temporary_vreg();
        let count_cap = self.temporary_vreg();
        let dcap = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), scratch_slot));
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::load_u64(&dcap, abi::stack_pointer(), newcap_slot));
        self.emit(abi::shift_right_immediate(&count_cap, &dcap, 3));
        self.emit_write_collection_header_full(&layout, &nb, &zero, &count_cap, &zero, &dcap);
        self.emit_publish_borrowed_scratch(scratch_slot, env);

        self.emit(abi::label(&ready));
        let block = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::add_immediate(&block, &block, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), data_slot));
        Ok(data_slot)
    }

    /// In a lambda borrowing its creator's scratch, store the working copy in
    /// `scratch_slot` back into the creator's slot. Nothing otherwise.
    fn emit_publish_borrowed_scratch(&mut self, scratch_slot: usize, env: Option<usize>) {
        let Some(index) = env else {
            return;
        };
        let block = self.temporary_vreg();
        let holder = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::load_u64(
            &holder,
            CLOSURE_ENV_REGISTER,
            closure_env_word_offset(index),
        ));
        self.emit(abi::store_u64(&block, &holder, 0));
    }
}
