use super::*;

impl TypeEnv {
    // 10. Call arity/arg types, thread + STATE agreement
    // ===========================================================================

    /// The unary counterpart of `check_binary_operands` (`syntaxcheck`'s
    /// `infer_unary` / `TYPE_UNARY_OPERATOR_MISMATCH`): `NOT` requires a Boolean
    /// operand, unary `-` a numeric one. Same memory-safety rationale — codegen
    /// picks the instruction from the operand type. `Unknown` never rejects.
    pub(super) fn check_unary_operand(
        &self,
        op: &str,
        operand: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let Some(t) = self.infer_type(operand, locals) else {
            return;
        };
        match op {
            "NOT" => {
                if !matches!(t.as_str(), "Boolean" | "Unknown") {
                    self.emit(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        format!("Operator `NOT` requires a Boolean operand, got {t}."),
                    );
                }
            }
            "-" => {
                if !matches!(
                    t.as_str(),
                    "Integer" | "Byte" | "Float" | "Fixed" | "Money" | "Unknown"
                ) {
                    self.emit(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        format!("Unary `-` requires a numeric operand, got {t}."),
                    );
                }
            }
            other => {
                self.emit(
                    "TYPE_UNARY_OPERATOR_UNKNOWN",
                    format!("Unknown unary operator `{other}`."),
                );
            }
        }
    }

    /// Reject a direct call whose argument count cannot match the callee's
    /// signature. Only internal functions have a known signature; builtins,
    /// runtime helpers, imports and indirect (function-typed local) calls are
    /// skipped.
    pub(super) fn check_call_arity(
        &self,
        target: &str,
        argc: usize,
        locals: &HashMap<String, String>,
    ) {
        // Calling something that is not a function — syntaxcheck's
        // SYMBOL_NOT_CALLABLE: a package constant (`math.pi()`), or a local
        // binding/parameter of a known non-function type.
        if builtins::is_package_constant(target) {
            self.emit(
                "SYMBOL_NOT_CALLABLE",
                format!("Package constant `{target}` is not callable."),
            );
            return;
        }
        if let Some(t) = locals.get(target) {
            // A local of FUNC type is an indirect call; its arity is the
            // function type's, not a named signature. Any other *known* local
            // type is not callable at all.
            if !t.is_empty() && t != "Unknown" && !t.starts_with("FUNC") {
                self.emit(
                    "SYMBOL_NOT_CALLABLE",
                    format!("Local binding or parameter `{target}` is not callable."),
                );
            }
            return;
        }
        let Some(sig) = self.functions.get(target) else {
            return;
        };
        let required = sig.total.saturating_sub(sig.optional);
        if argc < required || argc > sig.total {
            self.emit(
                "TYPE_CALL_ARITY_MISMATCH",
                format!(
                    "Call to `{target}` has {argc} argument(s), expected {required}..={}.",
                    sig.total
                ),
            );
        }
    }

