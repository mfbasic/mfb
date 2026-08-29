use super::compat::builtin_call_target;
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
        locals: &HashMap<String, ParameterType>,
    ) {
        let Some(t) = self.infer_type(operand, locals) else {
            return;
        };
        match op {
            "NOT" => {
                if !matches!(t, ParameterType::Boolean | ParameterType::Unknown) {
                    let t = t.name();
                    self.emit(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        format!("Operator `NOT` requires a Boolean operand, got {t}."),
                    );
                }
            }
            "-" => {
                if !matches!(
                    t,
                    ParameterType::Integer
                        | ParameterType::Byte
                        | ParameterType::Float
                        | ParameterType::Fixed
                        | ParameterType::Money
                        | ParameterType::Unknown
                ) {
                    let t = t.name();
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
    /// signature (`TYPE_CALL_ARITY_MISMATCH`). Three callee classes
    /// (plan-107-E): a function value (a local or global of FUNC type) takes
    /// exactly its type's parameter count; a builtin is `check_builtin_call_args`'s;
    /// a declared function's count is checked here only on the package path —
    /// on the source path lowering has already normalized the argument list
    /// (extras dropped, defaults filled), so the count the source wrote is
    /// `ir::shape`'s to check.
    pub(super) fn check_call_arity(
        &self,
        target: &str,
        argc: usize,
        locals: &HashMap<String, ParameterType>,
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
            if let ParameterType::Func(params, _, _) = t {
                self.check_function_value_arity(target, argc, params.len());
                return;
            }
            if !t.name().is_empty() && !matches!(t, ParameterType::Unknown) {
                self.emit(
                    "SYMBOL_NOT_CALLABLE",
                    format!("Local binding or parameter `{target}` is not callable."),
                );
            }
            return;
        }
        let Some(sig) = self.functions.get(target) else {
            // A global binding holding a function value is callable like a
            // local one (bug-198).
            if let Some(ParameterType::Func(params, _, _)) = self.globals.get(target) {
                self.check_function_value_arity(target, argc, params.len());
            }
            return;
        };
        // A builtin call lowering rewrote to its source-companion body is the
        // builtin's (`check_builtin_call_args`), not the body's signature's.
        if self.source_path.get() || builtin_call_target(target).is_some() {
            return;
        }
        let required = sig.total.saturating_sub(sig.optional);
        if argc < required || argc > sig.total {
            self.emit(
                "TYPE_CALL_ARITY_MISMATCH",
                format!(
                    "Call to `{target}` has {argc} argument(s), expected {required} to {}.",
                    sig.total
                ),
            );
        }
    }

    /// A function value's callable type carries no defaults, so the call
    /// supplies exactly its parameter count (syntaxcheck's
    /// `check_function_value_call`). Package path only, like every count rule:
    /// the source-written count is `ir::shape`'s.
    fn check_function_value_arity(&self, target: &str, argc: usize, expected: usize) {
        if argc != expected && !self.source_path.get() {
            self.emit(
                "TYPE_CALL_ARITY_MISMATCH",
                format!("Call to `{target}` has {argc} argument(s), expected {expected}."),
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
        locals: &HashMap<String, ParameterType>,
    ) {
        // A function value (a local or global of FUNC type) has no named
        // signature; its callable type gives the per-position parameter types
        // (syntaxcheck's `check_function_value_call`). A local shadows a global.
        let function_value = match locals.get(target) {
            Some(t) => Some(t),
            None if self.functions.contains_key(target) => None,
            None => self.globals.get(target),
        };
        if let Some(t) = function_value {
            if let ParameterType::Func(params, _, _) = t {
                for (index, (arg, expected)) in args.iter().zip(params.iter()).enumerate() {
                    let Some(actual) = self.infer_type(arg, locals) else {
                        continue;
                    };
                    if !self.expression_compatible(expected, &actual, arg) {
                        let (actual, expected) = (actual.name(), expected.name());
                        self.emit(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            format!(
                                "Argument {} for `{target}` has type {actual}, expected {expected}.",
                                index + 1
                            ),
                        );
                    }
                }
            }
            return;
        }
        let Some(sig) = self.functions.get(target) else {
            return;
        };
        // A builtin call lowering rewrote to its source-companion body is
        // type-checked as the builtin (`check_builtin_call_args`), whose
        // registry signature is the one the source wrote against; only the
        // body's declared `STATE` clauses still bind the arguments here.
        let rewritten_builtin = builtin_call_target(target).is_some();
        for (index, arg) in args.iter().enumerate() {
            let Some(param_type) = sig.params.get(index) else {
                break;
            };
            let Some(actual) = self.infer_type(arg, locals) else {
                continue;
            };
            self.check_argument_state_agreement(target, index, param_type, &actual);
            if rewritten_builtin {
                continue;
            }
            // Strip a resource argument's `STATE T` clause; the parameter type
            // is the bare resource type.
            let actual = resource_base_type(&actual);
            let param_type = resource_base_type(param_type);
            self.check_literal_range(&param_type, arg);
            if !self.expression_compatible(&param_type, &actual, arg) {
                let (actual, param_type) = (actual.name(), param_type.name());
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
        locals: &HashMap<String, ParameterType>,
    ) {
        if target != crate::codegen::builtins::thread::TRANSFER_RESOURCE {
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
        // The thread plane's resource and its `STATE` clause both ride inside the
        // handle's rendered spelling, so these read off the names.
        let handle_name = handle_type.name();
        let resource_name = resource_type.name();
        let Some(plane_resource) = crate::types::thread_resource(&handle_name) else {
            return;
        };
        let plane_state = crate::codegen::resource::state_type_name(plane_resource);
        let resource_state = crate::codegen::resource::state_type_name(&resource_name);
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
                crate::codegen::resource::base_resource_name(plane_resource)
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
        param_type: &ParameterType,
        actual: &ParameterType,
    ) {
        let Some(param_state) = param_type.state() else {
            return; // bare parameter: the opt-out — any state or none.
        };
        let arg_state = actual.state();
        if arg_state.as_ref() == Some(&param_state) {
            return;
        }
        let (param_state, arg_state) = (param_state.name(), arg_state.map(|s| s.name()));
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
    /// must be defaultable (`TYPE_STATE_INVALID`). plan-74 lifted the former
    /// `TYPE_UNION_STATE_FORBIDDEN` ban here — a resource union may now return a
    /// uniform STATE just as a concrete stateful resource does; the defaultable
    /// rule is a property of the STATE type itself and does not care which position
    /// declares it.
    ///
    /// This was unreachable from a return for a subtle reason worth recording:
    /// the binding rules pattern-match `" STATE "` in a type string, and the return
    /// type string never contained it (plan-52-D restored that append). But the
    /// append alone does **not** make it fire — it runs over `IrOp::Bind`, and a
    /// function's return is not a binding. The same omission that rejected the
    /// legal stateful `RETURN` also hid it, and each needed its own fix.
    pub(super) fn check_return_state_declaration(&self, function: &IrFunction) {
        let Some(state_type) = function.returns.state() else {
            return;
        };
        if !self.is_defaultable(&state_type, &mut HashSet::new()) {
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
        type_: &ParameterType,
        value: &Option<IrValue>,
        locals: &HashMap<String, ParameterType>,
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
            if let Some(declared) = type_.state().map(|s| s.name().into_owned()) {
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
        let Some(value_state) = actual.state() else {
            return; // stateless initializer: attach (or stay bare) — both legal.
        };
        let value_state = value_state.name();
        match type_.state().map(|s| s.name().into_owned()) {
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
