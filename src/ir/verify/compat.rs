use super::*;

/// The builtin a lowered call targets, by the name the source wrote: the
/// target itself when it is a builtin call, or — for a call lowering rewrote
/// to a source-companion body (a `Body::Mfb`/`Body::Rewrite` member, the
/// `astrings` Tier-B transforms, the `term::drawText(AttributedString)`
/// bridge) — the member that body implements. `None` for a declared function.
///
/// A rewritten call keeps the builtin argument normalization (extras kept, no
/// defaults filled), so its lowered `args` are still the checker's list; only
/// the target name changed, and the rules below must report the builtin.
pub(super) fn builtin_call_target(target: &str) -> Option<String> {
    if builtins::is_builtin_call(target) {
        return Some(target.to_string());
    }
    if target == crate::internal_name::internalize("__term_drawTextAttr") {
        return Some(crate::codegen::builtins::term::DRAW_TEXT.to_string());
    }
    if let Some(owner) = crate::codegen::builtins::strings::tier_b_transform_owner(target) {
        return Some(owner.to_string());
    }
    crate::codegen::registry::rewrite_owner(target)
}

/// The name of a bare general built-in predicate in a callback position: a
/// `FunctionRef` lowering typed from the list's element type, or the `Local`
/// it left when it could not (no such local exists — the name is the builtin's).
fn builtin_predicate_name<'a>(
    value: &'a IrValue,
    locals: &HashMap<String, ParameterType>,
) -> Option<&'a str> {
    match value {
        IrValue::FunctionRef { name, .. } | IrValue::Local(name)
            if !locals.contains_key(name)
                && crate::codegen::builtins::general::builtin_function_id(name).is_some() =>
        {
            Some(name)
        }
        _ => None,
    }
}

/// Whether an argument is a non-negative integer literal that fits in a `Byte`
/// (0..=255) — the same rule `expression_compatible` applies to let an
/// `Integer` literal satisfy a `Byte` parameter. Only a literal qualifies; a
/// computed `Integer` value does not.
fn is_byte_literal(value: &IrValue) -> bool {
    matches!(value, IrValue::Const { type_: ParameterType::Integer, value }
        if value.parse::<u16>().is_ok_and(|n| n <= u8::MAX as u16))
}