    /// Reject a call to a known user function whose argument types are
    /// incompatible with the declared parameter types (`syntaxcheck`'s
    /// `TYPE_CALL_ARGUMENT_MISMATCH`). On decoded package IR this is an ABI-level
    /// type confusion: codegen marshals each argument by its declared parameter
    /// type, so a crafted `String` passed where an `Integer` is expected is read
    /// as an integer at the callee boundary. Lowering has already normalized the
    /// call (positional, defaults filled, union members wrapped), so a direct
    /// arg-type-vs-param-type comparison is faithful. `Unknown` never rejects.
    pub(super) fn check_call_argument_types(
        &self,
        target: &str,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        if locals.contains_key(target) {
            return; // indirect call — no named signature
        }
        let Some(sig) = self.functions.get(target) else {
            return;
        };
        for (index, arg) in args.iter().enumerate() {
            let Some(param_type) = sig.params.get(index) else {
                break;
            };
            let Some(actual) = self.infer_type(arg, locals) else {
                continue;
            };
            self.check_argument_state_agreement(target, index, param_type, &actual);
            // Strip a resource argument's `STATE T` clause; the parameter type
            // is the bare resource type.
            let actual = resource_base_type(&actual).to_string();
            let param_type = resource_base_type(param_type);
            self.check_literal_range(param_type, arg);
            if !self.expression_compatible(param_type, &actual, arg) {
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Argument {} for `{target}` has type {actual}, expected {param_type}.",
                        index + 1
                    ),
                );
            }
        }
    }

    /// Reject a `thread::transfer` whose transferred resource's `STATE` disagrees
    /// with the thread plane's declared element `STATE` (`TYPE_STATE_MISMATCH`,
    /// plan-54 — closes bug-257).
    ///
    /// A transfer is a **move to a re-typer**: the accepting thread re-declares the
    /// resource type (`RES f AS File STATE Cursor = thread::accept(t)`), and the
    /// STATE payload carries no runtime tag, so its type comes entirely from
    /// whichever type string each side holds. Unlike a parameter — a non-escaping
    /// alias, where bare reads as "opaque" and accepts any state — the transfer
    /// escapes the frame, so the plane and the transferred resource must name the
    /// **same** state. Both bare is agreement; every disagreement (a stateful
    /// resource on a bare plane, a bare resource on a stateful plane, or two
    /// different states) is the cross-thread confusion bug-257 demonstrated: a
    /// `Cursor{pos:Integer}` sent, read as a `Label{name:String}`.
    ///
    /// This mirrors the escape rule (`mfb spec language resource-management`
    /// §15.5): a transfer is an escape position, so STATE must be in the contract —
    /// here, the plane type. The check runs on the lowered `transferResource` call
    /// (arg 0 = the thread handle whose type carries the plane STATE, arg 1 = the
    /// transferred resource).
    pub(super) fn check_thread_transfer_state(
        &self,
        target: &str,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        if target != crate::builtins::thread::TRANSFER_RESOURCE {
            return;
        }
        let (Some(handle), Some(resource)) = (args.first(), args.get(1)) else {
            return;
        };
        let (Some(handle_type), Some(resource_type)) = (
            self.infer_type(handle, locals),
            self.infer_type(resource, locals),
        ) else {
            return;
        };
        let Some(plane_resource) = crate::builtins::thread::thread_resource(&handle_type) else {
            return;
        };
        let plane_state = crate::builtins::resource::state_type_name(plane_resource);
        let resource_state = crate::builtins::resource::state_type_name(&resource_type);
        if plane_state == resource_state {
            return; // both bare, or the same state — the agreeing case.
        }
        let detail = match (plane_state, resource_state) {
            (Some(plane), Some(actual)) => format!(
                "carries STATE `{actual}` but the thread plane declares `STATE {plane}`; a transfer moves the resource to a thread that re-types it, so both must name the same state"
            ),
            (Some(plane), None) => format!(
                "carries no STATE but the thread plane declares `STATE {plane}`; the accepting thread would read an unattached state"
            ),
            (None, Some(actual)) => format!(
                "carries STATE `{actual}` but the thread plane is bare; a bare plane asserts the resource has no state — declare the plane `RES {} STATE {actual}`",
                crate::builtins::resource::base_resource_name(plane_resource)
            ),
            // Equal (both None) is handled above; unreachable.
            (None, None) => return,
        };
        self.emit(
            "TYPE_STATE_MISMATCH",
            format!("`thread::transfer` {detail}."),
        );
    }

    /// Reject a `RES` parameter whose declared `STATE` disagrees with the state
    /// its argument actually carries (`TYPE_STATE_MISMATCH`, plan-52-C).
    ///
    /// A resource's STATE type is fixed at its **owning binding**; parameters only
    /// observe. Nothing checked this, so a parameter could **attach** a payload to
    /// a stateless resource, or **re-type** one it should only read — and the
    /// payload carries no runtime type tag, so its type comes entirely from
    /// whichever type string the reader holds. A `Cursor{pos:Integer}` read through
    /// a `STATE Label{name:String}` parameter interprets the integer as a String
    /// header. That is statically decidable from the two type strings already in
    /// hand here, and not checkable at runtime at all.
    ///
    /// The table (`mfb spec language resource-management` §15.5):
    ///
    /// | argument     | param `STATE T` | param bare |
    /// |--------------|-----------------|------------|
    /// | carries `T`  | ✓               | ✓          |
    /// | carries `T2` | ✗               | ✓          |
    /// | stateless    | ✗               | ✓          |
    ///
    /// **A bare parameter accepts anything and this must stay that way.** Bare
    /// reads as "opaque" at a parameter — sound because a non-owning pointer cannot escape the
    /// frame that took it — and every close op depends on it: `FUNC close(RES db AS
    /// Db)` names no STATE and must accept a `Db` whatever its owner attached.
    /// Tightening bare to stateless-only would break every one of them.
    ///
    /// Note the intuitive rule is the unsafe one: allowing `stateless → STATE T`
    /// so a parameter may attach is precisely what makes two disagreeing state types
    /// reachable with **no stateful binding anywhere** — `a(RES p AS File STATE
    /// Cursor)` allocates, then `b(RES p AS File STATE Label)` reads that block as
    /// a Label.
    pub(super) fn check_argument_state_agreement(
        &self,
        target: &str,
        index: usize,
        param_type: &str,
        actual: &str,
    ) {
        let Some(param_state) = crate::builtins::resource::state_type_name(param_type) else {
            return; // bare parameter: the opt-out — any state or none.
        };
        let arg_state = crate::builtins::resource::state_type_name(actual);
        if arg_state == Some(param_state) {
            return;
        }
        let detail = match arg_state {
            Some(arg_state) => format!(
                "carries STATE `{arg_state}`; a parameter observes a resource's state, it cannot re-type it"
            ),
            None => format!(
                "carries no STATE; a parameter cannot attach one — declare `STATE {param_state}` on the owning binding"
            ),
        };
        self.emit(
            "TYPE_STATE_MISMATCH",
            format!(
                "Argument {} for `{target}` is declared `STATE {param_state}` but {detail}.",
                index + 1
            ),
        );
    }

    /// Apply the STATE payload-type rules to a **declared return**: the state type
    /// must be defaultable (`TYPE_STATE_INVALID`) and its base must not be a
    /// resource union (`TYPE_UNION_STATE_FORBIDDEN`) — the same two rules the
    /// binding position has always enforced, since they are properties of the
    /// state type itself and do not care which position declares it.
    ///
    /// These were unreachable from a return for a subtle reason worth recording:
    /// the binding rules pattern-match `" STATE "` in a type string, and the return
    /// type string never contained it (plan-52-D restored that append). But the
    /// append alone does **not** make them fire — they run over `IrOp::Bind`, and a
    /// function's return is not a binding. The same omission that rejected the
    /// legal stateful `RETURN` also hid these two, and each needs its own fix.
    pub(super) fn check_return_state_declaration(&self, function: &IrFunction) {
        let Some(state_type) = crate::builtins::resource::state_type_name(&function.returns) else {
            return;
        };
        let base = resource_base_type(&function.returns);
        if self.unions.contains_key(base) {
            self.emit(
                "TYPE_UNION_STATE_FORBIDDEN",
                format!(
                    "FUNC `{}` returns resource union `{base}` with STATE `{state_type}`; a resource union carries no STATE — use a concrete stateful resource.",
                    function.name
                ),
            );
        }
        if !self.is_defaultable(state_type, &mut HashSet::new()) {
            self.emit(
                "TYPE_STATE_INVALID",
                format!(
                    "FUNC `{}` return STATE type `{state_type}` must be a copyable, defaultable data type.",
                    function.name
                ),
            );
        }
    }

    /// Reject a **bare** `RES` binding of a value that carries a `STATE`
    /// (`TYPE_STATE_MISMATCH`, plan-52-D Phase 2).
    ///
    /// A bare binding **erases** the STATE from the type string, which is the
    /// laundering primitive: once returns carry their STATE, the erasure would
    /// defeat the return check itself —
    ///
    /// ```basic
    /// FUNC launder() AS RES SfFile             ' promises "no state"
    ///   RES tmp AS SfFile = openStateful()     ' bare bind of a stateful value
    ///   RETURN tmp                             ' expected SfFile, actual SfFile -> accepted
    /// END FUNC
    /// RES g AS SfFile STATE Cursor = launder() ' attaches a Cursor over a live FileInfo
    /// ```
    ///
    /// so `launder` would hand back a resource secretly carrying a `FileInfo`, and
    /// the caller's `STATE Cursor` binding would alias it — the bare return's "no
    /// state" promise is what a later attach relies on. This rule is unreachable
    /// before the return append (nothing could produce a stateful resource from a
    /// call), and reachable the moment it lands: the two ship together, never apart.
    ///
    /// The mirror of the parameter rule, and note it goes the OTHER way:
    ///
    /// | initializer  | binding `STATE T`            | binding bare |
    /// |--------------|------------------------------|--------------|
    /// | carries `T`  | ✓ (adopts)                   | ✗            |
    /// | carries `T2` | ✗                            | ✗            |
    /// | stateless    | ✓ **the one true attach point** | ✓         |
    ///
    /// `stateful → bare` is safe for a **parameter** (a non-owning pointer cannot escape the
    /// frame, so forgetting the state is unobservable) and unsafe for a **binding**
    /// (an owner escapes). Yes for params, no for owners — the escape distinction
    /// is the whole rule.
    /// plan-59-C: is `value` a direct read of a bare `RES` parameter — i.e. a value
    /// whose `STATE` is **opaque** ("some state or none") rather than known-absent?
    ///
    /// Deliberately narrow: only a direct `Var` read counts. Anything that has
    /// passed through a call has that call's declared return type, which names its
    /// `STATE` (or names none) and is checked on its own terms. Widening this to a
    /// dataflow analysis would be the whole-program aliasing analysis §3 rejects.
    pub(super) fn is_opaque_state_value(&self, value: &IrValue) -> bool {
        matches!(value, IrValue::Local(name)
            if self.current_opaque_params.borrow().contains(name.as_str()))
    }

    pub(super) fn check_binding_state_agreement(
        &self,
        name: &str,
        type_: &str,
        value: &Option<IrValue>,
        locals: &HashMap<String, String>,
    ) {
        let Some(value) = value else {
            return;
        };
        // plan-59-C: binding a bare `RES` parameter under a CONCRETE `STATE` is an
        // unprovable narrowing — the checker knows only that it carries *some*
        // state. Checked before the agreement arms below, which cannot see it: an
        // opaque value's type string names no STATE, so `state_type_name` returns
        // `None` and it would otherwise be treated as provably stateless and
        // silently adopt the declared type.
        if self.is_opaque_state_value(value) {
            if let Some(declared) = crate::builtins::resource::state_type_name(type_) {
                self.emit(
                    "TYPE_STATE_OPAQUE_NARROWING",
                    format!(
                        "binding `{name}` declares `STATE {declared}`, but its initializer is a bare `RES` parameter whose STATE is opaque — it carries some state or none, and the compiler cannot prove it is a `{declared}`."
                    ),
                );
                return;
            }
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        let Some(value_state) = crate::builtins::resource::state_type_name(&actual) else {
            return; // stateless initializer: attach (or stay bare) — both legal.
        };
        match crate::builtins::resource::state_type_name(type_) {
            // Adopting the state it already carries — the agreeing case.
            Some(declared) if declared == value_state => {}
            Some(declared) => self.emit(
                "TYPE_STATE_MISMATCH",
                format!(
                    "binding `{name}` declares `STATE {declared}` but its initializer carries STATE `{value_state}`; a resource's STATE type is fixed where it is created."
                ),
            ),
            None => self.emit(
                "TYPE_STATE_MISMATCH",
                format!(
                    "binding `{name}` is bare but its initializer carries STATE `{value_state}`; a bare binding asserts the resource has no state — declare `STATE {value_state}`."
                ),
            ),
        }
    }

    // ===========================================================================
}
