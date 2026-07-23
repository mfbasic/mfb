use super::*;

impl TypeEnv {
    // 11. Result-type checks + builtin call args
    // ===========================================================================

    /// Reject a `MemberAccess` whose annotated result type disagrees with the
    /// declared type of the field it reads.
    ///
    /// `infer_type` prefers this annotation over resolving the field, so a lie
    /// here propagates into every downstream rule: an `Integer` field annotated
    /// `String` lets `field & "x"` pass and codegen concatenates through an
    /// integer. Reject only when the target's record type and the field are both
    /// resolvable — an unresolved shape is left unchecked, as elsewhere.
    pub(super) fn check_member_access_type(
        &self,
        target: &IrValue,
        member: &str,
        node: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let Some(annotated) = usable_type(node.annotated_type()) else {
            return;
        };
        let Some(target_type) = self.infer_type(target, locals) else {
            return;
        };
        let Some(declared) = self.field_type(resource_base_type(&target_type), member) else {
            return;
        };
        if !self.compatible(&declared, &annotated) {
            self.emit(
                VERIFY_TYPE,
                format!(
                    "member `{target_type}::{member}` is annotated as {annotated}, but the field is declared {declared}"
                ),
            );
        }
    }

    /// Reject an operator node whose annotated result type disagrees with the
    /// type its operands produce. `derived` is `None` when the result cannot be
    /// derived (an operand type is unknown, or the operands disagree), in which
    /// case the annotation is left alone.
    pub(super) fn check_operator_result_type(&self, node: &IrValue, derived: Option<String>) {
        let (Some(derived), Some(annotated)) = (derived, usable_type(node.annotated_type())) else {
            return;
        };
        if !self.compatible(&derived, &annotated) {
            self.emit(
                VERIFY_TYPE,
                format!(
                    "operator result is annotated {annotated}, but its operands produce {derived}"
                ),
            );
        }
    }

    /// Reject a call node whose annotated result type disagrees with the callee's
    /// declared return type.
    ///
    /// Every computed node carries its own result type (plan-20-B) and
    /// `infer_type` echoes it. That is the front end's truth on the source path,
    /// but on the decoded-package path the annotation is attacker-controlled, and
    /// every rule built on `infer_type` — member access, operator operands, call
    /// arguments — then validates a fiction. A `String`-returning call annotated
    /// `Account` makes `MemberAccess{member:"balance"}` typecheck against a
    /// foreign record's layout; annotated `Integer`, it makes `result - 5` emit an
    /// integer subtract over a string pointer.
    ///
    /// The callee's declared `returns` is the independent source of truth, so the
    /// annotation must agree with it. Both `Call` and `CallResult` annotate the
    /// callee's return type (a fallible call's `Result OF T` is unwrapped to `T`
    /// by the node kind itself). For an internal function the truth is its
    /// `FnSig`; for a **builtin** (no `FnSig`) the truth is the arg-typed
    /// return-type oracle `builtins::resolve_call_return_type` — the same resolver
    /// the front end used to produce the annotation — so a crafted `.mfp` cannot
    /// fabricate a record return on, say, `strings.length` and defeat the
    /// downstream member-access check (bug-162). An indirect call through a local
    /// is skipped; `Unknown` on either side never rejects.
    pub(super) fn check_call_result_type(
        &self,
        target: &str,
        node: &IrValue,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        if locals.contains_key(target) {
            return; // indirect call — no named signature
        }
        let Some(annotated) = usable_type(node.annotated_type()) else {
            return;
        };
        let declared = if let Some(sig) = self.functions.get(target) {
            usable_type(Some(&sig.returns))
        } else {
            // Builtin: derive the expected return from the same arg-typed oracle
            // the monomorphizer uses. Reconcile only when every argument type is
            // known (`resource_base_type` strips a resource `STATE T` clause, as
            // `check_builtin_call_args` does) so an inference gap never rejects.
            let Some(arg_types) = args
                .iter()
                .map(|a| {
                    self.infer_type(a, locals)
                        .map(|t| resource_base_type(&t).to_string())
                })
                .collect::<Option<Vec<String>>>()
            else {
                return;
            };
            crate::builtins::resolve_call_return_type(target, &arg_types)
                .and_then(|t| usable_type(Some(&t)))
        };
        let Some(declared) = declared else {
            return;
        };
        if !self.expression_compatible(&declared, &annotated, node) {
            self.emit(
                VERIFY_TYPE,
                format!(
                    "call to `{target}` is annotated as returning {annotated}, but `{target}` returns {declared}"
                ),
            );
        }
    }