/// Resolve a table-driven builtin call, retrying with `Integer`-literal
/// arguments coerced to `Byte` when the exact-typed resolution fails
/// (the former source checker's `resolve_table_call_with_byte_literals`): the table arm
/// resolves by exact argument-type match, which rejects an integer literal
/// passed to a `Byte` parameter (`astrings::foreground(255, 0, 0)`). Each
/// subset of the eligible positions is tried, so a literal that is validly
/// either `Integer` or `Byte` resolves against whichever the overload expects.
fn resolve_table_call_with_byte_literals(
    target: &str,
    arg_types: &[ParameterType],
    args: &[IrValue],
) -> Option<ParameterType> {
    if let Some(return_type) = builtins::resolve_call_return_type_typed(target, arg_types, true) {
        return Some(return_type);
    }
    let eligible: Vec<usize> = arg_types
        .iter()
        .enumerate()
        .filter(|(index, type_)| {
            matches!(type_, ParameterType::Integer) && args.get(*index).is_some_and(is_byte_literal)
        })
        .map(|(index, _)| index)
        .collect();
    // Bound the subset search: a `Byte`-parameter overload never has many
    // positions, and this only runs on the error path.
    if eligible.is_empty() || eligible.len() > 6 {
        return None;
    }
    for mask in 1u32..(1u32 << eligible.len()) {
        let mut trial = arg_types.to_vec();
        for (bit, &index) in eligible.iter().enumerate() {
            if mask & (1 << bit) != 0 {
                trial[index] = ParameterType::Byte;
            }
        }
        if let Some(return_type) = builtins::resolve_call_return_type_typed(target, &trial, true) {
            return Some(return_type);
        }
    }
    None
}

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
        locals: &HashMap<String, ParameterType>,
    ) {
        let Some(annotated) = usable_type(node.annotated_parameter_type()) else {
            return;
        };
        let Some(target_type) = self.infer_type(target, locals) else {
            return;
        };
        let Some(declared) = self.field_type(&resource_base_type(&target_type), member) else {
            return;
        };
        if !self.compatible(&declared, &annotated) {
            let (target_type, annotated, declared) =
                (target_type.name(), annotated.name(), declared.name());
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
    pub(super) fn check_operator_result_type(
        &self,
        node: &IrValue,
        derived: Option<ParameterType>,
    ) {
        let (Some(derived), Some(annotated)) =
            (derived, usable_type(node.annotated_parameter_type()))
        else {
            return;
        };
        if !self.compatible(&derived, &annotated) {
            let (annotated, derived) = (annotated.name(), derived.name());
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
        locals: &HashMap<String, ParameterType>,
    ) {
        if locals.contains_key(target) {
            return; // indirect call — no named signature
        }
        let Some(annotated) = usable_type(node.annotated_parameter_type()) else {
            return;
        };
        let declared = if let Some(sig) = self.functions.get(target) {
            usable_type(Some(sig.returns.clone()))
        } else {
            // Builtin: derive the expected return from the same arg-typed oracle
            // the monomorphizer uses. Reconcile only when every argument type is
            // known (`resource_base_type` strips a resource `STATE T` clause, as
            // `check_builtin_call_args` does) so an inference gap never rejects.
            let Some(arg_types) = args
                .iter()
                .map(|a| self.infer_type(a, locals).map(|t| resource_base_type(&t)))
                .collect::<Option<Vec<ParameterType>>>()
            else {
                return;
            };
            crate::codegen::builtins::resolve_call_return_type_typed(target, &arg_types, false)
                .and_then(|t| usable_type(Some(t)))
        };
        let Some(declared) = declared else {
            return;
        };
        if !self.expression_compatible(&declared, &annotated, node) {
            let (annotated, declared) = (annotated.name(), declared.name());
            self.emit(
                VERIFY_TYPE,
                format!(
                    "call to `{target}` is annotated as returning {annotated}, but `{target}` returns {declared}"
                ),
            );
        }
    }

    /// Emit `TYPE_CALL_ARITY_MISMATCH` and return `true` when `actual` falls
    /// outside `[min, max]` for a per-name-arity built-in (bug-342 A10 — the
    /// term/collections/general arity checks shared this exact body).
    fn builtin_arity_errored(&self, target: &str, actual: usize, min: usize, max: usize) -> bool {
        if actual < min || actual > max {
            // On the source path the count the source wrote is `ir::shape`'s
            // to report (lowering pads a builtin's optional trailing arguments
            // and appends the extras, so the lowered count is not that count);
            // the structural check still ends this call's checks, as the
            // checker's did.
            if !self.source_path.get() {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.emit(
                    "TYPE_CALL_ARITY_MISMATCH",
                    format!("Call to `{target}` has {actual} argument(s), expected {expected}."),
                );
            }
            true
        } else {
            false
        }
    }

    /// The builtin-call family — the former source checker's `check_builtin_call` transcribed
    /// over the lowered call (plan-107-E): `TYPE_CALL_ARITY_MISMATCH` and
    /// `TYPE_CALL_ARGUMENT_MISMATCH` for every builtin whose arguments the
    /// checker validated (`builtins::checks_call_arguments`), in the checker's
    /// own dispatch order — the four bespoke arms (`general`, `collections`,
    /// `term`, `thread`) ahead of the shared package table — and with its
    /// ordering inside a call: arity before resolution, resolution before the
    /// comparability rule. Lowering has already normalized the call the way
    /// the checker did (named arguments bound, unknown names dropped, extras
    /// kept), so the lowered `args` are the checker's normalized list.
    ///
    /// On decoded package IR this is the ABI defense PKG-02 named: codegen
    /// marshals every builtin argument by the registry's declared parameter
    /// type, so a crafted `math.sqrt("x")` reaches the float instruction with a
    /// string pointer unless rejected here. An argument whose type cannot be
    /// inferred is `Unknown` — the checker's own spelling for it — rather than
    /// a reason to skip the call.
    pub(super) fn check_builtin_call_args(
        &self,
        target: &str,
        args: &[IrValue],
        locals: &HashMap<String, ParameterType>,
    ) {
        // plan-54: a `thread::transfer` moves a resource to a re-typing thread, so
        // the transferred resource's STATE must agree with the plane's declared
        // STATE. Run before the STATE-stripping arg-type collection below, which
        // would erase exactly the clause this check needs.
        self.check_thread_transfer_state(target, args, locals);
        let Some(builtin) = builtin_call_target(target) else {
            return;
        };
        let target = builtin.as_str();
        if !builtins::checks_call_arguments(target) {
            return;
        }
        // Strip the `STATE T` clause a resource argument carries in its type
        // (`File STATE FileState` → `File`); resolve_call and the parameter
        // tables use the bare resource type.
        let arg_types: Vec<ParameterType> = args
            .iter()
            .map(|arg| {
                self.infer_type(arg, locals)
                    .map(|t| resource_base_type(&t))
                    .unwrap_or(ParameterType::Unknown)
            })
            .collect();
        let arg_type_names = || {
            arg_types
                .iter()
                .map(|t| t.name().into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let expected_overloads = || {
            builtins::expected_arguments(target).unwrap_or_else(|| "supported overload".to_string())
        };
        let registry = crate::codegen::registry::registry();

        if crate::codegen::builtins::general::is_general_call(target) {
            if let Some((min, max)) = builtins::arity(target) {
                if self.builtin_arity_errored(target, args.len(), min, max) {
                    return;
                }
            }
            if builtins::resolve_call_return_type_typed(target, &arg_types, true).is_none() {
                // A package-provided override may accept what the built-in
                // rejects (plan-01-overload §A.3.2) — never reject those.
                if crate::codegen::builtins::general::is_overridable(target)
                    && arg_types.len() == 1
                    && builtins::general_override_target(target, &arg_types[0].name()).is_some()
                {
                    return;
                }
                self.emit_argument_mismatch(format!(
                    "Call to `{target}` has argument type(s) ({}), expected {}.",
                    arg_type_names(),
                    expected_overloads()
                ));
                return;
            }
            self.check_builtin_comparability(target, target, &arg_types);
            return;
        }

        if registry.owning_package(target) == Some("collections") {
            // `callee` is a collections native member; the generic dequalifier
            // hands back its bare native name (`collections.get` -> `get`).
            let member = builtins::native_builtin_target(target).unwrap_or(target);
            // A bare general built-in predicate in the callback position
            // (bug-368): the predicate's type is derived from the list's element
            // type, and the diagnostic quotes the predicate's NAME.
            if crate::codegen::registry::callback_member(target) && args.len() == 2 {
                if let Some(predicate) = builtin_predicate_name(&args[1], locals) {
                    let collection_type_name = arg_types[0].name().into_owned();
                    let predicate_type = match &arg_types[0] {
                        ParameterType::ListOf(element) => {
                            crate::codegen::builtins::general::filter_predicate_type(
                                predicate,
                                &element.name(),
                            )
                        }
                        _ => None,
                    };
                    let Some(predicate_type) = predicate_type else {
                        self.emit_argument_mismatch(format!(
                                "Call to `{target}` has argument type(s) ({collection_type_name}, {predicate}), expected {}.",
                                expected_overloads()
                            ),
                        );
                        return;
                    };
                    let trial = vec![arg_types[0].clone(), ParameterType::parse(&predicate_type)];
                    if builtins::resolve_call_return_type_typed(target, &trial, true).is_none() {
                        self.emit_argument_mismatch(format!(
                                "Call to `{target}` has argument type(s) ({collection_type_name}, {predicate_type}), expected {}.",
                                expected_overloads()
                            ),
                        );
                    }
                    return;
                }
            }
            if let Some((min, max)) = builtins::arity(target) {
                if self.builtin_arity_errored(target, args.len(), min, max) {
                    return;
                }
            }
            if builtins::resolve_call_return_type_typed(target, &arg_types, true).is_none() {
                self.emit_argument_mismatch(format!(
                    "Call to `{target}` has argument type(s) ({}), expected {}.",
                    arg_type_names(),
                    expected_overloads()
                ));
                return;
            }
            self.check_builtin_comparability(target, member, &arg_types);
            return;
        }

        if registry.owning_package(target) == Some("term") {
            if let Some((min, max)) = builtins::arity(target) {
                if self.builtin_arity_errored(target, args.len(), min, max) {
                    return;
                }
            }
            // `term::drawText` additionally accepts an `AttributedString` at the
            // text position (the source-companion overload); only the third
            // expected type flips. Whether the `astrings` bridge that overload
            // routes to is imported is a source fact (`ir::shape`).
            let third_is_attributed = target == crate::codegen::builtins::term::DRAW_TEXT
                && arg_types.len() == 3
                && arg_types[2].name() == "AttributedString";
            let params: Vec<ParameterType> = if third_is_attributed {
                vec![
                    ParameterType::Integer,
                    ParameterType::Integer,
                    ParameterType::named("AttributedString"),
                ]
            } else {
                builtins::argument_types_typed(target).unwrap_or_default()
            };
            let mismatch = params.iter().zip(arg_types.iter()).zip(args.iter()).any(
                |((expected, actual), arg)| !self.expression_compatible(expected, actual, arg),
            );
            if mismatch {
                let expected = builtins::expected_arguments(target)
                    .unwrap_or_else(|| "no arguments".to_string());
                self.emit_argument_mismatch(format!(
                    "Call to `{target}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names()
                ));
            }
            return;
        }

        if crate::codegen::builtins::thread::is_thread_call(target) {
            if target == "thread.start" {
                // The entry must be an exported ISOLATED FUNC of an imported
                // package. The IR keeps only what survives lowering — a
                // `FunctionRef` typed `ISOLATED FUNC` — which is a superset of
                // the valid entries (a same-package `self::` export and a bare
                // same-package function both canonicalize to the bare name), so
                // the source path leaves the rejection to `ir::shape` and only
                // the package path rejects here; a call whose entry fails even
                // this test is not checked further, as the checker did not.
                let entry_is_isolated_ref = matches!(
                    args.first(),
                    Some(IrValue::FunctionRef {
                        type_: ParameterType::Func(_, _, true),
                        ..
                    })
                );
                if !entry_is_isolated_ref {
                    if !self.source_path.get() {
                        self.emit_argument_mismatch("thread.start entry point must be an exported ISOLATED FUNC from an imported package.".to_string(),
                        );
                    }
                    return;
                }
            }
            if let Some((min, max)) = builtins::arity(target) {
                if self.builtin_arity_errored(target, args.len(), min, max) {
                    return;
                }
            }
            if builtins::resolve_call_return_type_typed(target, &arg_types, true).is_none() {
                self.emit_argument_mismatch(format!(
                    "Call to `{target}` has argument type(s) ({}), expected {}.",
                    arg_type_names(),
                    expected_overloads()
                ));
            }
            return;
        }

        // The shared package table: arity, then arg-typed overload resolution
        // with the literal→`Byte` retry the checker gave the table packages.
        if let Some((min, max)) = builtins::arity(target) {
            if self.builtin_arity_errored(target, args.len(), min, max) {
                return;
            }
        }
        if resolve_table_call_with_byte_literals(target, &arg_types, args).is_none() {
            self.emit_argument_mismatch(format!(
                "Call to `{target}` has argument type(s) ({}), expected {}.",
                arg_type_names(),
                expected_overloads()
            ));
        }
    }

    /// `collections` element searches compare elements for equality, so the
    /// list's element type must be comparable — the former source checker's
    /// `check_general_builtin_comparability` (TYPE_REQUIRES_COMPARABLE), run
    /// only after the call resolved, as the checker did.
    fn check_builtin_comparability(&self, target: &str, member: &str, arg_types: &[ParameterType]) {
        if !matches!(member, "contains" | "replace" | "find") {
            return;
        }
        let Some(ParameterType::ListOf(element)) = arg_types.first() else {
            return;
        };
        if !matches!(**element, ParameterType::Unknown) && !self.is_comparable(element) {
            let element = element.name();
            self.emit(
                "TYPE_REQUIRES_COMPARABLE",
                format!("Call to `{target}` requires a comparable type, got `{element}`."),
            );
        }
    }

    // ===========================================================================
    // 12. Compatibility + typed statement checks
    // ===========================================================================

    /// Type compatibility (the former source checker's `compatible`). `Unknown` on either side is
    /// compatible; the `RES` ownership marker is stripped; container types
    /// recurse; a union accepts any of its variants.
    ///
    /// plan-106-B: structural. The prefix cascade
    /// (`strip_prefix("RES ")`/`("List OF ")`/`("Result OF ")` + `parse_map`)
    /// became variant matches; `ir::verify` holds no copy of the type grammar.
    /// The **tail** stays in the name domain on purpose — bare-vs-qualified
    /// nominal equality (`fs.File` ≡ `File`) and union-variant membership are
    /// lookups keyed by type NAME, which is what an import registers.
    pub(super) fn compatible(&self, expected: &ParameterType, actual: &ParameterType) -> bool {
        if matches!(expected, ParameterType::Unknown) || matches!(actual, ParameterType::Unknown) {
            return true;
        }
        let expected = strip_res(expected);
        let actual = strip_res(actual);
        if expected == actual {
            return true;
        }
        match (expected, actual) {
            (ParameterType::ListOf(e), ParameterType::ListOf(a))
            | (ParameterType::ResultOf(e), ParameterType::ResultOf(a)) => {
                return self.compatible(e, a);
            }
            (ParameterType::MapOf(ek, ev), ParameterType::MapOf(ak, av)) => {
                return self.compatible(ek, ak) && self.compatible(ev, av);
            }
            _ => {}
        }
        let expected_name = expected.name();
        let actual_name = actual.name();
        // Bare-name equality (an imported type is registered under its bare
        // name; a qualified `pkg.Type` reference resolves to the same type).
        let expected_bare = expected_name
            .rsplit('.')
            .next()
            .unwrap_or(expected_name.as_ref());
        let actual_bare = actual_name
            .rsplit('.')
            .next()
            .unwrap_or(actual_name.as_ref());
        // ...except between two DIFFERENT BUILT-IN RESOURCE types. This fallback
        // was written when bare names were globally unique, so `File` and
        // `fs.File` could only ever mean the same type; plan-110 broke that
        // premise by giving `net`, `tcp`, `udp` and `tls` resources with
        // identical bare names (`Socket`, `Listener`). Without the guard
        // `RES s AS udp::Socket = tcp::accept(...)` type-checked -- a TCP stream
        // bound as a datagram socket, silently, surfacing much later as a
        // confusing "NIR declares unused runtime helper" or not at all.
        //
        // Restricted to built-in resources on purpose. A blanket
        // "two qualified names differ" rule is WRONG: a user package imported
        // under an alias yields `comparable.Box` for a type whose defining
        // package spells it `package_comparable_types.Box`, and those are the
        // same type (`syntax/packages/package-comparable-import-invalid` caught
        // exactly that). Built-in resources have no alias path -- they are
        // package-qualified end to end since plan-97/bug-441 -- so for them a
        // differing qualifier really does mean a differing type.
        let distinct_builtin_resources = expected_name != actual_name
            && crate::codegen::resource::is_builtin_resource_type(&expected_name)
            && crate::codegen::resource::is_builtin_resource_type(&actual_name);
        if expected_bare == actual_bare && !distinct_builtin_resources {
            return true;
        }
        // A union accepts any of its variants. A variant may be spelled qualified
        // (a package-scoped resource, `fs.File`) or bare, so match either form.
        if let Some(variants) = self.union_variants(&expected_name) {
            if variants.contains(actual_name.as_ref()) || variants.contains(actual_bare) {
                return true;
            }
        }
        false
    }

    /// the former source checker's `expression_compatible`: `compatible`, plus the literal
    /// coercions that the AST checker allows for constant arguments — a `Byte`
    /// parameter accepts an in-range `Integer` literal, `Fixed` accepts an
    /// `Integer`/`Float` literal. The `Const` node carries the literal type and
    /// value, so the same check applies on the IR.
    pub(super) fn expression_compatible(
        &self,
        expected: &ParameterType,
        actual: &ParameterType,
        value: &IrValue,
    ) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        if let IrValue::Const { type_, value } = value {
            match (expected, type_) {
                (ParameterType::Byte, ParameterType::Integer) => {
                    return value.parse::<u16>().is_ok_and(|n| n <= u8::MAX as u16);
                }
                (ParameterType::Fixed, ParameterType::Integer)
                | (ParameterType::Fixed, ParameterType::Float) => return true,
                // A decimal literal coerces to a Money slot (plan-29-A §4.4).
                (ParameterType::Money, ParameterType::Integer)
                | (ParameterType::Money, ParameterType::Float) => return true,
                _ => {}
            }
        }
        // Negated numeric literal into Fixed / Money (`-1`, `-1.25`).
        if matches!(expected, ParameterType::Fixed | ParameterType::Money) {
            if let IrValue::Unary { op, operand, .. } = value {
                if op == "-"
                    && matches!(operand.as_ref(), IrValue::Const { type_, .. } if matches!(type_, ParameterType::Integer | ParameterType::Float))
                {
                    return true;
                }
            }
        }
        false
    }

    /// Reject a `RETURN <value>` whose value type is incompatible with the
    /// function's declared return type (the former source checker's `TYPE_RETURN_MISMATCH`).
    /// Codegen places the return value into the ABI return slot by the declared
    /// type, so a crafted mismatch is a type confusion at the return boundary.
    pub(super) fn check_return_type(
        &self,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) {
        let expected = self.current_return.borrow().clone();
        if matches!(expected, ParameterType::Nothing | ParameterType::Unknown)
            || expected.name().is_empty()
        {
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
    /// type — the former source checker's `TYPE_BINDING_MISMATCH`. The caller suppresses this
    /// when a literal-range error already fired for the same binding (matching
    /// the former source checker's `!reported_range_error` guard), so a single out-of-range
    /// literal is reported once, as the more specific range error.
    pub(super) fn check_binding_type(
        &self,
        name: &str,
        declared: &ParameterType,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) {
        let expected = resource_base_type(declared);
        if matches!(expected, ParameterType::Nothing | ParameterType::Unknown)
            || expected.name().is_empty()
        {
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
        let actual = resource_base_type(&actual);
        if !self.expression_compatible(&expected, &actual, value) {
            let (actual, expected) = (actual.name(), expected.name());
            self.emit(
                "TYPE_BINDING_MISMATCH",
                format!("Binding `{name}` has initializer type {actual}, expected {expected}."),
            );
        }
    }

    /// Reject a control-flow condition (IF/WHILE/LOOP UNTIL/WHEN guard) whose
    /// type is provably not Boolean — the former source checker's
    /// `TYPE_CONDITION_REQUIRES_BOOLEAN`. `what` is the statement-specific
    /// message prefix (`"IF condition"`, `"WHEN guard"`, …).
    pub(super) fn check_condition_boolean(
        &self,
        what: &str,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) {
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible(&ParameterType::Boolean, &actual, value) {
            self.emit(
                "TYPE_CONDITION_REQUIRES_BOOLEAN",
                format!("{what} has type {actual}, expected Boolean."),
            );
        }
    }

    /// Reject an assignment whose value type is incompatible with the target
    /// binding's settled type — the former source checker's `TYPE_ASSIGNMENT_MISMATCH`. The
    /// caller suppresses this when a literal-range error already fired
    /// (the former source checker's `!reported_range_error` guard). Unlike `TYPE_BINDING_MISMATCH`
    /// no explicit-annotation gate applies: by assignment time the binding's
    /// type is settled regardless of how it was declared.
    pub(super) fn check_assignment_type(
        &self,
        name: &str,
        declared: &ParameterType,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) {
        let expected = resource_base_type(declared);
        if matches!(expected, ParameterType::Nothing | ParameterType::Unknown)
            || expected.name().is_empty()
        {
            return;
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible(&expected, &actual, value) {
            let (actual, expected) = (actual.name(), expected.name());
            self.emit(
                "TYPE_ASSIGNMENT_MISMATCH",
                format!("Assignment to `{name}` has type {actual}, expected {expected}."),
            );
        }
    }

    /// The former source checker's constructor rules on a lowered `Constructor` value: the
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
        locals: &HashMap<String, ParameterType>,
    ) {
        // `Ok`/`Result` are compiler-owned (the former source checker's TYPE_RESULT_IS_IMPLICIT).
        if matches!(type_name, "Ok" | "Result") {
            self.emit(
                "TYPE_RESULT_IS_IMPLICIT",
                format!("`{type_name}` is compiler-owned and cannot be constructed directly."),
            );
            return;
        }
        // `AttributedString` is an opaque built-in with no user-visible fields
        // (plan-89-A): it is created with `astrings::fromString(text)`, never
        // with `AttributedString[...]` (the source checker's arm, plan-107-D).
        if type_name == "AttributedString" {
            self.emit(
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                "`AttributedString` is an opaque built-in type and cannot be constructed; use `astrings::fromString(text)` to create one.".to_string(),
            );
            return;
        }
        // Compiler-owned records may never be user-constructed
        // (TYPE_READ_ONLY_RECORD_CONSTRUCTOR). The Error/ErrorLoc arm of that
        // rule is `ir::shape`'s: lowering itself emits `Constructor{Error}`
        // for the `error()` builtin and trap machinery, so on the IR a user
        // `Error[..]` is indistinguishable from a legitimate synthesized one.
        if read_only_record_type(&ParameterType::parse(type_name)) {
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
        // from its declaring file (the former source checker's TYPE_MEMBER_NOT_VISIBLE arms).
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
    /// union (a value smuggled under a tag the union does not define), or whose
    /// wrapped `value` does not have the `member_type` it is tagged with. The
    /// tag check alone left the payload unreconciled: a crafted `.mfp` could wrap
    /// an Integer under a record variant, and a later MATCH/`UnionExtract` would
    /// read that variant's record layout off the Integer (bug-404 — the wrap-side
    /// counterpart of `check_union_extract`, the read side bug-162 guarded). The
    /// payload reconciliation is skipped when the value's type is unknown, so
    /// legitimate IR — whose `member_type` is the wrapped value's own type
    /// (`lower.rs:3312`) — never rejects.
    pub(super) fn check_union_wrap(
        &self,
        union_type: &str,
        member_type: &str,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) {
        if member_type.is_empty() {
            return;
        }
        if let Some(variants) = self.union_variants(union_type) {
            if !variants.contains(member_type) {
                self.emit(
                    VERIFY_TYPE,
                    format!("`{member_type}` is not a variant of union `{union_type}`"),
                );
                return;
            }
        }
        if let Some(actual) = self.infer_type(value, locals) {
            if !self.expression_compatible(&ParameterType::parse(member_type), &actual, value) {
                let actual = actual.name();
                self.emit(
                    VERIFY_TYPE,
                    format!(
                        "UnionWrap payload has type {actual}, expected variant `{member_type}`"
                    ),
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
        locals: &HashMap<String, ParameterType>,
    ) {
        if type_.is_empty() {
            return;
        }
        let Some(union_type) = self.infer_type(value, locals) else {
            return;
        };
        let union_type = resource_base_type(&union_type).name();
        if let Some(variants) = self.union_variants(&union_type) {
            if !variants.contains(type_) {
                self.emit(
                    VERIFY_TYPE,
                    format!("`{type_}` is not a variant of union `{union_type}`"),
                );
            }
        }
    }

    /// the former source checker's TYPE_LAMBDA_CAPTURE_UNSUPPORTED at the `Closure` use site.
    /// Lowering's capture list is the front end's (`captured_locals` + the
    /// assignment target, in that order), and the licence a capture was given
    /// survives as its SHAPE: a `LocalRef` is the compiler-proven non-escaping
    /// `MUT` by-ref capture, a `Local` is an ordinary by-value copy. So a
    /// by-value capture of a `MUT` local is the "mutable capture" rejection, a
    /// resource is rejected in either shape (§12.4), and a by-value capture must
    /// be copyable.
    pub(super) fn check_closure_captures(
        &self,
        captures: &[IrValue],
        locals: &HashMap<String, ParameterType>,
    ) {
        let muts = self.current_muts.borrow();
        for capture in captures {
            let (name, by_ref) = match capture {
                IrValue::Local(name) => (name, false),
                IrValue::LocalRef { name, .. } => (name, true),
                _ => continue,
            };
            let Some(type_) = locals.get(name) else {
                continue;
            };
            let mutable = muts.get(name).copied().unwrap_or(false);
            if mutable && !by_ref {
                self.emit(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    format!(
                        "Lambda captures mutable local `{name}`; mutable captures are invalid."
                    ),
                );
            } else if self.is_resource_type(type_) {
                self.emit(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    format!(
                        "Lambda captures resource local `{name}`; resource captures are invalid."
                    ),
                );
            } else if !mutable && !self.is_copyable(type_, &mut HashSet::new()) {
                self.emit(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    format!(
                        "Lambda captures non-copyable local `{name}` of type `{}`; non-copyable captures are invalid.",
                        type_.name()
                    ),
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
