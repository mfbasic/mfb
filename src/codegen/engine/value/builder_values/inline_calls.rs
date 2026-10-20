//! plan-176-U: inline and ABI call lowering — checked values, inline conversions and builtins,
//! the MFB fast path, infallible members, the `net` form predicates and runtime helper calls.
//! The entry points and shared types live in `engine/value/builder_values/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
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
    pub(super) fn lower_checked_value(
        &mut self,
        success_type: &ParameterType,
        value: &NirValue,
        observe: bool,
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
        //
        // bug-798: only where the capture sits at such a boundary. A `MOD`/`^`
        // lifted from inside an operator tree (`observe` FALSE) is captured for
        // its domain error alone; its result flows on into the tree, and the
        // tree's own boundary observes it exactly as the untrapped form does.
        let lowered = self.lower_value(value).and_then(|success| {
            if observe {
                self.observe_float(value, &success)?;
            }
            Ok(success)
        });
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
        // bug-566: a `Checked` operand is lowered by `lower_value`, which
        // registered its own pending temp — the statement-scope drop frees it, and
        // a second free here would be a double free. plan-165-D: on bug-778's
        // unboxed path a fresh block result (`a & b TRAP(e)`) is instead taken over
        // by the temporary, exactly as a raw member's is, so the trapped form
        // copies nothing the untrapped binding does not.
        let raw_success = if self.claim_raw_member_result_for_unboxed_trap(&success) {
            RawSuccessBlock::OwnedByThisFrame
        } else {
            RawSuccessBlock::OwnedElsewhere
        };
        self.materialize_current_result(success_type, "checked".to_string(), false, raw_success)
    }

    /// Lower an inline conversion built-in (`toInt`/`toFloat`/`toFixed`/`toByte`)
    /// for an inline `TRAP`: emit the normal inline conversion but capture its
    /// error return (which would otherwise auto-propagate) so the raw `Result`
    /// is left in the standard registers, then materialize it as a value.
    pub(super) fn lower_inline_conversion_raw(
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
        // bug-566: `toInt`/`toFloat`/`toByte`/`toMoney`/`toScalar` produce scalars,
        // for which `result_payload_is_block` is false and nothing is emitted.
        // `toFixed` produces a `Fixed`, also a scalar. No conversion here allocates
        // a block, so there is none to own.
        self.materialize_current_result(
            &success_type,
            format!("callResult {target}"),
            false,
            RawSuccessBlock::OwnedElsewhere,
        )
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
    pub(super) fn try_abi_inline_lower(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Option<Result<ValueResult, String>> {
        let lower = crate::codegen::registry::abi_inline_lower(target)?;
        // An inline body reads its arguments positionally (`args[1]`, `args[2]`),
        // so a call whose arity is outside what the member declares does not
        // refuse -- it indexes past the end and PANICS, and a panicking build has
        // no located error to show for itself. Arity is the front end's
        // invariant, not something a program can break, so this is not a
        // diagnostic anyone should see; it is the one place the invariant is
        // CHECKED, so a defect upstream reports itself here instead of taking the
        // process down. Measured: 280 arity-corrupted modules panicked before it,
        // the same 56 across each of the five backends.
        //
        // The bound is the registry's own `(min, max)` -- `min` counts the
        // required parameters and `max` the widest overload -- because an
        // optional parameter is a REAL short call: `strings::padRight(s, 8)` omits
        // the pad character and reaches the body with two arguments for a
        // three-parameter member. A guard written as "exactly the parameter
        // count" refuses that, which is how this one was first written and what
        // the corpus caught.
        if let Some((min, max)) = crate::codegen::registry::registry().arity(target) {
            if args.len() < min || args.len() > max {
                return Some(Err(format!(
                    "native code inline call '{target}' has {} argument(s), but \
                     its lowering is written for {min}..={max}",
                    args.len()
                )));
            }
        }
        // bug-863: lowering pads every omitted `Fill` parameter with the default of
        // the overload the call resolves to. A call missing one got past the front
        // end unpadded, which is a front-end defect, not a short call.
        if !crate::codegen::registry::call_carries_its_fill_defaults(target, args.len()) {
            return Some(Err(format!(
                "internal error: call '{target}' reached code generation with {} \
                 argument(s) and its default parameters unpadded",
                args.len()
            )));
        }
        // bug-760 B1: a hash lookup finds only live entries — a tombstone has no
        // bucket, and the lazy index build skips it; a key type that scans instead
        // never holds tombstones (`lower_map_remove_key_in_place`). So its map
        // operand is read as it is, and a lookup loop over a map that removals
        // keep leaving tombstones in does not settle it every time.
        self.map_settle_exempt_armed = matches!(
            target,
            "collections.get" | "collections.getOr" | "collections.hasKey" | "collections.contains"
        );
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
        let lowered = crate::trace::timed_tally(
            "abi_inline builtin",
            || target.to_string(),
            || Some(lower(self, &arg_values, &ctx)),
        )?;
        // plan-134-E: a collection built out of other values owns its elements' graphs.
        // A member whose result already owns its graphs is not one: an element READ
        // (`get`/`getOr`) is `materialize_owned_element`'s independent copy, and
        // `transform` owns its callback results. Copying their edges again orphaned
        // the first copies — one block per edge per call.
        if crate::codegen::builtins::collections::member_result_owns_its_graph(target) {
            return Some(lowered);
        }
        Some(lowered.and_then(|result| self.own_collection_payload_edges(result)))
    }

    /// Pre-lower each `NirValue` arg to a `ValueResult` for an `AbiInline` body
    /// ("pre-lowered `ValueResult`"). A single arg is consumed immediately and needs
    /// no stabilization. With two or more args, lowering a later arg can reset
    /// temporaries and clobber an earlier arg's register, so each is spilled to a
    /// fresh stack slot and, after the reset, reloaded into a persistent register —
    /// the exact spill/reset/reload the former `bits::gen_two_integers` did by hand,
    /// now owned by the dispatch so every multi-arg `AbiInline` body gets
    /// non-aliasing operands (and byte-identically to the pre-migration bodies).
    pub(crate) fn lower_abi_inline_args(
        &mut self,
        args: &[NirValue],
    ) -> Result<Vec<ValueResult>, String> {
        if args.len() <= 1 {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                let value = self.lower_value(arg)?;
                self.observe_float(arg, &value)?;
                values.push(value);
            }
            return Ok(values);
        }
        // Spill each lowered arg to its own stack slot (arg0 spilled before arg1 is
        // lowered, so a temporary reset cannot clobber it).
        let mut spilled = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let value = self.lower_value(arg)?;
            // bug-754: an argument is an observation boundary (plan-17), for an
            // inline builtin exactly as for a called one (`emit_call`). Checked as
            // each is lowered, so a later argument's effects never run first.
            self.observe_float(arg, &value)?;
            // A register-native vector (plan-01-vector) is one register PER LANE, and
            // the slot below is one word: it has to become its record block first.
            // Without this a vector built in place as an argument —
            // `RETURN collections::append(ring, vector::Float2[…])` — reached the
            // store as a `%%vecnative` operand and the compiler panicked. Identity for
            // everything that is not a native vector, so no other argument changes.
            let value = self.vector_value_as_block(value)?;
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
                location: Operand::from(register),
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
    pub(super) fn try_mfb_fast_path(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Option<Result<ValueResult, String>> {
        let fast_path = crate::codegen::registry::mfb_fast_path(target)?;
        fast_path(self, target, args).transpose()
    }

    pub(super) fn lower_inline_builtin_raw(
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
                // bug-533: only the `String` overload of `replace` reaches here
                // — `inline_builtin_raw_supported` gates on the argument type, and
                // the `List` overload is infallible. `lower_replace`'s empty-`old`
                // guard raises `ErrInvalidArgument` through the same
                // `emit_error_register_return` tail as the index members above, so
                // the `raw_result_capture` set here redirects it to the capture
                // point instead of auto-propagating past the handler.
                Some("replace") => self.lower_replace(args),
                other => Err(format!(
                    "native raw inline builtin '{target}' ({other:?}) is not supported"
                )),
            }
        };
        self.raw_result_capture = previous;
        let success = lowered?;
        // bug-561: `materialize_current_result` COPIES this block into the
        // `Result`; nothing else owns it afterwards, and `lower_value`'s
        // registration was bypassed by lowering the member directly.
        self.register_raw_member_result_temp(target, args, &success);
        // bug-566: the line above is bug-561's answer for this site — the block is
        // registered as a statement-scope pending temp, so THIS frame frees it
        // there, not here. Two frees would be a double free.
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
        let raw_success = if self.raw_result_is_borrowed_resource(target, &success_type) {
            RawSuccessBlock::BorrowedResource
        } else if self.claim_raw_member_result_for_unboxed_trap(&success) {
            RawSuccessBlock::OwnedByThisFrame
        } else {
            RawSuccessBlock::OwnedElsewhere
        };
        self.materialize_current_result(
            &success_type,
            format!("callResult {target}"),
            false,
            raw_success,
        )
    }

    /// Inline `TRAP` on a provably-infallible inline built-in (plan-26-A). Unlike
    /// [`lower_inline_builtin_raw`](Self::lower_inline_builtin_raw) there is no
    /// error exit to capture — the member cannot fail — so we lower it exactly as
    /// the normal call path would, then tag the single-register success `Ok` and
    /// materialize an always-`Ok` `Result OF <success>`. No capture label is
    /// needed (nothing ever branches to the error tail). The `TRAP`'s handler is
    /// therefore dead code, which the front-end flags with the advisory
    /// `TYPE_INLINE_TRAP_DEAD_HANDLER` warning.
    pub(super) fn lower_inline_infallible_raw(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let success = self.lower_infallible_member(target, args)?;
        // bug-561: same bypass, same abandoned block — see
        // `register_raw_member_result_temp`. This member cannot fail, so the
        // registration is unconditionally on the taken path.
        self.register_raw_member_result_temp(target, args, &success);
        let success_type = success.type_.clone();
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &success.location));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        let raw_success = if self.raw_result_is_borrowed_resource(target, &success_type) {
            RawSuccessBlock::BorrowedResource
        } else if self.claim_raw_member_result_for_unboxed_trap(&success) {
            RawSuccessBlock::OwnedByThisFrame
        } else {
            RawSuccessBlock::OwnedElsewhere
        };
        self.materialize_current_result(
            &success_type,
            format!("callResult {target}"),
            false,
            raw_success,
        )
    }

    /// bug-778: an unboxed inline-`TRAP` temporary OWNS its payload. When the raw
    /// member's block is the statement-scope temp `register_raw_member_result_temp`
    /// just registered — a fresh block of this frame's — the temporary takes it
    /// over instead of copying it and letting the statement free the original,
    /// exactly as an owning `Bind` of the untrapped member claims it
    /// (`lower_value_owned`). Only on the unboxed path: the heap `Result`'s own
    /// copy-and-free answer for `OwnedByThisFrame` is a different contract.
    fn claim_raw_member_result_for_unboxed_trap(&mut self, success: &ValueResult) -> bool {
        if self.unboxed_trap_active.is_none() || !self.pending_temp_would_be_claimed(success) {
            return false;
        }
        self.claim_pending_temp(success);
        true
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
                location: Operand::from(register),
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
        let helper_args = args.to_vec();
        self.lower_runtime_helper_call_copied(helper, target, args, helper_args, raw)
    }

    /// [`Self::lower_runtime_helper_call`] over `helper_args`, its own copy of
    /// `args`.
    fn lower_runtime_helper_call_copied(
        &mut self,
        helper: runtime::RuntimeHelper,
        target: &str,
        args: &[NirValue],
        mut helper_args: Vec<NirValue>,
        raw: bool,
    ) -> Result<ValueResult, String> {
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
        // (`io::input`'s omitted prompt and timeout are `DefaultValue::Fill`s,
        // padded in the IR like every other optional argument — bug-699.)
        if target == "thread.start" {
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
                value: crate::codegen::builtins::net::DEFAULT_LISTEN_BACKLOG.to_string(),
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
            // bug-630: a GENERIC descriptor return never resolves here. The
            // catalog freezes `thread.waitFor`'s `Var("Out")` into the ABI string
            // `"Out"`, and `declared("Out")` is not a type — it used to flow into
            // the assignment's marshalling and fail there ("native inlined field
            // size not available for type 'Out'"), which named the symptom and
            // hid the cause. The seven generic rows are all `thread.*` and every
            // one is typed from its handle argument above, so declining costs
            // nothing and turns a mis-lowering into the resolver's own error.
            .or_else(|| {
                runtime::spec_for_call(target)
                    .filter(|spec| !spec.abi.returns_generic)
                    .map(|spec| ParameterType::declared(spec.abi.returns))
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
            // bug-809: `fs::openData(String, mode)` and `(List OF Byte, mode)` read
            // their bytes from different offsets, so the byte-list overload lowers
            // through its own `fs.openDataBytes` code form, selected by the static
            // type of the value — the same refusal-on-unknown rule as `tcp::write`.
            "fs.openData" => self.net_write_payload_form(
                args.first(),
                crate::codegen::builtins::fs::func_open_data::OPEN_DATA_BYTES,
                "fs.openData",
            )?,
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
