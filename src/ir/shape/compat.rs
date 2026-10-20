//! plan-176-N: the shape checker's type compatibility — comparability, resource types and
//! assignability. The `Walker` struct and the free helpers live in `ir/shape/mod.rs`.

use super::*;

impl<'a> Walker<'a> {
    /// Whether a type is an `=`-comparable operand, as the source checker
    /// judged it (the former source checker's `is_comparable_with_seen`): primitives,
    /// `Error`/`ErrorLoc`/`Scalar`, enums, records of comparable fields, and any
    /// nominal it cannot resolve; never a collection, a callable, a result, a
    /// thread handle, a resource, a union or `AttributedString`.
    pub(super) fn is_comparable(&self, type_: &ParameterType) -> bool {
        self.is_comparable_seen(type_, &mut HashSet::new())
    }

    fn is_comparable_seen(&self, type_: &ParameterType, seen: &mut HashSet<ParameterType>) -> bool {
        match type_ {
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::Unknown => true,
            ParameterType::ListOf(_)
            | ParameterType::SetOf(_)
            | ParameterType::MapOf(_, _)
            | ParameterType::Func(..)
            | ParameterType::ResultOf(_)
            | ParameterType::Res(_)
            | ParameterType::ThreadHandle { .. } => false,
            ParameterType::Named(_) => {
                // plan-111-B: four nominals, four interned-`Symbol` compares.
                if type_.is_named("Error") || type_.is_named("ErrorLoc") || type_.is_named("Scalar")
                {
                    return true;
                }
                // Wraps a list overlay (like `List`), plan-89-A.
                if type_.is_named("AttributedString") {
                    return false;
                }
                let name = type_.clone();
                if self.is_resource_type(type_) || !seen.insert(name.clone()) {
                    return false;
                }
                let Some(shape) = self.types.get(type_) else {
                    return true;
                };
                let result = if shape.is_enum {
                    true
                } else if shape.is_record {
                    shape
                        .fields
                        .iter()
                        .all(|field| self.is_comparable_seen(field, seen))
                } else {
                    false
                };
                seen.remove(&name);
                result
            }
            // A stateful resource is a resource, so it is not comparable —
            // plan-111-B: the same trap as `validate_package_type`'s `Stateful`
            // arm. The `other` arm below re-wraps into one opaque nominal, and
            // the structural `without_state` cannot peel a re-wrapped spelling,
            // so a `Db STATE DbInfo` would have fallen through to the
            // unknown-type tail and been called COMPARABLE. Peel before the
            // re-wrap. A stateful type whose base is not a resource still falls
            // through, exactly as its opaque nominal did.
            ParameterType::Stateful { .. } if self.is_resource_type(type_) => false,
            // The checker routed every other spelling through its nominal arm.
            other => self.is_comparable_seen(&ParameterType::named(&other.name()), seen),
        }
    }

    /// The checker's resource registry: builtin resources plus the native and
    /// imported ones this project declares/imports (an imported resource is
    /// spelled `binding.Type` in source; its table row carries the bare name).
    /// A stateful resource carries its ` STATE T` clause inside the spelling
    /// (`SoundFile STATE SoundInfo`); recognition keys on the base name.
    pub(super) fn is_resource_type(&self, type_: &ParameterType) -> bool {
        // plan-111-B: the STATE peel is structural (`without_state`); the
        // builtin close table is registry surface that still speaks names, so
        // the base renders for that one lookup and for the bare-name fallback
        // (an imported resource is spelled `binding.Type` in source while its
        // table row carries the bare name).
        let base = type_.without_state();
        let base_name = base.name();
        crate::codegen::resource::is_builtin_resource(&base)
            || self.resource_types.contains(&base)
            || base_name.rsplit_once('.').is_some_and(|(_, bare)| {
                self.resource_types.contains(&ParameterType::declared(bare))
            })
    }