    /// Reject a call to a numeric built-in whose argument types match no
    /// overload — the IR-level counterpart of `syntaxcheck`'s per-built-in
    /// `TYPE_CALL_ARGUMENT_MISMATCH`, reusing the *same* `resolve_call` dispatch
    /// the compiler already uses for return-type inference (so there is one
    /// source of truth for the argument rules, not a re-implementation). On
    /// decoded package IR a crafted `math.sqrt("x")` would otherwise reach
    /// codegen, which selects the float instruction from the declared numeric
    /// type. Restricted to the pure-numeric packages (math/bits) where the
    /// arguments are ordinary values with no receiver/predicate normalization,
    /// so `resolve_call`'s None is unambiguously an argument mismatch. Skipped
    /// unless every argument type is known (no false rejection).
    pub(super) fn check_builtin_call_args(
        &self,
        target: &str,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        // plan-54: a `thread::transfer` moves a resource to a re-typing thread, so
        // the transferred resource's STATE must agree with the plane's declared
        // STATE. Run before the STATE-stripping arg-type collection below, which
        // would erase exactly the clause this check needs.
        self.check_thread_transfer_state(target, args, locals);
        // `collections` element searches compare elements for equality, so the
        // list's element type must be comparable — syntaxcheck's
        // `check_special_builtin_arguments` arm of TYPE_REQUIRES_COMPARABLE.
        if matches!(
            target,
            "collections.contains" | "collections.replace" | "collections.find"
        ) {
            if let Some(first) = args.first() {
                if let Some(t) = self.infer_type(first, locals) {
                    if let Some(element) = resource_base_type(&t).strip_prefix("List OF ") {
                        if element != "Unknown" && !self.is_comparable(element) {
                            self.emit(
                                "TYPE_REQUIRES_COMPARABLE",
                                format!(
                                    "Call to `{target}` requires a comparable type, got `{element}`."
                                ),
                            );
                        }
                    }
                }
            }
        }
        // Strip the `STATE T` clause a resource argument carries in its type
        // string (`File STATE FileState` → `File`); resolve_call and the
        // parameter tables use the bare resource type.
        let arg_types: Option<Vec<String>> = args
            .iter()
            .map(|a| {
                self.infer_type(a, locals)
                    .map(|t| resource_base_type(&t).to_string())
            })
            .collect();
        let Some(arg_types) = arg_types else {
            return;
        };
        // `term` exposes its per-name signatures (`arity`, `param_types`)
        // rather than an arg-typed `resolve_call`, so check against those with
        // the ported `expression_compatible` — the same data syntaxcheck's
        // `check_term_builtin_call` uses, so term's signature is single-source.
        if builtins::term::is_term_call(target) {
            if let Some((min, max)) = builtins::term::arity(target) {
                if arg_types.len() < min || arg_types.len() > max {
                    let expected = if min == max {
                        min.to_string()
                    } else {
                        format!("{min} to {max}")
                    };
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{target}` has {} argument(s), expected {expected}.",
                            arg_types.len()
                        ),
                    );
                    return;
                }
            }
            let params = builtins::term::param_types(target).unwrap_or(&[]);
            let mut mismatch = false;
            for (i, param) in params.iter().enumerate() {
                if let (Some(actual), Some(arg)) = (arg_types.get(i), args.get(i)) {
                    if !self.expression_compatible(param, actual, arg) {
                        mismatch = true;
                    }
                }
            }
            if mismatch {
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{target}` has argument type(s) that do not match its signature."
                    ),
                );
            }
            return;
        }
        // `collections`/`general` builtins: per-name arity, then arg-typed
        // overload resolution (syntaxcheck's check_general_builtin_call arms).
        if builtins::collections::is_collections_call(target) {
            if let Some((min, max)) = builtins::collections::arity(target) {
                if arg_types.len() < min || arg_types.len() > max {
                    let expected = if min == max {
                        min.to_string()
                    } else {
                        format!("{min} to {max}")
                    };
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{target}` has {} argument(s), expected {expected}.",
                            arg_types.len()
                        ),
                    );
                    return;
                }
            }
            if builtins::collections::resolve_call(target, &arg_types).is_none() {
                let expected = builtins::collections::expected_arguments(target)
                    .unwrap_or("supported overload");
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{target}` has argument type(s) ({}), expected {expected}.",
                        arg_types.join(", ")
                    ),
                );
            }
            return;
        }
        if builtins::general::is_general_call(target) {
            if let Some((min, max)) = builtins::general::arity(target) {
                if arg_types.len() < min || arg_types.len() > max {
                    let expected = if min == max {
                        min.to_string()
                    } else {
                        format!("{min} to {max}")
                    };
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{target}` has {} argument(s), expected {expected}.",
                            arg_types.len()
                        ),
                    );
                    return;
                }
            }
            if builtins::general::resolve_call(target, &arg_types).is_none() {
                // A package-provided override may accept what the built-in
                // rejects (plan-01-overload §A.3.2) — never reject those.
                if builtins::general::is_overridable(target)
                    && arg_types.len() == 1
                    && builtins::general_override_target(target, &arg_types[0]).is_some()
                {
                    return;
                }
                let expected =
                    builtins::general::expected_arguments(target).unwrap_or("supported overload");
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{target}` has argument type(s) ({}), expected {expected}.",
                        arg_types.join(", ")
                    ),
                );
            }
            return;
        }
        let unresolved = if builtins::math::is_math_call(target) {
            builtins::math::resolve_call(target, &arg_types).is_none()
        } else if builtins::bits::is_bits_call(target) {
            builtins::bits::resolve_call(target, &arg_types).is_none()
        } else if builtins::vector::is_vector_call(target) {
            builtins::vector::resolve_call(target, &arg_types).is_none()
        } else if builtins::strings::is_strings_call(target) {
            builtins::strings::resolve_call(target, &arg_types).is_none()
        } else if builtins::encoding::is_encoding_call(target) {
            builtins::encoding::resolve_call(target, &arg_types).is_none()
        } else if builtins::io::is_io_call(target) {
            builtins::io::resolve_call(target, &arg_types).is_none()
        } else if builtins::fs::is_fs_call(target) {
            builtins::fs::resolve_call(target, &arg_types).is_none()
        } else if builtins::net::is_net_call(target) {
            builtins::net::resolve_call(target, &arg_types).is_none()
        } else if builtins::os::is_os_call(target) {
            builtins::os::resolve_call(target, &arg_types).is_none()
        } else {
            return;
        };
        if unresolved {
            self.emit(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                format!("Arguments to `{target}` do not match any overload."),
            );
        }
    }

    // ===========================================================================
    // 12. Compatibility + typed statement checks
    // ===========================================================================

    /// Type compatibility (`syntaxcheck::compatible`), on canonical type-name
    /// strings. `Unknown` on either side is compatible; the `RES` ownership
    /// marker is stripped; container types recurse; a union accepts any of its
    /// variants. Anything unresolved falls back to string equality (never a
    /// false rejection because callers gate on both types being known).
    pub(super) fn compatible(&self, expected: &str, actual: &str) -> bool {
        if expected == "Unknown" || actual == "Unknown" {
            return true;
        }
        let expected = expected.strip_prefix("RES ").unwrap_or(expected);
        let actual = actual.strip_prefix("RES ").unwrap_or(actual);
        if expected == actual {
            return true;
        }
        if let (Some(e), Some(a)) = (
            expected.strip_prefix("List OF "),
            actual.strip_prefix("List OF "),
        ) {
            return self.compatible(e, a);
        }
        if let (Some(e), Some(a)) = (
            expected.strip_prefix("Result OF "),
            actual.strip_prefix("Result OF "),
        ) {
            return self.compatible(e, a);
        }
        if let (Some((ek, ev)), Some((ak, av))) = (parse_map(expected), parse_map(actual)) {
            return self.compatible(ek, ak) && self.compatible(ev, av);
        }
        // Bare-name equality (an imported type is registered under its bare
        // name; a qualified `pkg.Type` reference resolves to the same type).
        let expected_bare = expected.rsplit('.').next().unwrap_or(expected);
        let actual_bare = actual.rsplit('.').next().unwrap_or(actual);
        if expected_bare == actual_bare {
            return true;
        }
        // A union accepts any of its variants.
        if let Some(variants) = self.union_variants(expected) {
            if variants.contains(actual_bare) {
                return true;
            }
        }
        false
    }

    /// `syntaxcheck::expression_compatible`: `compatible`, plus the literal
    /// coercions that the AST checker allows for constant arguments — a `Byte`
    /// parameter accepts an in-range `Integer` literal, `Fixed` accepts an
    /// `Integer`/`Float` literal. The `Const` node carries the literal type and
    /// value, so the same check applies on the IR.
    pub(super) fn expression_compatible(
        &self,
        expected: &str,
        actual: &str,
        value: &IrValue,
    ) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        if let IrValue::Const { type_, value } = value {
            match (expected, type_.as_str()) {
                ("Byte", "Integer") => {
                    return value.parse::<u16>().is_ok_and(|n| n <= u8::MAX as u16);
                }
                ("Fixed", "Integer") | ("Fixed", "Float") => return true,
                // A decimal literal coerces to a Money slot (plan-29-A §4.4).
                ("Money", "Integer") | ("Money", "Float") => return true,
                _ => {}
            }
        }
        // Negated numeric literal into Fixed / Money (`-1`, `-1.25`).
        if expected == "Fixed" || expected == "Money" {
            if let IrValue::Unary { op, operand, .. } = value {
                if op == "-"
                    && matches!(operand.as_ref(), IrValue::Const { type_, .. } if type_ == "Integer" || type_ == "Float")
                {
                    return true;
                }
            }
        }
        false
    }

    /// Reject a `RETURN <value>` whose value type is incompatible with the
    /// function's declared return type (`syntaxcheck`'s `TYPE_RETURN_MISMATCH`).
    /// Codegen places the return value into the ABI return slot by the declared
    /// type, so a crafted mismatch is a type confusion at the return boundary.
    pub(super) fn check_return_type(&self, value: &IrValue, locals: &HashMap<String, String>) {
        let expected = self.current_return.borrow().clone();
        if expected.is_empty() || expected == "Nothing" || expected == "Unknown" {
            return;
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible(&expected, &actual, value) {
            self.emit(
                "TYPE_RETURN_MISMATCH",
                format!("RETURN value has type {actual}, expected {expected}."),
            );
        }
    }

    /// Reject a binding whose initializer type is incompatible with its declared
    /// type — `syntaxcheck`'s `TYPE_BINDING_MISMATCH`. The caller suppresses this
    /// when a literal-range error already fired for the same binding (matching
    /// syntaxcheck's `!reported_range_error` guard), so a single out-of-range
    /// literal is reported once, as the more specific range error.
    pub(super) fn check_binding_type(
        &self,
        name: &str,
        declared: &str,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let expected = resource_base_type(declared);
        if expected.is_empty() || expected == "Nothing" || expected == "Unknown" {
            return;
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        // Compare base-to-base: `declared` is already stripped, so the initializer
        // must be too, or `RES h AS File STATE Cursor = openTagged(p)` reads as
        // "initializer `File STATE Cursor`, expected `File`". Before returns
        // carried their STATE (plan-52-D) an initializer's type never contained
        // one, so the asymmetry was invisible. Whether the two STATEs *agree* is a
        // separate question, answered by `check_binding_state_agreement`.
        let actual = resource_base_type(&actual).to_string();
        if !self.expression_compatible(expected, &actual, value) {
            self.emit(
                "TYPE_BINDING_MISMATCH",
                format!("Binding `{name}` has initializer type {actual}, expected {expected}."),
            );
        }
    }

    /// Reject a control-flow condition (IF/WHILE/LOOP UNTIL/WHEN guard) whose
    /// type is provably not Boolean — `syntaxcheck`'s
    /// `TYPE_CONDITION_REQUIRES_BOOLEAN`. `what` is the statement-specific
    /// message prefix (`"IF condition"`, `"WHEN guard"`, …).
    pub(super) fn check_condition_boolean(
        &self,
        what: &str,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible("Boolean", &actual, value) {
            self.emit(
                "TYPE_CONDITION_REQUIRES_BOOLEAN",
                format!("{what} has type {actual}, expected Boolean."),
            );
        }
    }

    /// Reject an assignment whose value type is incompatible with the target
    /// binding's settled type — `syntaxcheck`'s `TYPE_ASSIGNMENT_MISMATCH`. The
    /// caller suppresses this when a literal-range error already fired
    /// (syntaxcheck's `!reported_range_error` guard). Unlike `TYPE_BINDING_MISMATCH`
    /// no explicit-annotation gate applies: by assignment time the binding's
    /// type is settled regardless of how it was declared.
    pub(super) fn check_assignment_type(
        &self,
        name: &str,
        declared: &str,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let expected = resource_base_type(declared);
        if expected.is_empty() || expected == "Nothing" || expected == "Unknown" {
            return;
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible(expected, &actual, value) {
            self.emit(
                "TYPE_ASSIGNMENT_MISMATCH",
                format!("Assignment to `{name}` has type {actual}, expected {expected}."),
            );
        }
    }

    /// The syntaxcheck constructor rules on a lowered `Constructor` value: the
    /// name must be a record TYPE (`TYPE_CONSTRUCTOR_REQUIRES_RECORD`), the
    /// argument count must equal the field count exactly — records have no
    /// field defaults — (`TYPE_CONSTRUCTOR_ARITY_MISMATCH`), and each argument
    /// must be compatible with its positional field
    /// (`TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH`). Lowering reorders named
    /// arguments into field order, so positional checking covers both forms.
    pub(super) fn check_constructor(
        &self,
        type_name: &str,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        // `Ok`/`Result` are compiler-owned (syntaxcheck's TYPE_RESULT_IS_IMPLICIT).
        if matches!(type_name, "Ok" | "Result") {
            self.emit(
                "TYPE_RESULT_IS_IMPLICIT",
                format!("`{type_name}` is compiler-owned and cannot be constructed directly."),
            );
            return;
        }
        // Compiler-owned records may never be user-constructed (syntaxcheck's
        // TYPE_READ_ONLY_RECORD_CONSTRUCTOR). The Error/ErrorLoc arm of that
        // rule stays in syntaxcheck: lowering itself emits `Constructor{Error}`
        // for the `error()` builtin and trap machinery, so on the IR a user
        // `Error[..]` is indistinguishable from a legitimate synthesized one.
        if read_only_record_type(type_name) {
            self.emit(
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                format!("TYPE `{type_name}` is compiler-owned and cannot be constructed."),
            );
            return;
        }
        if !self.records.contains_key(type_name) {
            // A constructor naming a declared non-record type is malformed; an
            // unknown name is left alone (could be a builtin record).
            let kind = if self.unions.contains_key(type_name) {
                Some("UNION")
            } else if self.enums.contains_key(type_name) {
                Some("ENUM")
            } else {
                None
            };
            if let Some(kind) = kind {
                self.emit(
                    "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
                    format!("`{type_name}` is a {kind}, not a record TYPE."),
                );
            }
            return;
        }
        // A private type (or one with hidden fields) may only be constructed
        // from its declaring file (syntaxcheck's TYPE_MEMBER_NOT_VISIBLE arms).
        if let Some((file, visibility)) = self.type_decl_info.get(type_name) {
            if visibility == "private" && !file.is_empty() && *file != *self.current_file.borrow() {
                self.emit(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    format!("Constructor `{type_name}` is not visible from this file."),
                );
                return;
            }
        }
        if let Some(private) = self.private_fields.get(type_name) {
            if self
                .type_decl_info
                .get(type_name)
                .is_some_and(|(file, _)| !file.is_empty() && *file != *self.current_file.borrow())
            {
                for field in private {
                    self.emit(
                        "TYPE_MEMBER_NOT_VISIBLE",
                        format!(
                            "Constructor `{type_name}` cannot set hidden field `{field}` from this file."
                        ),
                    );
                }
            }
        }
        let Some(fields) = self.record_field_lists.get(type_name) else {
            return;
        };
        if args.len() != fields.len() {
            self.emit(
                "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
                format!(
                    "Constructor `{type_name}` has {} argument(s), expected {}.",
                    args.len(),
                    fields.len()
                ),
            );
        }
        for (index, arg) in args.iter().enumerate() {
            let Some((field_name, field_type)) = fields.get(index) else {
                continue;
            };
            let Some(actual) = self.infer_type(arg, locals) else {
                continue;
            };
            if !self.expression_compatible(field_type, &actual, arg) {
                self.emit(
                    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                    format!(
                        "Argument {} for `{type_name}` has type {actual}, expected {field_type} for field `{field_name}`.",
                        index + 1
                    ),
                );
            }
        }
    }

    /// Reject a `UnionWrap` whose `member_type` is not a variant of the named
    /// union (a value smuggled under a tag the union does not define).
    pub(super) fn check_union_wrap(&self, union_type: &str, member_type: &str) {
        if member_type.is_empty() {
            return;
        }
        if let Some(variants) = self.union_variants(union_type) {
            if !variants.contains(member_type) {
                self.emit(
                    VERIFY_TYPE,
                    format!("`{member_type}` is not a variant of union `{union_type}`"),
                );
            }
        }
    }

    /// Reject a `UnionExtract` whose extracted `type_` is not a variant of the
    /// union its `value` is typed as — the read counterpart of `check_union_wrap`.
    /// A crafted `.mfp` could otherwise extract a foreign variant's payload from a
    /// union that never carries it, so codegen reads that variant's layout off the
    /// wrong value (bug-162). Skipped when the value's type is unknown or is not a
    /// union, so a legitimate extract never rejects.
    pub(super) fn check_union_extract(
        &self,
        type_: &str,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        if type_.is_empty() {
            return;
        }
        let Some(union_type) = self.infer_type(value, locals) else {
            return;
        };
        let union_type = resource_base_type(&union_type);
        if let Some(variants) = self.union_variants(union_type) {
            if !variants.contains(type_) {
                self.emit(
                    VERIFY_TYPE,
                    format!("`{type_}` is not a variant of union `{union_type}`"),
                );
            }
        }
    }

    /// Verify every `Capture` in a value addresses a slot within the enclosing
    /// closure's captured-slot count. Skipped only when the function is never used
    /// as a closure body, so it has no environment to index at all.
    pub(super) fn check_value_captures(&self, value: &IrValue, slots: Option<usize>) {
        let Some(slots) = slots else {
            // The enclosing function is never targeted by any `Closure` node, so
            // it has no captured environment at all. A `Capture` here would lower
            // to an env-relative load off whatever `CLOSURE_ENV_REGISTER` holds
            // in a non-closure frame — an out-of-bounds read a crafted `.mfp`
            // could steer. The legitimate front end never emits a `Capture`
            // outside a closure body (zero-capture lambdas lower to a plain
            // `FunctionRef`), so any such `Capture` is malformed IR (bug-99).
            let mut stray = None;
            walk_captures(value, &mut |index| {
                if stray.is_none() {
                    stray = Some(index);
                }
            });
            if let Some(index) = stray {
                self.emit(
                    VERIFY_TYPE,
                    format!(
                        "closure capture index {index} appears in a function that is \
                         not a closure body (no captured environment)"
                    ),
                );
            }
            return;
        };
        let mut violation = None;
        walk_captures(value, &mut |index| {
            if index as usize >= slots && violation.is_none() {
                violation = Some(index);
            }
        });
        if let Some(index) = violation {
            self.emit(
                VERIFY_TYPE,
                format!("closure capture index {index} is out of range ({slots} slot(s))"),
            );
        }
    }

    // ===========================================================================
}
