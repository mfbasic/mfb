//! plan-176-U: value ownership — owned-param binds, `lower_value_owned`, owning-copy and string
//! provenance, runtime-result ownership, resource aliasing, graph copies, stored values,
//! shallow temps and the closure environment layout. The entry points and shared types live in
//! `engine/value/builder_values/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
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
    /// plan-147-E: whether the bind being lowered moves an owned parameter rather
    /// than copying it (`handover`'s `param_binds`, computed once per function).
    ///
    /// Answers `false` in a synthesized builder, at an op the analysis never saw, and
    /// for anything but a bare `Local` — the fail-closed direction, where the copy
    /// stands.
    fn owned_param_bind_moves(&self, value: &NirValue) -> bool {
        if !matches!(value, NirValue::Local(_)) {
            return false;
        }
        let Some(op) = self.current_op_key else {
            return false;
        };
        self.handover
            .as_ref()
            .is_some_and(|handover| handover.moves_owned_param(op))
    }

    /// Retire `name`'s owned-value cleanup and null its slot: the block now belongs
    /// to the binding being lowered.
    fn release_owned_param_to_binding(&mut self, name: &str) {
        let Some(local) = self.locals.get(name) else {
            return;
        };
        let stack_offset = local.stack_offset;
        if let Some(index) = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::OwnedValue(c) if c.stack_offset == stack_offset)
        }) {
            self.active_cleanups.remove(index);
        }
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            stack_offset,
        ));
    }

    pub(crate) fn lower_value_owned(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let result = self.lower_value(value)?;
        // bug-778: a reader of an unboxed inline-`TRAP` temporary hands over a
        // block the temporary owned and has just given up (its slot is nulled) —
        // this frame's own block, with no other owner. Copying it would be the
        // copy-out the heap `Result` forced; taking it is the move.
        if self.unboxed_trap_read_moves(value) {
            return Ok(result);
        }
        // bug-778: the desugar's delivery `Bind y = $trap_valN` is that slot's one
        // read (`trap_delivery_moves`), so the store takes the block and nulls the
        // slot — whose drop then skips it — instead of copying it.
        if let NirValue::Local(name) = value {
            if self.trap_delivery_moves.contains(name)
                && self.is_freeable_flat_value(&result.type_)
                && !self.owns_graph(&result.type_)
            {
                if let Some(local) = self.locals.get(name).filter(|local| !local.by_ref) {
                    let slot = local.stack_offset;
                    self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
                    return Ok(result);
                }
            }
        }
        // A register-native vector reaching an owner boundary (a binding, global,
        // return, closure env) materializes to its block here — the block is
        // registered as a temp by `vector_value_as_block` and claimed just below,
        // so the owner owns it exactly as it would an eager `Constructor` block.
        if Self::is_vector_native(&result) {
            let block = self.vector_value_as_block(result)?;
            self.claim_pending_temp(&block);
            return Ok(block);
        }
        // plan-147-E: `MUT y = p` where `p` is an owned PARAMETER at its last use hands
        // the block to the new binding instead of copying it. That is the
        // `__json_parseArrayItems` shape (`MUT acc = items`, then a loop of in-place
        // appends), where the entry copy is the whole cost.
        //
        // The transfer is the same one `plan_returned_move` makes: retire the
        // parameter's own `OwnedValue` cleanup and null its slot, so the block has
        // exactly one owner — the new binding, whose own cleanup the bind registers.
        // A null slot is "moved-from, not dropped" (§14.7), which every drop already
        // handles.
        if self.owned_param_bind_moves(value) {
            if let NirValue::Local(name) = value {
                let name = name.clone();
                self.release_owned_param_to_binding(&name);
                return Ok(result);
            }
        }
        if self.value_needs_owning_copy(value)
            && self.is_freeable_flat_value(&result.type_)
            && !Self::is_fresh_trapped_result_wrapper(value, &result.type_)
        {
            let copied = self.copy_flat_block(&result.type_, &result.location)?;
            return Ok(ValueResult {
                origin: None,
                type_: result.type_,
                location: Operand::from(copied),
                text: result.text,
            });
        }
        // plan-134-D: a recursive value is a graph `copy_flat_block` cannot copy, so an
        // aliasing source gets an independent graph from the walker — unless this store is
        // the source's last read (plan-134-C), when handing the same graph over is a move
        // and nothing else can observe it.
        if self.value_needs_owning_copy(value)
            && self.needs_graph_copy(&result.type_)
            && !Self::is_fresh_trapped_result_wrapper(value, &result.type_)
            && !self.store_is_last_use(value)
            && !self.store_is_borrowed_view()
        {
            let copied = self.copy_value_to_current_arena(&result.type_, &result.location)?;
            return Ok(ValueResult {
                origin: None,
                type_: result.type_,
                location: Operand::from(copied),
                text: result.text,
            });
        }
        // plan-134-G: the store took the graph at the source's last read, so the source
        // gives it up.
        if self.value_needs_owning_copy(value)
            && self.owns_graph(&result.type_)
            && !Self::is_fresh_trapped_result_wrapper(value, &result.type_)
            && !self.store_is_borrowed_view()
            && self.store_is_last_use(value)
        {
            self.release_moved_source(value, &result, false)?;
            return Ok(result);
        }
        // A fresh value returned unchanged becomes this owner's block; claim its
        // pending-temp registration so the statement-scope free never double-frees
        // what scope-drop (or the consuming store) now owns (plan-25).
        self.claim_pending_temp(&result);
        Ok(result)
    }

    /// bug-568: whether `result_type` is the fresh `Result` WRAPPER an inline
    /// `TRAP`'s lowering built, rather than the callee's own returned value.
    ///
    /// The three predicates [`Self::value_needs_owning_copy`] consults for a call
    /// — `call_returns_param_borrow`, `call_returns_rodata_string`,
    /// `static_string_value` — are all statements about the block the CALLEE
    /// hands back. On a `NirValue::CallResult` that block is not what lowering
    /// produced: every one of the three inline-`TRAP` lowerings ends at
    /// [`CodeBuilder::fresh_trapped_result_value`] holding a `{tag, size,
    /// payload}` block this frame's own `_mfb_arena_alloc` returned, with the
    /// callee's value COPIED into it (that copy, and the free of what it copied
    /// FROM, are bug-561's fix on the Ok branch and bug-565's on the error
    /// branch). Asking a callee-keyed question about it answers about the wrong
    /// block.
    ///
    /// Answering `true` there made `LET n AS Integer = risky(i) TRAP …` deep-copy
    /// the whole `Result` and abandon the original — 134 B on every call, on a
    /// path where the producer never fails, because `risky`'s body is
    /// `RETURN i` and `i` is a parameter. 26.8 MB at 200 000 iterations and
    /// 52.6 MB at 400 000, with the same loop over an infallible callee flat at
    /// 1.06 MB. A `String` payload cost 195 B (38.2 → 75.4 MB).
    ///
    /// **Both halves of the conjunction are load-bearing, and it fails CLOSED.**
    /// The `CallResult` node says the lowering went through a trapped-`Result`
    /// path; the `ResultOf` type is the witness that it actually produced a
    /// wrapper, since `fresh_trapped_result_value` is the only constructor of one
    /// (asserted by `codegen_trap_result_wrapper.rs`). If a future `CallResult`
    /// lowering ever returned something else, the type check fails and the copy
    /// is kept — the old, merely wasteful behaviour — rather than handing a
    /// binding an alias it would then `arena_free`.
    ///
    /// The plain-`Call` path is untouched and MUST be: there the lowered value IS
    /// the callee's block, so a param-borrow really does need the copy, and
    /// removing it would be a use-after-free of the caller's own argument
    /// (`a_param_borrow_without_a_trap_still_copies` pins it).
    fn is_fresh_trapped_result_wrapper(value: &NirValue, result_type: &ParameterType) -> bool {
        matches!(value, NirValue::CallResult { .. })
            && matches!(result_type, ParameterType::ResultOf(_))
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

    /// bug-536 shape B-2: whether `value` is a direct call to a user /
    /// `.mfb`-bodied function that guarantees a **fresh** bare `String` result
    /// (`function_returns_fresh_string`) — the one way a `String` call result with
    /// no native provenance mark becomes eligible for the statement-scope free.
    ///
    /// It is the exact complement of `call_returns_param_borrow` above, keyed off
    /// the same `functions` map and the same `callback_referenced_functions` set,
    /// and the predicate itself checks the borrow case first so a function can
    /// never be both. A target that is not in `functions` at all — every native
    /// intrinsic, every `LINK` symbol — answers `false` and keeps the plan-25
    /// exemption, which is the fail-closed direction: an unmarked, unpromised
    /// producer keeps leaking and is never wild-freed.
    pub(super) fn call_returns_fresh_string(&self, value: &NirValue) -> bool {
        let target = match value {
            NirValue::Call { target, .. } | NirValue::CallResult { target, .. } => target.as_str(),
            _ => return false,
        };
        if let Some(returns) = self.callable_value_return_type(target) {
            return returns == ParameterType::String;
        }
        self.functions
            .get(target)
            .is_some_and(|f| function_returns_fresh_string(f, &self.callback_referenced_functions))
    }

    /// bug-569: the return type of a call made THROUGH a callable value — a
    /// `FUNC(..) AS U` parameter, local or global — rather than to a named
    /// function, or `None` when `target` does not name such a value.
    ///
    /// A `Call` carries only its target's NAME, so an indirect invocation
    /// (`f(e.value)` in `__collections_mapValues`, `g(s)` in a user-written HOF)
    /// is indistinguishable from a direct call by shape. Resolving it through
    /// `functions` is wrong twice over: the usual answer is "not found", which
    /// silently keeps the plan-25 exemption and leaks; and where a top-level
    /// function happens to share the parameter's name, the answer is a promise
    /// made by a function this call never reaches. A callable value is looked up
    /// FIRST, so the binding wins over the shadowed name, exactly as it does at
    /// the call itself.
    ///
    /// Locals before globals, matching `overload_arg_type`'s resolution of the
    /// same three tables (bug-497).
    fn callable_value_return_type(&self, target: &str) -> Option<ParameterType> {
        let type_ = self
            .locals
            .get(target)
            .map(|local| local.type_.clone())
            .or_else(|| self.globals.get(target).map(|global| global.type_.clone()))?;
        match type_ {
            ParameterType::Func(_, returns, _) => Some(*returns),
            _ => None,
        }
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
        // bug-569: a call through a callable value is never a param borrow. Every
        // function reachable as a `FUNC` value is invoked through the
        // `FunctionRef` ABI, which owns and frees its result, and
        // `function_returns_param_borrow` excludes that set for exactly that
        // reason (plan-86 K1). Answering from `functions` here would let a
        // top-level function that shares the parameter's name decide, and a
        // borrow verdict is the one that cannot be taken back: it tells the
        // caller the block belongs to its own argument.
        if self.callable_value_return_type(target).is_some() {
            return false;
        }
        self.functions
            .get(target)
            .is_some_and(|f| function_returns_param_borrow(f, &self.callback_referenced_functions))
    }

    /// Whether lowering `value` yields a value whose lifetime is managed by the
    /// thread runtime, not by this scope — so its binding registers no owned-value
    /// free and an unbound temp is not freed (same exclusion principle as
    /// resources).
    ///
    /// That is a `thread` family call whose RAW helper result reaches the caller
    /// uncopied ([`Self::runtime_call_result_is_foreign_arena`]). A member read is
    /// never one: the thread `t.result` field is removed (`ir::verify` refuses it,
    /// TYPE_THREAD_RESULT_REMOVED), so `.result` is an ordinary record field, and
    /// classing it here left `LET s = b.result` an owning copy nobody freed.
    ///
    /// bug-622 A: it is NOT a call in
    /// [`Self::runtime_call_result_is_copied_at_call_site`]. `emit_runtime_helper_call`
    /// deep-copies that helper's result into the CURRENT thread's arena before the
    /// value exists, so what lowering yields is a fresh block with no other owner —
    /// the worker's original is never seen by this scope. Declaring the copy
    /// runtime-managed left it with no owner at all: a `LET r AS String =
    /// thread::waitFor(t)` leaked 16 B per thread, a recursive union result its
    /// whole copied graph.
    ///
    /// An inline-`TRAP`ped call (`CallResult`) is never runtime-managed, for any
    /// `thread` member: the value it yields is the `Result` block
    /// `materialize_current_result` just built in this frame, around its own copy of the
    /// payload (the raw helper block stays `RawSuccessBlock::OwnedElsewhere` there).
    /// Classifying it by target left the wrapper of every trapped `thread::isRunning` /
    /// `poll` / `cancel` / `send` unowned — 144 B per trapped `isRunning` (bug-622).
    pub(crate) fn value_is_runtime_managed(value: &NirValue) -> bool {
        let target = match value {
            NirValue::CallResult { .. } => return false,
            NirValue::Call { target, .. } | NirValue::RuntimeCall { target, .. } => target.as_str(),
            _ => return false,
        };
        Self::runtime_call_result_is_foreign_arena(target)
            && !Self::runtime_call_result_is_copied_at_call_site(target)
    }

    /// bug-622 A: whether `emit_runtime_helper_call` deep-copies this runtime call's
    /// success value into the CURRENT thread's arena (`copy_value_to_current_arena`)
    /// before handing it on — the single source of truth for that copy and for the
    /// ownership of what it yields.
    ///
    /// These five are the `thread` members that READ a value out of another arena:
    /// a worker's terminal result (`waitFor`) or a queued message / resource
    /// (`receive`/`read`, `acceptResource`/`readResource`). The RAW helper result is
    /// still another arena's ([`Self::runtime_call_result_is_foreign_arena`] keeps
    /// answering `true`); the COPY is this frame's, freshly `_mfb_arena_alloc`ed for
    /// every freeable-flat or graph type, so its owner frees it like any other fresh
    /// value. "Current" is whichever thread runs the call — inside a worker
    /// (`thread::receive(w)`) that is the worker's own arena, and so is the free.
    ///
    /// Both spellings the frontend and the emitter use are listed: the NIR names
    /// (`receive`, `acceptResource`) and the handle-direction runtime names they are
    /// rewritten to (`read`, `readResource`).
    pub(crate) fn runtime_call_result_is_copied_at_call_site(target: &str) -> bool {
        matches!(
            target,
            "thread.waitFor"
                | "thread.read"
                | "thread.receive"
                | "thread.acceptResource"
                | "thread.readResource"
        )
    }

    /// bug-566: whether the block a RUNTIME HELPER returns lives outside the
    /// calling thread's arena — the one exclusion that separates "this frame may
    /// free it" from a cross-arena wild free.
    ///
    /// This describes the RAW helper result only. bug-622 A: for the members in
    /// [`Self::runtime_call_result_is_copied_at_call_site`] the value the call site
    /// yields is a copy in the current arena, owned there; this predicate still
    /// answers `true` for them because the pointer the helper handed back — the one
    /// an inline `TRAP`'s `materialize_current_result` sees — is not this frame's.
    ///
    /// `x19` (the arena state) is PER-THREAD. A block produced by
    /// `thread::waitFor`, `thread::receive` or `thread.result` lives in the thread's
    /// block arena (plan-156-A: the control block's own arena, which every value
    /// crossing the boundary is copied into), and `materialize_current_result`'s
    /// `worker_error_source` flag exists for exactly that reason on the error side.
    /// Freeing one of those from this thread writes this thread's free list into
    /// another arena's heap.
    ///
    /// It is a package prefix rather than a per-member list because the property is
    /// a property of the `thread` FAMILY: every member of it marshals across the
    /// boundary. `every_block_returning_runtime_helper_is_classified` (in
    /// `codegen::registry`) asserts that the calls this answers `true` for are
    /// exactly the block-returning `thread.*` entries of the runtime-call catalog,
    /// so a new thread member with a block return reds a test rather than silently
    /// joining the freed side.
    ///
    /// Everything else — `fs.readText`, `os.getEnv`, `process.receive`,
    /// `net.resolve`, … — allocates its result in the CALLER's arena with
    /// `_mfb_arena_alloc`, which is why `LET s AS String = fs::readText(p)` has
    /// always freed it at scope drop (`owns_freeable_value` in `lower_bind_op`
    /// excludes only the runtime-managed ones). A helper that broke that would
    /// already be a wild free at every `LET`.
    pub(crate) fn runtime_call_result_is_foreign_arena(target: &str) -> bool {
        target.starts_with("thread.") || target.starts_with("thread::")
    }

    /// bug-566 / bug-576: whether the block a runtime-helper call returned is this
    /// frame's to free once the value it yielded is dead — after
    /// `materialize_current_result` copied it into an inline-`TRAP`'s `Result`
    /// (bug-566), or at statement scope when nothing bound it at all (bug-576).
    ///
    /// This is the SAME gate a `Bind` of the same call already applies —
    /// `owns_freeable_value` in `lower_bind_op` registers a scope-drop
    /// `arena_free` for exactly `!runtime_managed && is_freeable_flat_value(T)` —
    /// so the licence is not new: it is the licence the non-trapped spelling of the
    /// same call has always used, asked at the site where the `TRAP` desugar
    /// bypassed it. `LET s AS String = fs::readText(p)` is flat; the same call under
    /// a `TRAP` leaked ~130 B per iteration until this answered.
    ///
    /// A resource handle, a `Thread`, a scalar: `is_freeable_flat_value` is false,
    /// so nothing is emitted. A `thread.*` result: foreign arena, likewise nothing.
    pub(crate) fn runtime_result_is_caller_owned(
        &self,
        target: &str,
        result_type: &ParameterType,
    ) -> bool {
        !Self::runtime_call_result_is_foreign_arena(target)
            && (self.is_freeable_flat_value(result_type) || self.owns_graph(result_type))
    }

    /// bug-643: whether the RAW result of a runtime helper is a **resource record**
    /// this frame's own arena holds — the one owned shape
    /// [`Self::runtime_result_is_caller_owned`] deliberately answers `false` for,
    /// because `is_freeable_flat_value` excludes resources (their record is
    /// reclaimed by the resource cleanup, never by a generic `arena_free`).
    ///
    /// The distinction matters at exactly one place: the inline-`TRAP` `Result`
    /// wrap (`materialize_current_result`). For a **sendable** resource it used to
    /// route the producer's record through `copy_resource_to_current_arena`, which
    /// allocates a SECOND resource record, copies the header into it, and tombstones
    /// the source `moved|closed`. That is right across a thread boundary and wrong
    /// in the same arena: the producer's record is already here, nothing else names
    /// it, the moved bit makes `emit_resource_block_reclaim` skip it — so every
    /// `RES c = udp::bind(…) TRAP … END TRAP` leaked one record per bind. A
    /// non-sendable resource (`tls::Socket`, audio) already takes the pointer-carry
    /// arm and never leaked, which is why the leak looked resource-kind-specific.
    ///
    /// Two exclusions, both inherited rather than restated: a `thread.*` result is
    /// the thread's block-arena record (`runtime_call_result_is_foreign_arena`) and must keep
    /// its deep copy, and a **borrowed** element result (`collections::get`,
    /// `tcp::poll` over a list) is a pointer into a collection whose owner closes
    /// it, which [`Self::raw_result_is_borrowed_resource`] answers for.
    pub(crate) fn runtime_result_is_owned_resource_record(
        &self,
        target: &str,
        result_type: &ParameterType,
    ) -> bool {
        // plan-156-C: `thread::start` allocates the parent view in the CALLER's arena,
        // like any opener, even though its package's other results are foreign.
        (target == "thread.start" || !Self::runtime_call_result_is_foreign_arena(target))
            && !Self::target_returns_borrowed_resource(target)
            && self.is_sendable_resource_nominal(result_type)
    }

    /// bug-648: whether a trapped call's success value is a resource (or resource
    /// union) ELEMENT that its collection still owns and closes — the
    /// [`RawSuccessBlock::BorrowedResource`] answer. The `Result` wrap carries it by
    /// pointer, because the deep copy tombstones its source `moved|closed`, and the
    /// `TRAP` temp that receives it does not own it (`trap_ownership`). A
    /// non-resource `get` (a `String`, a record) keeps its owned copy.
    pub(crate) fn raw_result_is_borrowed_resource(
        &self,
        target: &str,
        result_type: &ParameterType,
    ) -> bool {
        Self::target_returns_borrowed_resource(target)
            && (self.resource_cleanup_symbol(result_type).is_some()
                || self.resource_union_cleanup(result_type).is_some())
    }

    /// bug-576: whether a runtime helper must MARK its result fresh
    /// (`mark_fresh_string`) for the statement-scope free to claim it — the extra
    /// hurdle exactly one result type has.
    ///
    /// `register_pending_temp` already frees an unbound, freeable-flat call result
    /// with no provenance at all: a `List OF Byte` from `fs::readBytes`, a
    /// `net.Address` from `tcp::localAddress`. Those blocks are self-contained and
    /// could only have come from an allocation. A bare `String` is the one shape
    /// that might instead be rodata or a view into an argument, so plan-25 exempted
    /// every `String` and bug-536 shape B reinstated the free only for a producer
    /// that marks the block it just allocated. A runtime helper allocates its result
    /// with `_mfb_arena_alloc` in the calling thread's arena and hands back the only
    /// pointer — but it never marked, so `LET s AS String = os::arch()` was flat
    /// while `len(os::arch())` leaked 64 B per call, `os::hostName()` 129 B and
    /// `fs::tempDirectory()` 260 B.
    ///
    /// Wildcard-free on purpose: a new `ParameterType` variant is a BUILD ERROR
    /// here rather than a silent `false`. (`false` is the pre-fix leaking
    /// direction for a `String`-like shape and the already-correct answer for
    /// everything else, so the wildcard would be quiet in both directions.)
    fn runtime_result_needs_fresh_string_mark(result_type: &ParameterType) -> bool {
        match result_type {
            // The one type `pending_temp_is_freeable` refuses without provenance.
            ParameterType::String => true,
            // Everything else is already answered by `pending_temp_is_freeable`
            // alone: a scalar / `Nothing` / handle / type-level shape is not
            // freeable-flat and nothing is emitted for it, and a composite block
            // (collection, `Result`, record, data union) is freed unmarked.
            ParameterType::AttributeString
            | ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Integer
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::ListOf(_)
            | ParameterType::MapOf(_, _)
            | ParameterType::SetOf(_)
            | ParameterType::MapEntryOf(_, _)
            | ParameterType::ResultOf(_)
            | ParameterType::Res(_)
            | ParameterType::Stateful { .. }
            | ParameterType::C(_)
            | ParameterType::Named(_)
            | ParameterType::UserOf(_, _)
            | ParameterType::Var(_)
            | ParameterType::Func(_, _, _)
            | ParameterType::Arg(_)
            | ParameterType::Unknown
            | ParameterType::ThreadHandle { .. } => false,
        }
    }

    /// bug-576: mark a non-`raw` runtime helper's `String` result as a fresh block
    /// this frame owns, so an UNBOUND one reaches the statement-scope free.
    ///
    /// Called by `emit_runtime_helper_call` on the value it is about to return, so
    /// the mark names that node's own result operand and `lower_value`'s identity
    /// test accepts it. The gate is [`Self::runtime_result_is_caller_owned`] — the
    /// SAME question the `Bind` path (`owns_freeable_value`) and bug-566's
    /// inline-`TRAP` path already ask of this call, so no new licence is created
    /// here: it is the licence `LET s AS String = fs::readText(p)` has always
    /// exercised, asked at the position that had no owner at all. A bound or
    /// returned result is unaffected because its owner claims the temp
    /// (`claim_pending_temp`) exactly as it does for a marked native producer.
    ///
    /// The raw `thread.*` block is excluded (it is the thread's block arena's —
    /// `x19` is per-thread). bug-622 A: `block` is never that raw block for a call in
    /// [`Self::runtime_call_result_is_copied_at_call_site`] — it is the operand
    /// `copy_value_to_current_arena` just allocated in this arena — so those calls
    /// are marked too, and an unbound `len(thread::waitFor(t))` frees its copy.
    pub(crate) fn mark_runtime_helper_result_fresh(
        &mut self,
        target: &str,
        result_type: &ParameterType,
        block: impl Into<Operand>,
    ) {
        if Self::runtime_result_needs_fresh_string_mark(result_type)
            && (self.runtime_result_is_caller_owned(target, result_type)
                || Self::runtime_call_result_is_copied_at_call_site(target))
        {
            self.mark_fresh_string(block);
        }
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
    /// * `Local` — naming an existing binding, resource parameter, or `FOR EACH`
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
            // (`lower_bind_op`, `builder_control/bind_op.rs`: `resource_cleanup_symbol(type_)` or
            // `resource_union_cleanup(type_)`). A `MemberAccess` reading an
            // ordinary field is not resource-typed, so it never reaches the
            // alias branch and still takes the owned-value path below it.
            NirValue::MemberAccess { .. } => true,
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => {
                Self::target_returns_borrowed_resource(target)
            }
            _ => false,
        }
    }

    /// The call TARGETS whose resource result is a pointer to a resource another
    /// owner still closes — the borrowed-element shapes named in
    /// [`Self::value_aliases_live_resource`]'s doc, split out so the raw
    /// inline-`TRAP` path can ask the same question of a bare target string
    /// (bug-643) instead of restating the list.
    pub(crate) fn target_returns_borrowed_resource(target: &str) -> bool {
        matches!(
            crate::codegen::registry::native_bare_target(target),
            Some("get" | "getOr")
        ) || matches!(target, "tcp.poll" | "udp.poll" | "tls.poll")
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

    /// plan-134-D: whether an owning store of `type_` needs the graph deep copy — a value
    /// whose type reaches a type cycle is a pointer-linked graph `copy_flat_block` cannot
    /// copy — and may take it: a value with a resource anywhere inside it is move-only and
    /// keeps today's alias. The one definition every store uses (`lower_value_owned`,
    /// `lower_returned_value`, `materialize_owned_element`).
    ///
    /// bug-781: a value holding a function value is in the class too — the closure it holds
    /// is its own, so an owning store copies the closure and its environment.
    pub(crate) fn needs_graph_copy(&self, type_: &ParameterType) -> bool {
        !self.is_freeable_flat_value(type_)
            && (crate::codegen::collection::layout::type_reaches_cycle(&self.type_model, type_)
                || crate::codegen::collection::layout::type_contains_func(&self.type_model, type_))
            && !crate::codegen::collection::layout::type_contains_resource(&self.type_model, type_)
    }

    /// plan-134-G: whether the owner of a `type_` value frees it, through `_mfb_rt_graph_drop`
    /// (plan-134-F). This is the class `needs_graph_copy` defines, narrowed to the types with a
    /// walker kind; a type that only reaches a cycle has none yet (plan-134-H) and keeps today's
    /// leak. Every ownership gate of plan-134-G §2.1 asks this beside `is_freeable_flat_value`.
    pub(crate) fn owns_graph(&self, type_: &ParameterType) -> bool {
        // bug-781: a bare function value needs no walker kind — its owner frees it with
        // `_mfb_rt_closure_drop` directly (`emit_graph_value_drop`).
        self.needs_graph_copy(type_)
            && (matches!(type_, ParameterType::Func(..)) || self.graph_drop_kind(type_).is_some())
    }

    /// plan-134-D: whether `value`, stored by the op being lowered, is that place's last
    /// read (plan-134-C), so the store may hand the same graph over instead of copying it.
    /// Only a local or a field of a local can be a site; a builder with no analysis (a
    /// synthesized function) answers no, so its stores copy.
    pub(crate) fn store_is_last_use(&self, value: &NirValue) -> bool {
        use crate::codegen::engine::analysis::last_use::Place;
        let (Some(sites), Some(op)) = (self.move_sites.as_ref(), self.current_op_key) else {
            return false;
        };
        let place = match value {
            NirValue::Local(name) => Place::Local(name.clone()),
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                NirValue::Local(name) => Place::Field(name.clone(), member.clone()),
                _ => return false,
            },
            _ => return false,
        };
        sites.is_last_use(op, &place) && self.move_source_is_owned(&place)
    }

    /// plan-134-D: whether the op being lowered binds a borrowed MATCH view (plan-134-C):
    /// the bound local only inspects its source, so its bind needs no copy.
    pub(crate) fn store_is_borrowed_view(&self) -> bool {
        matches!(
            (self.move_sites.as_ref(), self.current_op_key),
            (Some(sites), Some(op)) if sites.is_borrow(op)
        )
    }

    /// plan-134-E: lower a value a CONSTRUCTION store writes into another value — a
    /// constructor argument, a `WITH` update, a union wrap, a collection literal element or
    /// in-place item, a `STATE` replacement. The writers byte-copy a flat payload themselves,
    /// so a flat value lowers exactly as `lower_value` does (no copy, and a fresh temp stays
    /// registered for the statement-scope free). A value whose type reaches a type cycle holds
    /// pointers the byte copy would share, so an aliasing source gets its own graph from the
    /// walker — unless the store is the source's last read (plan-134-C), which moves.
    pub(crate) fn lower_value_stored(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        self.lower_value_stored_as(value, StoreShape::Payload)
    }

    /// plan-134-G: [`Self::lower_value_stored`] for a store that keeps the value's POINTER — a
    /// record constructor argument or `WITH` update (a recursive field is never inlined) and a
    /// resource `STATE` replacement.
    pub(crate) fn lower_value_stored_field(
        &mut self,
        value: &NirValue,
    ) -> Result<ValueResult, String> {
        self.lower_value_stored_as(value, StoreShape::Pointer)
    }

    /// plan-134-G: [`Self::lower_value_stored`] for a union wrap, which byte-copies the variant
    /// record's block into the union.
    pub(crate) fn lower_value_stored_inline(
        &mut self,
        value: &NirValue,
    ) -> Result<ValueResult, String> {
        self.lower_value_stored_as(value, StoreShape::Inline)
    }

    /// plan-134-G: the ownership of a stored recursive value, now that its owners free it. A
    /// store that keeps the pointer takes the whole graph. A store that byte-copies the top
    /// block (a record or union payload, a union wrap) takes that block's children, and the
    /// block it copied is freed shallowly at the end of the statement:
    ///
    /// * a fresh value — its pending temp is claimed (pointer) or made shallow (inline);
    /// * the last read of an owning place (plan-134-C) — the place releases the graph
    ///   ([`Self::release_moved_source`]);
    /// * any other aliasing source — copied, and an inline store frees the copy's top block.
    fn lower_value_stored_as(
        &mut self,
        value: &NirValue,
        shape: StoreShape,
    ) -> Result<ValueResult, String> {
        let result = self.lower_value(value)?;
        if !self.needs_graph_copy(&result.type_) {
            return Ok(result);
        }
        let owned = self.owns_graph(&result.type_);
        let inline = match shape {
            StoreShape::Pointer => false,
            StoreShape::Inline => true,
            StoreShape::Payload => {
                self.payload_edge_shape(&result.type_) != Some(PayloadEdgeShape::Pointer)
            }
        };
        if !self.value_needs_owning_copy(value)
            || Self::is_fresh_trapped_result_wrapper(value, &result.type_)
        {
            if owned {
                if inline {
                    self.make_pending_temp_shallow(&result);
                } else {
                    self.claim_pending_temp(&result);
                }
            }
            return Ok(result);
        }
        if self.store_is_borrowed_view() {
            return Ok(result);
        }
        if self.store_is_last_use(value) {
            if owned {
                self.release_moved_source(value, &result, inline)?;
            }
            return Ok(result);
        }
        let copied = self.copy_value_to_current_arena(&result.type_, &result.location)?;
        let copied = ValueResult {
            origin: None,
            type_: result.type_,
            location: Operand::from(copied),
            text: result.text,
        };
        if owned && inline {
            self.register_shallow_temp(&copied);
        }
        Ok(copied)
    }

    /// plan-134-G: turn the pending temp `result` owns (the most recently registered, when its
    /// location matches) into a shallow free.
    fn make_pending_temp_shallow(&mut self, result: &ValueResult) {
        if let Some(temp) = self
            .pending_temp_frees
            .last_mut()
            .filter(|temp| temp.location == result.location)
        {
            temp.shallow = true;
        }
    }

    /// plan-134-G: a statement-scope shallow free of `result`'s block.
    fn register_shallow_temp(&mut self, result: &ValueResult) {
        let slot = self.allocate_stack_object("shallow_temp", 8);
        self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
        self.pending_temp_frees.push(PendingTemp {
            type_: result.type_.clone(),
            slot,
            location: result.location.clone(),
            shallow: true,
            resource: false,
            union_box: false,
        });
    }

    /// plan-134-G: a store took `value`'s graph at its place's last read, so the place gives it
    /// up: its word is zeroed, which every drop of the place skips. An `inline` store copied
    /// only the top block's bytes, so that block is also freed (shallowly) at the end of the
    /// statement. `store_is_last_use` only answers yes for a place whose root local owns its
    /// block and, for a field, a pointer field of a record (`move_source_is_owned`).
    pub(crate) fn release_moved_source(
        &mut self,
        value: &NirValue,
        result: &ValueResult,
        inline: bool,
    ) -> Result<(), String> {
        if inline {
            self.register_shallow_temp(result);
        }
        match value {
            NirValue::Local(name) => {
                let slot = self
                    .locals
                    .get(name)
                    .map(|local| local.stack_offset)
                    .ok_or_else(|| format!("moved local '{name}' is not bound"))?;
                self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
                Ok(())
            }
            NirValue::MemberAccess { target, member } => {
                let base = self.lower_value(target)?;
                let offset = self
                    .pointer_field_offset(&base.type_, member)
                    .ok_or_else(|| {
                        format!(
                            "moved field '{member}' of '{}' is not a record pointer field",
                            base.type_
                        )
                    })?;
                self.emit(abi::store_u64(abi::ZERO, &base.location, offset));
                Ok(())
            }
            _ => Err("a moved store source must be a local or a field of one".to_string()),
        }
    }

    /// plan-134-G: the offset of record `type_`'s field `member` when it is a pointer field (a
    /// separate allocation), `None` for an inlined field or a non-record.
    fn pointer_field_offset(&self, type_: &ParameterType, member: &str) -> Option<usize> {
        let fields = self.type_model.record_fields.get(type_)?;
        let index = fields.iter().position(|(name, _)| name == member)?;
        (!self.record_field_is_inlined(&fields[index].1)).then_some(8 * index)
    }

    /// plan-134-G: a place may be moved out of only if its root local owns its block — a live
    /// `OwnedValue` cleanup at the local's slot, which is what `release_moved_source` gives up.
    /// A `MATCH` view, a by-ref capture or any other alias owns nothing, so its store copies.
    fn move_source_is_owned(
        &self,
        place: &crate::codegen::engine::analysis::last_use::Place,
    ) -> bool {
        use crate::codegen::engine::analysis::last_use::Place;
        let (root, member) = match place {
            Place::Local(name) => (name, None),
            Place::Field(name, member) => (name, Some(member)),
        };
        let Some(local) = self.locals.get(root) else {
            return false;
        };
        if local.by_ref {
            return false;
        }
        let owns = self.active_cleanups.iter().any(|cleanup| {
            matches!(cleanup, ActiveCleanup::OwnedValue(c) if c.stack_offset == local.stack_offset)
        });
        owns && member
            .is_none_or(|member| self.pointer_field_offset(&local.type_, member).is_some())
    }

    /// bug-781: what the environment owns in the word a capture of `capture`,
    /// lowered to a value of `type_`, is stored in. The rule is the one
    /// `lower_value_owned` applied to produce that value, so the environment owns
    /// exactly the blocks that call handed it: a by-value `String`, flat
    /// composite or graph-owning value (a recursive value, a function value, a
    /// composite holding one). A by-ref capture holds the address of another
    /// binding's slot, and a scalar, resource or thread handle owns no block, so
    /// their words are copied verbatim and never freed.
    pub(crate) fn closure_env_word(
        &self,
        capture: &NirValue,
        type_: &ParameterType,
    ) -> ClosureEnvWord {
        if matches!(
            capture,
            NirValue::LocalRef { .. } | NirValue::Capture { by_ref: true, .. }
        ) {
            return ClosureEnvWord::Plain;
        }
        if self.resource_union_cleanup(type_).is_some() {
            return ClosureEnvWord::ResourceUnionBox;
        }
        if self.is_freeable_flat_value(type_) || self.owns_graph(type_) {
            ClosureEnvWord::Owned(type_.clone())
        } else {
            ClosureEnvWord::Plain
        }
    }

    /// bug-781: note the environment layout of a `LAMBDA` this function creates,
    /// for the module to emit its environment routine once. A `LAMBDA` created at
    /// several sites (a cloned loop body) must describe the same layout at each:
    /// one routine serves every environment the `LAMBDA` makes.
    pub(crate) fn record_closure_env_layout(
        &mut self,
        layout: ClosureEnvLayout,
    ) -> Result<(), String> {
        match self
            .closure_env_layouts
            .iter()
            .find(|known| known.symbol == layout.symbol)
        {
            Some(known) if *known == layout => Ok(()),
            Some(known) => Err(format!(
                "closure environment `{}` is created with two layouts: {known:?} and {layout:?}",
                layout.symbol
            )),
            None => {
                self.closure_env_layouts.push(layout);
                Ok(())
            }
        }
    }
}