    /// Type compatibility as the source checker judged it (the former source checker's `compatible`):
    /// `Unknown` on either side is compatible; the `RES` marker is stripped;
    /// containers, thread handles and callable types recurse (parameters
    /// contravariant); a union accepts any of its variants; a qualified nominal
    /// equates to its bare form unless both name distinct declarations.
    pub(super) fn compatible(&self, expected: &ParameterType, actual: &ParameterType) -> bool {
        if matches!(expected, ParameterType::Unknown) || matches!(actual, ParameterType::Unknown) {
            return true;
        }
        // The `RES` marker and a resource's ` STATE T` clause are both
        // ownership-axis annotations the checker compared without.
        let expected = strip_res(expected).without_state();
        let actual = strip_res(actual).without_state();
        match (&expected, &actual) {
            (ParameterType::ListOf(expected), ParameterType::ListOf(actual))
            | (ParameterType::SetOf(expected), ParameterType::SetOf(actual))
            | (ParameterType::ResultOf(expected), ParameterType::ResultOf(actual)) => {
                self.compatible(expected, actual)
            }
            (
                ParameterType::MapOf(expected_key, expected_value),
                ParameterType::MapOf(actual_key, actual_value),
            ) => {
                self.compatible(expected_key, actual_key)
                    && self.compatible(expected_value, actual_value)
            }
            (
                ParameterType::ThreadHandle {
                    worker: expected_worker,
                    msg: expected_message,
                    res: expected_resource,
                    out: expected_output,
                },
                ParameterType::ThreadHandle {
                    worker: actual_worker,
                    msg: actual_message,
                    res: actual_resource,
                    out: actual_output,
                },
            ) => {
                expected_worker == actual_worker
                    && self.compatible(expected_message, actual_message)
                    && self.compatible(expected_resource, actual_resource)
                    && self.compatible(expected_output, actual_output)
            }
            (
                ParameterType::Func(expected_params, expected_return, expected_isolated),
                ParameterType::Func(actual_params, actual_return, actual_isolated),
            ) => {
                (!expected_isolated || *actual_isolated)
                    && expected_params.len() == actual_params.len()
                    && expected_params
                        .iter()
                        .zip(actual_params.iter())
                        // Parameters are contravariant (bug-173 A). A
                        // lambda's `STATE T` parameter is part of its callable
                        // type (bug-1030) and is not an annotation to compare
                        // without here: a `FUNC(File STATE Cur)` reads its
                        // argument's state, so it is not a `FUNC(File)`, whose
                        // callers may pass any state or none.
                        .all(|(expected, actual)| {
                            strip_res(expected).state() == strip_res(actual).state()
                                && self.compatible(actual, expected)
                        })
                    && self.compatible(expected_return, actual_return)
            }
            (ParameterType::Named(_), ParameterType::Named(_)) => {
                let (expected_name, actual_name) = (expected.name(), actual.name());
                if expected_name == actual_name {
                    return true;
                }
                // bug-632: a BUILT-IN qualifier only, as in
                // `ir::verify::compat::compatible` — a user package's type is
                // package-qualified end to end, and equating `ov.A` with a local
                // `A` by the last segment is the confusion that qualification
                // prevents.
                let expected_bare =
                    crate::codegen::builtins::builtin_qualified_bare_leaf(&expected_name);
                let actual_bare =
                    crate::codegen::builtins::builtin_qualified_bare_leaf(&actual_name);
                let expected_info = self
                    .types
                    .get(&expected)
                    .or_else(|| self.types.get(&ParameterType::declared(expected_bare)));
                // A union accepts any of its variant values; a variant may be
                // spelled qualified (`fs.File`) or bare.
                if expected_info.is_some_and(|info| {
                    info.variants
                        .iter()
                        .any(|variant| variant == actual_name.as_ref() || variant == actual_bare)
                }) {
                    return true;
                }
                if expected_bare != actual_bare {
                    return false;
                }
                // Shared bare names unify only when both resolve to the SAME
                // declaration (bug-41), or when either side is unregistered (a
                // built-in nominal such as `net.Url`).
                let actual_info = self
                    .types
                    .get(&actual)
                    .or_else(|| self.types.get(&ParameterType::declared(actual_bare)));
                match (expected_info, actual_info) {
                    (Some(expected_info), Some(actual_info)) => expected_info.id == actual_info.id,
                    _ => true,
                }
            }
            _ => expected == actual,
        }
    }

    /// the former source checker's `expression_compatible`: `compatible`, plus the literal
    /// coercions a constant argument enjoys — an in-range integer literal into
    /// `Byte`, a numeric literal (negated or not) into `Fixed`/`Money`, and a
    /// list literal of such literals into a list of them.
    pub(super) fn expression_compatible(
        &self,
        expected: &ParameterType,
        actual: &ParameterType,
        expression: &HirExpression,
    ) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        // bug-820: a `Float`-suffixed literal (`1.5f`) is a Float, not an
        // untyped literal. A Fixed slot still accepts it (lowering converts it as
        // `toFixed(Float)` would); a Money slot does not, because nothing
        // converts into Money implicitly (`mfb spec language types`, `Money`).
        let float_suffixed_into_money = |number: &HirExpression| {
            matches!(expected, ParameterType::Money)
                && matches!(number, HirExpression::Number(text) if text.ends_with('f'))
        };
        match (expected, actual, expression) {
            (ParameterType::Byte, ParameterType::Integer, HirExpression::Number(value)) => value
                .parse::<u16>()
                .is_ok_and(|number| number <= u8::MAX as u16),
            (
                ParameterType::Fixed | ParameterType::Money,
                ParameterType::Integer | ParameterType::Float,
                HirExpression::Number(_),
            ) => !float_suffixed_into_money(expression),
            (
                ParameterType::Fixed | ParameterType::Money,
                ParameterType::Integer | ParameterType::Float,
                HirExpression::Unary {
                    operator, operand, ..
                },
            ) if *operator == UnaryOp::Negate
                && matches!(operand.as_ref(), HirExpression::Number(_)) =>
            {
                !float_suffixed_into_money(operand)
            }
            (
                ParameterType::ListOf(expected_element),
                ParameterType::ListOf(_),
                HirExpression::ListLiteral(values),
            ) => values.iter().all(|value| {
                let Some(actual_element) = numeric_literal_type(value) else {
                    return false;
                };
                self.expression_compatible(expected_element, &actual_element, value)
            }),
            _ => false,
        }
    }
}
