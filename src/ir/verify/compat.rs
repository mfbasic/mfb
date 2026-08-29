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
            let expected = if min == max {
                min.to_string()
            } else {
                format!("{min} to {max}")
            };
            self.emit(
                "TYPE_CALL_ARITY_MISMATCH",
                format!("Call to `{target}` has {actual} argument(s), expected {expected}."),
            );
            true
        } else {
            false
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
        locals: &HashMap<String, ParameterType>,
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
                    if let ParameterType::ListOf(element) = resource_base_type(&t) {
                        if !matches!(*element, ParameterType::Unknown)
                            && !self.is_comparable(&element)
                        {
                            let element = element.name();
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
        let arg_types: Option<Vec<ParameterType>> = args
            .iter()
            .map(|a| self.infer_type(a, locals).map(|t| resource_base_type(&t)))
            .collect();
        let Some(arg_types) = arg_types else {
            return;
        };
        // The diagnostics below quote the argument list; render once, here.
        let arg_type_names = || {
            arg_types
                .iter()
                .map(|t| t.name().into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        };
        // `term` exposes its per-name signatures (`arity`, machine `argument_types`)
        // rather than an arg-typed `resolve_call`, so check against those with
        // the ported `expression_compatible` — the same data syntaxcheck's
        // `check_term_builtin_call` uses, so term's signature is single-source.
        if crate::codegen::registry::registry().owning_package(target) == Some("term") {
            if let Some((min, max)) = builtins::arity(target) {
                if self.builtin_arity_errored(target, arg_types.len(), min, max) {
                    return;
                }
            }
            let params = builtins::argument_types_typed(target).unwrap_or_default();
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
        // Every collections member is a registered function now — the native members
        // (`get`, …) and the source generics (`sort`, …, `Body::Mfb` descriptors) —
        // scoped to collections so the other migrated packages (csv/json/…) still
        // fall through as before.
        if crate::codegen::registry::registry().owning_package(target) == Some("collections") {
            if let Some((min, max)) = builtins::arity(target) {
                if self.builtin_arity_errored(target, arg_types.len(), min, max) {
                    return;
                }
            }
            if builtins::resolve_call_return_type_typed(target, &arg_types, false).is_none() {
                let expected = builtins::expected_arguments(target)
                    .unwrap_or_else(|| "supported overload".to_string());
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{target}` has argument type(s) ({}), expected {expected}.",
                        arg_type_names()
                    ),
                );
            }
            return;
        }
        if crate::codegen::builtins::general::is_general_call(target) {
            if let Some((min, max)) = builtins::arity(target) {
                if self.builtin_arity_errored(target, arg_types.len(), min, max) {
                    return;
                }
            }
            if builtins::resolve_call_return_type_typed(target, &arg_types, false).is_none() {
                // A package-provided override may accept what the built-in
                // rejects (plan-01-overload §A.3.2) — never reject those.
                if crate::codegen::builtins::general::is_overridable(target)
                    && arg_types.len() == 1
                    && builtins::general_override_target(target, &arg_types[0].name()).is_some()
                {
                    return;
                }
                let expected = builtins::expected_arguments(target)
                    .unwrap_or_else(|| "supported overload".to_string());
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{target}` has argument type(s) ({}), expected {expected}.",
                        arg_type_names()
                    ),
                );
            }
            return;
        }
        // The arg-typed packages checked here (bug-342 A10). Each pairs its
        // membership test with an overload-resolution probe; a non-capturing
        // closure erases each package's distinct `ResolvedCall<'_>` type to a
        // plain `bool`. This set is deliberately narrower than
        // `resolve_call_return_type`'s (no term/collections/general — handled
        // above — and no crypto/json/csv/…), so it must stay an explicit table:
        // widening it would reject programs codegen currently accepts.
        // plan-72-BB: the narrow membership stays an explicit list (widening it
        // would reject programs codegen accepts — see the note above), but the
        // per-package `resolve_call` probes collapse to the registry aggregate,
        // which resolves each of these packages byte-identically to its own
        // `resolve_call` (proven by the descriptor parity tests + artifact-gate).
        type IsCall = fn(&str) -> bool;
        // `encoding` migrated to the clean-room registry; its membership is the
        // narrow `owning_package == "encoding"` (not the broad `registry::is_member`,
        // which would widen this set to every migrated package and reject programs
        // codegen accepts — see the note above).
        fn is_encoding_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("encoding")
        }
        // `bits` migrated to the clean-room registry; its membership is the narrow
        // `owning_package == "bits"` (mirroring `is_encoding_call`), replacing the
        // deleted `builtins::bits::is_bits_call`.
        fn is_bits_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("bits")
        }
        // `os` migrated to the clean-room registry; its membership is the narrow
        // `owning_package == "os"` (mirroring `is_bits_call`/`is_encoding_call`),
        // replacing the deleted `builtins::os::is_os_call`.
        fn is_os_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("os")
        }
        // `fs` migrated to the clean-room registry; membership is the narrow
        // `owning_package == "fs"`, replacing the deleted `builtins::fs::is_fs_call`.
        fn is_fs_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("fs")
        }
        // `io` migrated to the clean-room registry; membership is the narrow
        // `owning_package == "io"`, replacing the deleted `builtins::io::is_io_call`.
        fn is_io_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("io")
        }
        // `math` migrated to the clean-room registry; membership is the narrow
        // `owning_package == "math"`, replacing the deleted `builtins::math::is_math_call`.
        fn is_math_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("math")
        }
        // `vector` migrated to the clean-room registry; membership is the narrow
        // `owning_package == "vector"` (function members only — constants are folded, not
        // called), replacing the deleted `builtins::vector::is_vector_call`.
        fn is_vector_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("vector")
        }
        // `net` migrated to the clean-room registry — membership via `owning_package`,
        // replacing the deleted `builtins::net::is_net_call`.
        fn is_net_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("net")
        }
        // `strings` migrated to the clean-room registry (plan-99 PART B) — membership
        // via `owning_package`, replacing the deleted `builtins::strings::is_strings_call`.
        fn is_strings_call(name: &str) -> bool {
            crate::codegen::registry::registry().owning_package(name) == Some("strings")
        }
        let checked: [IsCall; 9] = [
            is_math_call,
            is_bits_call,
            is_vector_call,
            is_strings_call,
            is_encoding_call,
            is_io_call,
            is_fs_call,
            is_net_call,
            is_os_call,
        ];
        if checked.iter().any(|is_call| is_call(target))
            && builtins::resolve_call_return_type_typed(target, &arg_types, false).is_none()
        {
            self.emit(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                format!("Arguments to `{target}` do not match any overload."),
            );
        }
    }

    // ===========================================================================
    // 12. Compatibility + typed statement checks
    // ===========================================================================

    /// Type compatibility (`syntaxcheck::compatible`). `Unknown` on either side is
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
        if expected_bare == actual_bare {
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

    /// `syntaxcheck::expression_compatible`: `compatible`, plus the literal
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
    /// function's declared return type (`syntaxcheck`'s `TYPE_RETURN_MISMATCH`).
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
    /// type — `syntaxcheck`'s `TYPE_BINDING_MISMATCH`. The caller suppresses this
    /// when a literal-range error already fired for the same binding (matching
    /// syntaxcheck's `!reported_range_error` guard), so a single out-of-range
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
    /// type is provably not Boolean — `syntaxcheck`'s
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
    /// binding's settled type — `syntaxcheck`'s `TYPE_ASSIGNMENT_MISMATCH`. The
    /// caller suppresses this when a literal-range error already fired
    /// (syntaxcheck's `!reported_range_error` guard). Unlike `TYPE_BINDING_MISMATCH`
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
        locals: &HashMap<String, ParameterType>,
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

    /// syntaxcheck's TYPE_LAMBDA_CAPTURE_UNSUPPORTED at the `Closure` use site.
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
