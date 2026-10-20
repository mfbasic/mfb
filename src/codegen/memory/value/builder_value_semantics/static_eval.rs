//! plan-176-P: static evaluation — globals, constant locals, static strings and the static type of
//! an operand, call or composite. String concatenation and `MATCH` comparison live in
//! `memory/value/builder_value_semantics/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn global_value(&self, name: &str) -> Result<GlobalValue, String> {
        self.globals
            .get(name)
            .cloned()
            .ok_or_else(|| format!("native code global '{name}' does not resolve"))
    }

    pub(crate) fn load_global_address(&mut self, name: &str) -> Result<VirtualRegister, String> {
        let global = self.global_value(name)?;
        let register = self.allocate_register();
        self.emit(abi::add_immediate(
            &register,
            ARENA_STATE_REGISTER,
            global.offset,
        ));
        Ok(register)
    }

    pub(crate) fn local_constant_value(&self, value: &NirValue) -> Option<NirValue> {
        match value {
            NirValue::Const { .. } => Some(value.clone()),
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.clone()),
            NirValue::Global { .. } => None,
            NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => self
                .static_primitive_text(&args[0])
                .map(|value| NirValue::Const {
                    type_: ParameterType::String,
                    value,
                }),
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
                    .map(|value| NirValue::Const {
                        type_: ParameterType::String,
                        value,
                    })
            }
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                // `typeName` folds to the argument type's SPELLING.
                self.static_type_name(&args[0])
                    .map(|type_| NirValue::Const {
                        type_: ParameterType::String,
                        value: type_.name().into_owned(),
                    })
            }
            NirValue::Binary { op, .. } if *op == BinaryOp::Concat => self
                .static_string_value(value)
                .map(|value| NirValue::Const {
                    type_: ParameterType::String,
                    value,
                }),
            _ => None,
        }
    }

    /// [`static_string_value`](Self::static_string_value) for a **pre-lowered**
    /// `abi_inline` arg: the constant-folding reads the source `NirValue` off the
    /// `ValueResult::origin` (the value is already lowered, but its source node is
    /// kept for exactly this kind of compile-time fold).
    pub(crate) fn static_string_value_vr(&self, value: &ValueResult) -> Option<String> {
        value
            .origin
            .as_ref()
            .and_then(|nir| self.static_string_value(nir))
    }

    pub(crate) fn static_string_value(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } if matches!(type_, ParameterType::String) => {
                Some(value.clone())
            }
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_string_value(constant)),
            NirValue::Global { .. } => None,
            NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
                self.static_primitive_text(&args[0])
            }
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
            }
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                // MUST use the SAME resolver the builder's `typeName` fold uses
                // (`static_type_name_for_fold`, via `resolve_call_return_type`), not
                // the coarser `static_type_name` whose call arm only recognizes a
                // hardcoded builtin list. The builder folds `typeName(<any call>)`
                // — e.g. `typeName(math::abs(x))` or `typeName(io::isBuffered())` —
                // to a rodata `String` constant; if this ownership classifier misses
                // that fold (as `static_type_name` does for a package call not in its
                // list), `value_needs_owning_copy` returns false and `LET t =
                // typeName(math::abs(x))` binds `t` straight to the rodata pointer
                // with no deep copy — scope-drop then `arena_free`s a read-only
                // constant (a write into rodata → SIGBUS). Same fold-mismatch class
                // as the strings-package note below.
                self.static_type_name_for_fold(&args[0])
                    .map(|type_| type_.name().into_owned())
            }
            NirValue::Binary {
                op, left, right, ..
            } if *op == BinaryOp::Concat => {
                let left = self.static_string_value(left)?;
                let right = self.static_string_value(right)?;
                Some(format!("{left}{right}"))
            }
            // The unicode case/normalization builtins fold a static-string argument
            // to a static result (each `strings::` `func_*` lowering consults
            // `static_strings_package_string`). This resolver MUST recognize the
            // same folds: `value_needs_owning_copy` consults it to decide whether a
            // bound value is a rodata constant needing a deep copy. If it misses the
            // fold, `LET r = caseFold("HELLO")` binds `r` straight to the folded
            // rodata pointer with no copy, and scope-drop then `arena_free`s a
            // read-only constant — an arena free-list corruption that only surfaces
            // on a *later* allocation (e.g. the next `mfb test` case).
            NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. }
                if args.len() == 1
                    && matches!(
                        target.as_str(),
                        "strings.upper"
                            | "strings.lower"
                            | "strings.caseFold"
                            | "strings.normalizeNfc"
                    ) =>
            {
                let value = self.static_string_value(&args[0])?;
                Some(match target.as_str() {
                    "strings.upper" => crate::unicode::backend::upper(&value),
                    "strings.lower" => crate::unicode::backend::lower(&value),
                    "strings.caseFold" => crate::unicode::backend::case_fold(&value),
                    "strings.normalizeNfc" => crate::unicode::backend::normalize_nfc(&value),
                    _ => unreachable!(),
                })
            }
            _ => None,
        }
    }

    pub(crate) fn static_primitive_text(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } => match type_ {
                // Float/Fixed constants fold to the runtime formatter's
                // default-precision rendering (2 places; bug-358, plan-28-B).
                ParameterType::Float | ParameterType::Fixed => {
                    crate::numeric::default_to_string_text(type_, value)
                }
                ParameterType::Integer | ParameterType::Byte | ParameterType::String => {
                    Some(value.clone())
                }
                ParameterType::Boolean => match value.as_str() {
                    "true" => Some("TRUE".to_string()),
                    "false" => Some("FALSE".to_string()),
                    _ => None,
                },
                _ => None,
            },
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_primitive_text(constant)),
            NirValue::Global { .. } => None,
            _ => None,
        }
    }

    /// The static type of a value for the **in-place collection gates**, widened
    /// past [`Self::static_type_name`]'s hand-written builtin-name table to the
    /// declared return type of a call to a user (or `LINK`) function.
    ///
    /// Those gates ask one question — "is this operand statically exactly `T`?" —
    /// to tell a single-element `append(list, x)` from a bulk
    /// `append(list, otherList)`. `static_type_name` answers `None` for ANY call it
    /// does not have a hard-coded row for, so every
    /// `list = collections::append(list, someFunc(…))` fell off the in-place path
    /// and copied the whole buffer per element: the accumulate loop ran O(n^2)
    /// (50 000 appends took 60 s against 3 ms for the same loop appending a plain
    /// local). A callee's declared `returns` is exactly as static as a local's
    /// declared type, so answering from it is sound for a gate that only asks the
    /// operand's type — it is NOT a widening of `static_type_name`, whose other
    /// consumers (the float-numeric-error gate, module analysis, binary typing)
    /// must keep their exact current answers.
    ///
    /// bug-626: a builtin call is typed too, from the registry resolver the type
    /// checker uses, so `keep = collections::append(keep, fs::readText(p))` no
    /// longer misses the in-place path for want of a row in the name table (it
    /// copied the whole list per append: 5,000 appends of a 5,000-byte file
    /// mapped 63 GB). Every caller compares the answer for equality with the
    /// element, key or collection type and re-checks the lowered type, and no
    /// call is an aliasing source (`value_is_aliasing_source`), so this admits
    /// nothing a user function's result was not already admitted with.
    ///
    /// This query is **not** the one the in-place gates ask — since bug-681 they
    /// ask [`Self::static_operand_type`], which widens it once more. Keep this
    /// one's answers exactly as they are: its other consumer is
    /// `nir_call_is_infallible_builtin` (`engine/control/builder_control.rs`),
    /// which types a builtin's arguments to decide whether the call can fail, so
    /// answering here for an expression it used to give up on changes which
    /// failure paths the optimizer may elide — in programs that contain no
    /// self-update at all. That is a separate change with its own risk and its
    /// own measurement; it does not belong in a fix to a collection gate.
    pub(crate) fn static_item_type(&self, value: &NirValue) -> Option<ParameterType> {
        if let Some(type_) = self.static_type_name(value) {
            return Some(type_);
        }
        self.static_call_type(value, &|builder, arg| builder.static_item_type(arg))
    }

    /// The static type of a self-update's **operand** — the item appended, the
    /// element added or removed, the key removed — for the `G11` gates, and for
    /// nothing else.
    ///
    /// bug-681: [`Self::static_item_type`]'s widening applied only at the value's
    /// own top level. A `Binary`, `Unary`, `MemberAccess` or `ResultValue` around
    /// the call was matched by `static_type_name`, whose composite arms recurse
    /// into *itself*, so the call underneath hit the builtin-name table and
    /// answered `None` — `G11` declined and the statement rebuilt the whole
    /// collection. One `* 2` was the difference between `append(xs, f(i))` (in
    /// place, 1 ms at n=16000) and `append(xs, f(i) * 2)` (2798 ms and quadratic;
    /// a 259,920-element decode was killed by the OOM killer), and it was never
    /// `append`-specific: `add`, `remove` and `removeKey` all lost the path the
    /// same way.
    ///
    /// So the composite arms are re-run here — via
    /// [`Self::static_composite_type`], the one implementation `static_type_name`
    /// also uses — with the recursion pointed back at this query, so every leaf
    /// gets the same call lookups the top level gets. **Widen this one, not its
    /// two parents**: `static_type_name` also gates float-arithmetic lowering and
    /// `is_function_value`, and `static_item_type` also feeds the
    /// infallible-builtin oracle; this query has exactly six callers, all of them
    /// `G11` (`grep -n static_operand_type src/codegen/collection/assign/`).
    ///
    /// It is a widening of the *query*, never of a gate: every caller still
    /// compares the answer for equality with the element, key or collection type,
    /// which is what keeps the single-element and bulk `append` arms apart.
    pub(crate) fn static_operand_type(&self, value: &NirValue) -> Option<ParameterType> {
        if let Some(type_) = self.static_type_name(value) {
            return Some(type_);
        }
        if let Some(type_) =
            self.static_call_type(value, &|builder, arg| builder.static_operand_type(arg))
        {
            return Some(type_);
        }
        self.static_composite_type(value, &|builder, operand| {
            builder.static_operand_type(operand)
        })
    }

    /// The declared return type of a call — a user (or `LINK`) function's
    /// `returns`, a package's, else the registry resolver over argument types
    /// `arg_type` supplies. `None` for any value that is not a call.
    ///
    /// Split out of [`Self::static_item_type`] (bug-681) so
    /// [`Self::static_operand_type`] performs the same lookup with its own,
    /// wider, argument query rather than a copy of this one.
    fn static_call_type(
        &self,
        value: &NirValue,
        arg_type: &dyn Fn(&Self, &NirValue) -> Option<ParameterType>,
    ) -> Option<ParameterType> {
        match value {
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => {
                if let NirValue::Call { .. } = value {
                    if let Some(type_) = self
                        .functions
                        .get(target)
                        .map(|function| function.returns.clone())
                        .or_else(|| self.package_return_types.get(target).cloned())
                    {
                        return Some(type_);
                    }
                }
                let arg_types = args
                    .iter()
                    .map(|arg| arg_type(self, arg))
                    .collect::<Option<Vec<_>>>()?;
                builtins::resolve_call_return_type_typed(target, &arg_types, false)
            }
            _ => None,
        }
    }

    /// The static type of a **composite** value — one whose type is derived from
    /// its operands' — with `operand_type` the query used for those operands.
    ///
    /// Split out of [`Self::static_type_name`] (bug-681) so the one set of
    /// derivation rules serves both queries: `static_type_name` passes itself and
    /// keeps its exact answers, while [`Self::static_operand_type`] passes
    /// *itself* and so reaches the user-`FUNC` and registry call lookups from
    /// inside a `Binary`, `Unary`, `MemberAccess` or `ResultValue`. Answers
    /// `None` for any value that is not composite — the caller has already
    /// handled the leaves. A new composite `NirValue` belongs here, so the two
    /// queries cannot drift apart again.
    fn static_composite_type(
        &self,
        value: &NirValue,
        operand_type: &dyn Fn(&Self, &NirValue) -> Option<ParameterType>,
    ) -> Option<ParameterType> {
        match value {
            NirValue::ResultValue { value } => match operand_type(self, value)? {
                // A non-`Result` operand answers with its own type, as the
                // `strip_prefix(…).or_else(…)` this replaces did.
                ParameterType::ResultOf(success) => Some(*success),
                other => Some(other),
            },
            NirValue::Binary {
                op, left, right, ..
            } => {
                if op.is_comparison() || matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                {
                    return Some(ParameterType::Boolean);
                }
                if *op == BinaryOp::Concat {
                    return Some(ParameterType::String);
                }
                let left = operand_type(self, left)?;
                let right = operand_type(self, right)?;
                Some(promoted_binary_type(*op, &left, &right))
            }
            NirValue::Unary { op, operand, .. } => {
                if *op == UnaryOp::Not {
                    Some(ParameterType::Boolean)
                } else {
                    operand_type(self, operand)
                }
            }
            NirValue::MemberAccess { target, member } => {
                let target_type = operand_type(self, target)?;
                self.member_type_of(&target_type, member)
            }
            _ => None,
        }
    }

    pub(crate) fn static_type_name(&self, value: &NirValue) -> Option<ParameterType> {
        match value {
            NirValue::Const { type_, .. } => Some(type_.clone()),
            NirValue::Local(name) => self.locals.get(name).map(|local| local.type_.clone()),
            NirValue::LocalRef { type_, .. } => Some(type_.clone()),
            NirValue::Global { name, type_ } => {
                if is_unset_type(type_) {
                    self.globals.get(name).map(|global| global.type_.clone())
                } else {
                    Some(type_.clone())
                }
            }
            NirValue::FunctionRef { type_, .. }
            | NirValue::Closure { type_, .. }
            | NirValue::Capture { type_, .. }
            | NirValue::Constructor { type_, .. }
            | NirValue::WithUpdate { type_, .. }
            | NirValue::ListLiteral { type_, .. }
            | NirValue::SetLiteral { type_, .. }
            | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
            NirValue::UnionWrap { union_type, .. } => Some(union_type.clone()),
            NirValue::UnionExtract { type_, .. } => Some(type_.clone()),
            NirValue::ResultIsOk { .. } => Some(ParameterType::Boolean),
            NirValue::ResultError { .. } => Some(error_type()),
            // bug-471: the success type, matching the `CallResult` arm below.
            NirValue::Checked { type_, .. } => Some(type_.clone()),
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => match target.as_str() {
                "replace" | "typeName" | "toString" => Some(ParameterType::String),
                "find" | "len" | "toInt" => Some(ParameterType::Integer),
                "mid" => Some(ParameterType::String),
                "toFloat" => Some(ParameterType::Float),
                "toFixed" => Some(ParameterType::Fixed),
                "toByte" => Some(ParameterType::Byte),
                "toMoney" => Some(ParameterType::Money),
                "toScalar" => Some(scalar_type()),
                "isNumeric" => Some(ParameterType::Boolean),
                // A list element read resolves to the list's element type, so an
                // append/set whose item is a `get` (or arithmetic over `get`s)
                // stays on the allocation-free in-place fast path instead of the
                // value-semantic fallback that copies the whole list each op
                // (bug-01). Only lists are resolved here (map value reads fall
                // through to the conservative `None`).
                "get" | "getOr" | "collections.get" | "collections.getOr" => args
                    .first()
                    .and_then(|arg| self.static_type_name(arg))
                    .and_then(|type_| match type_ {
                        ParameterType::ListOf(element) => Some(*element),
                        _ => None,
                    }),
                "math.floor" | "math.ceil" | "math.round" => Some(ParameterType::Integer),
                "math.sqrt" | "math.exp" | "math.log" | "math.log10" | "math.sin" | "math.cos"
                | "math.tan" | "math.asin" | "math.acos" | "math.atan" => {
                    args.first().and_then(|arg| self.static_type_name(arg))
                }
                "math.pow" | "math.atan2" => {
                    args.first().and_then(|arg| self.static_type_name(arg))
                }
                // bug-630: a thread read can take its handle inline
                // (`thread::waitFor(thread::start(…))`) instead of through a
                // binding. Without this arm the nested `start` had no static
                // type, so `thread_runtime_return_type` could not reach the
                // `ThreadHandle` match that yields the worker's `Out` — and the
                // parent/worker direction split for `receive`/`accept` failed
                // outright ("handle has unknown type"). Delegate to the one
                // `thread.start` arm rather than copying its parent-kind
                // derivation, so bug-479's `worker: false` correction stays in a
                // single place.
                "thread.start" => self.thread_runtime_return_type("thread.start", args),
                _ => None,
            },
            // The composite arms (`ResultValue`, `Binary`, `Unary`,
            // `MemberAccess`) derive their type from their operands'; the
            // derivation lives in one place (bug-681) and here recurses into this
            // same query, so every answer is exactly what it was.
            _ => self.static_composite_type(value, &|builder, operand| {
                builder.static_type_name(operand)
            }),
        }
    }

    /// The type of `member` read off a value of `target_type`: the thread
    /// handle's `result`, a record or union-variant field, or a typed map
    /// entry's `key`/`value`. Split out of [`Self::static_type_name`] so
    /// [`Self::overload_arg_type`] can apply the same lookup to a target that
    /// only IT can type (bug-497: `makeRec().body`).
    fn member_type_of(&self, target_type: &ParameterType, member: &str) -> Option<ParameterType> {
        if member == "result" {
            if let ParameterType::ThreadHandle {
                worker: false, out, ..
            } = target_type
            {
                return Some(ParameterType::result_of((**out).clone()));
            }
        }
        // A record or union-variant field, read from the same tables the
        // field-access lowering itself uses. Without this arm the builder
        // could not name the type of `rec.field`, so `typeName(rec.field)`
        // failed to lower at all with "cannot determine typeName argument
        // type" (bug-366).
        // The two field tables are keyed by nominal type NAME, so the
        // lookup renders the (scalar-cheap) name — a name-keyed table
        // probe, not a type-string derivation.
        let owner = target_type.name();
        let field_type = self
            .type_model
            .record_fields
            .get(&ParameterType::declared(owner.as_ref()))
            .or_else(|| {
                self.type_model
                    .union_variant_fields
                    .get(&ParameterType::declared(owner.as_ref()))
            })
            .and_then(|fields| {
                fields
                    .iter()
                    .find(|(name, _)| name == member)
                    .map(|(_, type_)| type_.clone())
            });
        if field_type.is_some() {
            return field_type;
        }
        let (key_type, value_type) = typed_map_entry_type_parts(target_type)?;
        match member {
            "key" => Some(key_type.clone()),
            "value" => Some(value_type.clone()),
            _ => None,
        }
    }

    /// Static type of `value`, resolving builtin calls the hand-written
    /// [`Self::static_type_name`] table misses by delegating to the authoritative
    /// per-package resolvers (`builtins::resolve_call_return_type`).
    ///
    /// This is used **only** for the `typeName` compile-time fold (bug-354).
    /// `typeName` must fold its argument's type to a string constant; before this,
    /// `static_type_name`'s table named zero `strings.*`, so `typeName` of every
    /// `strings::` call — plus `math::abs/min/max` and
    /// `collections::find/contains/hasKey` — failed to lower on valid source.
    ///
    /// It deliberately does NOT widen `static_type_name` itself: that function
    /// also gates the in-place-append fast path, numeric-result typing, and the
    /// slice specialization, and widening it would shift their codegen for every
    /// program using these builtins (including inside inlined package bodies). The
    /// fold is the only place a table miss is a hard error, so the resolver
    /// fallback is scoped to exactly here. Recurses through itself so nested calls
    /// (`typeName(strings::upper(strings::lower(s)))`) resolve too.
    pub(crate) fn static_type_name_for_fold(&self, value: &NirValue) -> Option<ParameterType> {
        if let Some(type_) = self.static_type_name(value) {
            return Some(type_);
        }
        match value {
            // bug-874: `Color.Green` names a member of an enum TYPE — its target is the
            // type's name, not a binding, so the operand walk above has no local to
            // type. The member's type is the enum itself, the same table
            // `lower_match_compare` resolves it through.
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                NirValue::Local(type_name) if !self.locals.contains_key(type_name) => self
                    .type_model
                    .enum_members
                    .keys()
                    .find(|(enum_type, name)| name == member && enum_type.name() == *type_name)
                    .map(|(enum_type, _)| enum_type.clone()),
                _ => None,
            },
            NirValue::Call { target, args, .. }
            | NirValue::CallResult { target, args, .. }
            | NirValue::RuntimeCall { target, args, .. } => {
                // bug-672: a call to a module function has its declared return
                // type. Builtin resolution alone left `typeName(pick(1))` unfoldable,
                // a build error on valid source. A callable local of the same name
                // shadows the function, as at the call itself (bug-569).
                if let Some(local) = self.locals.get(target.as_str()) {
                    return match &local.type_ {
                        ParameterType::Func(_, returns, _) => Some((**returns).clone()),
                        _ => None,
                    };
                }
                if let Some(function) = self.functions.get(target.as_str()) {
                    return Some(function.returns.clone());
                }
                let arg_types = args
                    .iter()
                    .map(|arg| self.static_type_name_for_fold(arg))
                    .collect::<Option<Vec<_>>>()?;
                builtins::resolve_call_return_type_typed(target, &arg_types, false)
            }
            _ => None,
        }
    }

    /// Static type of a runtime-call ARGUMENT, for the code-form (overload)
    /// selection in [`Self::lower_runtime_helper_call`].
    ///
    /// bug-476: several native members collapse two overloads into one name and
    /// choose the *lowering* here, from an argument's static type —
    /// `tcp::write`/`tls::write`/`udp::send` (bytes vs text),
    /// `tcp::connect`/`tls::connect` (host/port vs `Address`), `net::ping`,
    /// `tcp`/`udp`/`tls` `poll` (scalar vs list), `tls::localAddress`
    /// (`Socket` vs `Listener`) and `io::print`'s `AttributedString` rewrite.
    /// [`Self::static_type_name`]'s `NirValue::Call` arm is a hand-written table
    /// of a dozen builtins, so **any other call answered `None`** and every one
    /// of those selectors silently took its fallback form. For `tcp::write` that
    /// meant `tcp::write(sock, buildHead(x))` marshalling a `String*` through the
    /// collection path: a garbage element count, a failed `write(2)`, and an
    /// `ErrConnectionClosed` raised with nothing on the wire — which is how
    /// `http::handleRequest` came to serve an empty reply to every request.
    ///
    /// The miss is only in the *call* arm, and only user/package functions and
    /// untabulated builtins are added: a call is resolved against the same
    /// return-type tables `emit_call` uses (the NIR function set, then the
    /// package return types), falling back to
    /// [`Self::static_type_name_for_fold`]'s registry resolver. It deliberately
    /// does NOT widen `static_type_name` itself — that also gates the in-place
    /// append/set fast path, numeric-result typing and the slice specialisation,
    /// where naming more call results changes codegen (and, for
    /// `x = collections::append(x, f())`, the aliasing decision) far outside an
    /// overload choice.
    ///
    /// bug-497 widened the call arm past named functions to a call through a
    /// FUNC-typed value, and added the `MemberAccess`/`ResultValue` arms, after
    /// `tcp::write(sock, f(x))` and `tcp::write(sock, makeRec().body)` were
    /// measured still taking the byte-list form — the same peer-controlled
    /// out-of-bounds read the named-function case was (OS-50). The write
    /// selectors now refuse an unresolved payload outright
    /// (`net_write_payload_form`), so a shape this cannot type is a build error.
    pub(crate) fn overload_arg_type(&self, value: &NirValue) -> Option<ParameterType> {
        if let Some(type_) = self.static_type_name(value) {
            return Some(type_);
        }
        match value {
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => {
                if let Some(type_) = self
                    .functions
                    .get(target.as_str())
                    .map(|function| function.returns.clone())
                    .or_else(|| self.package_return_types.get(target.as_str()).cloned())
                {
                    return Some(type_);
                }
                // bug-497: a call THROUGH a FUNC-typed value — `LET f AS
                // FUNC(String) AS String = reply` then `f(x)` — is a `Call` whose
                // target is the VALUE's name, so neither function table above
                // knows it. Its declared type carries the return type, exactly
                // as a named function's `returns` does.
                if let Some(ParameterType::Func(_, returns, _)) = self
                    .locals
                    .get(target.as_str())
                    .map(|local| local.type_.clone())
                    .or_else(|| {
                        self.globals
                            .get(target.as_str())
                            .map(|global| global.type_.clone())
                    })
                {
                    return Some(*returns);
                }
                self.static_type_name_for_fold(value)
            }
            // bug-497: `makeRec().body` — a field read off a call result.
            // `static_type_name`'s own arm gives up because it cannot type the
            // call; type the target with THIS resolver and reuse its field lookup.
            NirValue::MemberAccess { target, member } => {
                let target_type = self.overload_arg_type(target)?;
                self.member_type_of(&target_type, member).or_else(|| {
                    // `res.state` on a STATE-carrying resource (or resource
                    // union). `member_type_of` knows only record/union-variant
                    // fields, a thread handle's `result`, and a typed map's
                    // key/value — so every payload reached THROUGH the STATE
                    // block (`client.state.raw`) was unresolved, and the
                    // fail-closed write selector this bug added then refused a
                    // VALID program at build time:
                    //   error: native runtime tls.write: payload static type
                    //   <unresolved> is neither String nor List OF Byte
                    // (`tests/rt_macos_d4_union_state_tls.rs`). The lowering
                    // itself has always known this type — `emit_member_access`
                    // reads it off `type_.state()` — so only the static side
                    // was missing.
                    //
                    // Resolved HERE and not in `member_type_of`, because that
                    // is shared with `static_type_name`, which also gates the
                    // in-place append/set fast path: typing `client.state.raw`
                    // there would retype
                    // `client.state.raw = collections::append(client.state.raw, …)`
                    // and shift that statement's codegen and aliasing decision,
                    // far outside an overload choice.
                    if member == "state" {
                        target_type.state()
                    } else {
                        None
                    }
                })
            }
            NirValue::ResultValue { value } => match self.overload_arg_type(value)? {
                ParameterType::ResultOf(success) => Some(*success),
                other => Some(other),
            },
            _ => None,
        }
    }

    pub(crate) fn thread_runtime_return_type(
        &self,
        target: &str,
        args: &[NirValue],
    ) -> Option<ParameterType> {
        match target {
            // `thread::start` returns the PARENT handle. Its channel slots are
            // the worker entry's first parameter — plan-106-E: the isolated-FUNC
            // spelling is a variant, so the parameter is read off it instead of
            // being re-split out of `ISOLATED FUNC(` … `) AS ` — but the KIND is
            // the parent's, not the worker's.
            //
            // bug-479: this used to hand back that parameter verbatim, i.e. a
            // `ThreadWorker`. `registry/unify.rs` (`unify_at`) states the two never interchange
            // ("kind (parent `Thread` vs worker `ThreadWorker`) must match"), and
            // the descriptor agrees — `th(false, …)`. The wrong kind was invisible
            // for a plain call, because both spellings denote the same one-pointer
            // handle and nothing asks the type a question; it surfaced only under
            // an inline `TRAP`, where `materialize_current_result` marshals the
            // `Result`'s success payload BY TYPE and reported
            //   native inlined field size not available for type
            //   'ThreadWorker OF String TO Integer'
            // while lowering `bind $trap_res0 AS Result OF Thread OF String TO
            // Integer` — the two types in that message being different is the
            // whole clue.
            "thread.start" => match self.static_type_name(args.first()?)? {
                // plan-156-E: the worker's view STATE is its own; the parent's handle
                // is built from the channels alone.
                ParameterType::Func(params, _, true) => match &params.first()?.without_state() {
                    ParameterType::ThreadHandle { msg, res, out, .. } => {
                        Some(ParameterType::thread_handle(
                            false,
                            (**msg).clone(),
                            (**res).clone(),
                            (**out).clone(),
                        ))
                    }
                    // Not a thread handle: the entry is ill-formed and the
                    // frontend rejects it. Decline rather than invent a type.
                    _ => None,
                },
                _ => None,
            },
            "thread.isRunning" | "thread.poll" | "thread.isCancelled" => {
                Some(ParameterType::Boolean)
            }
            "thread.cancel"
            | "thread.send"
            | "thread.transferResource"
            | "thread.emitResource"
            | "thread.openStdIn"
            | "thread.closeStdIn" => Some(ParameterType::Nothing),
            // plan-111-E: the three thread planes are `ThreadHandle` fields, so
            // each is read off the variant rather than re-split from a spelling
            // by `parent_thread_output` / `thread_message` / `thread_resource`.
            "thread.waitFor" => match self.thread_handle_arg_type(args)? {
                ParameterType::ThreadHandle {
                    worker: false, out, ..
                } => Some((*out).clone()),
                _ => None,
            },
            "thread.receive" => match self.thread_handle_arg_type(args)? {
                ParameterType::ThreadHandle { msg, .. } => Some((*msg).clone()),
                _ => None,
            },
            // The resource plane: accept yields the thread's resource type
            // (worker reads the inbound queue, parent reads the outbound queue).
            "thread.acceptResource" | "thread.readResource" => {
                match self.thread_handle_arg_type(args)? {
                    ParameterType::ThreadHandle { res, .. } => Some((*res).clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The static type of a thread op's handle argument, `RES` marker and view `STATE`
    /// stripped: a view read out of a `RES` record field or collection element is typed
    /// `RES Thread OF …` (plan-156-D), a view may carry its own STATE (plan-156-E), and
    /// every thread op answers by the `ThreadHandle` inside.
    fn thread_handle_arg_type(&self, args: &[NirValue]) -> Option<ParameterType> {
        let type_ = match self.static_type_name(args.first()?)? {
            ParameterType::Res(inner) => *inner,
            other => other,
        };
        Some(type_.without_state())
    }
}
