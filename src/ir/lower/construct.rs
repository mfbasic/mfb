//! plan-176-N: IR lowering of constructed values — error values, union wrapping and conversion,
//! and constructor arguments. The lowering context and entry points live in `ir/lower/mod.rs`.

use super::*;

/// Build an `ErrorLoc` record value for a compile-time source location.
fn error_loc_value(file: &str, loc: IrSourceLoc) -> IrValue {
    IrValue::Constructor {
        type_: ParameterType::named("ErrorLoc"),
        args: vec![
            IrValue::Const {
                type_: ParameterType::String,
                value: file.to_string(),
            },
            IrValue::Const {
                type_: ParameterType::Integer,
                value: loc.line.to_string(),
            },
            IrValue::Const {
                type_: ParameterType::Integer,
                value: loc.column.to_string(),
            },
        ],
    }
}

/// Build an `Error` record value (code, message, source) for `error(...)`.
pub(super) fn build_error_value(
    code: IrValue,
    message: IrValue,
    file: &str,
    loc: IrSourceLoc,
) -> IrValue {
    IrValue::Constructor {
        type_: ParameterType::named("Error"),
        args: vec![code, message, error_loc_value(file, loc)],
    }
}

/// Wrap `base`, the lowering of `expression`, into the expected union when the
/// expression's static type is one of that union's variants (data or resource).
///
/// Called only by [`lower_expression_with_expected`], so every expected-type
/// position and every expression kind gets the wrap (bug-990). Idempotent: no
/// expected type, an already-wrapped value, an untypeable expression, or a type
/// that is not a variant of the WHOLE expected type (a `List OF Square` at a
/// `List OF Shape` slot, a union at its own union) all return `base` unchanged.
pub(super) fn wrap_union_value(
    base: IrValue,
    expression: &HirExpression,
    expected: Option<&ParameterType>,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    let Some(expected) = expected else {
        return base;
    };
    // Avoid double-wrapping when the value's own lowering already wrapped it
    // (e.g. a variant constructor assigned to a union-typed binding).
    if matches!(base, IrValue::UnionWrap { .. }) {
        return base;
    }
    // A list literal was lowered element by element against the expected element
    // type, and a map literal against its own annotation, so each already HAS the
    // type its lowering wrote; every other value has its expression's type.
    let actual_type = match &base {
        IrValue::ListLiteral { type_, .. }
        | IrValue::SetLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => type_.clone(),
        _ => match expression_type(expression, locals, context) {
            Some(type_) => type_,
            None => return base,
        },
    };
    union_conversion(base, &actual_type, expected, context)
}

/// The type a union's variant table knows a value by: without the `RES` alias
/// marker and without a `STATE` clause.
pub(super) fn bare_variant(type_: &ParameterType) -> ParameterType {
    match type_ {
        ParameterType::Res(inner) => inner.without_state(),
        other => other.without_state(),
    }
}

/// The `STATE` a value of `type_` carries, looking through the `RES` marker.
fn bare_payload_state(type_: &ParameterType) -> Option<ParameterType> {
    match type_ {
        ParameterType::Res(inner) => inner.state(),
        other => other.state(),
    }
}

/// The union form of `value` (of type `actual`) at a position expecting
/// `expected`, or `value` unchanged when the two are not related by union-variant
/// subsumption.
///
/// * A variant of the expected union is tagged in place: `UnionWrap`.
/// * A `List` whose elements convert, or a `Map` whose values do, becomes a NEW
///   collection with every element converted (bug-989, owner decision
///   2026-10-10): a `List OF Square` where a `List OF Shape` is expected is the
///   same subsumption as one `Square` where a `Shape` is expected, applied to each
///   element, at every nesting level. Nothing rewraps a payload in place — an
///   untagged record block and a `{tag, size, data}` block differ in size — so the
///   conversion is one O(n) copy per position, made by an ordinary call that the
///   statement frees like any other temporary. The source is not changed.
///
/// A collection already of the expected type is `value` itself: no call, no copy.
pub(super) fn union_conversion(
    value: IrValue,
    actual: &ParameterType,
    expected: &ParameterType,
    context: &mut LowerContext<'_>,
) -> IrValue {
    // A `List OF RES Handle` element position expects `RES Handle`: the `RES`
    // marks ownership, the union is `Handle`, exactly as a `RES h AS Handle`
    // binding's lowered type is the bare `Handle`.
    let union_type = match expected {
        ParameterType::Res(inner) => inner.as_ref(),
        other => other,
    };
    if context
        .type_index
        .variant_belongs_to_union(actual, union_type)
    {
        // The `STATE` rides the resource record, not the tag: a wrap of a value
        // that carries one is a union value carrying the same one (bug-1063), so
        // the agreement checks see what the payload really has.
        let union_type = match (union_type.state(), bare_payload_state(actual)) {
            (None, Some(state)) => union_type.with_state(&state),
            _ => union_type.clone(),
        };
        return IrValue::UnionWrap {
            union_type,
            // The variant the union's table knows (bug-1018): the wrap's tag is
            // the bare variant's, whatever marker or `STATE` the value carries.
            member_type: bare_variant(actual),
            value: Box::new(value),
        };
    }
    if !context
        .type_index
        .union_collection_converts(actual, expected)
    {
        return value;
    }
    let loc = context.current_loc;
    match (actual, expected) {
        (ParameterType::ListOf(from), ParameterType::ListOf(to)) => {
            // bug-1018: the converter takes and answers the element itself; the
            // `RES` marker is the slot's spelling, not part of the element's type,
            // as a `RES h AS Handle` binding's lowered type is the bare `Handle`.
            let (from, to) = match (from.as_ref(), to.as_ref()) {
                (ParameterType::Res(from), ParameterType::Res(to)) => (from, to),
                _ => (from, to),
            };
            let converter = union_converter(from, to, context);
            IrValue::Call {
                target: "collections.transform".to_string(),
                args: vec![
                    value,
                    IrValue::FunctionRef {
                        name: converter,
                        type_: ParameterType::func(vec![(**from).clone()], (**to).clone()),
                    },
                ],
                type_: expected.clone(),
                loc,
            }
        }
        _ => IrValue::Call {
            target: union_converter(actual, expected, context),
            args: vec![value],
            type_: expected.clone(),
            loc,
        },
    }
}

/// The lifted function `(value AS actual) AS expected` that converts one value
/// for [`union_conversion`], created on first use. Its body is the conversion
/// itself: a wrap for a variant, a `collections::transform` for a list, and for a
/// map a loop that sets each key to its converted value (a map has no native
/// value-mapping call to lower to).
fn union_converter(
    actual: &ParameterType,
    expected: &ParameterType,
    context: &mut LowerContext<'_>,
) -> String {
    let key = (actual.clone(), expected.clone());
    if let Some(name) = context.union_converters.get(&key) {
        return name.clone();
    }
    let name = format!("$lambda{}", context.next_lambda_id);
    context.next_lambda_id += 1;
    context.union_converters.insert(key, name.clone());
    let loc = context.current_loc;
    let value = "value".to_string();
    let body = match (actual, expected) {
        (ParameterType::MapOf(key_type, from), ParameterType::MapOf(_, to)) => {
            let converted = "converted".to_string();
            let entry = "entry".to_string();
            let item = union_conversion(
                IrValue::MemberAccess {
                    target: Box::new(IrValue::Local(entry.clone())),
                    member: "value".to_string(),
                    type_: (**from).clone(),
                },
                from,
                to,
                context,
            );
            vec![
                IrOp::Bind {
                    mutable: true,
                    name: converted.clone(),
                    type_: expected.clone(),
                    value: Some(IrValue::MapLiteral {
                        type_: expected.clone(),
                        entries: Vec::new(),
                    }),
                    explicit_type: false,
                    loc,
                },
                IrOp::ForEach {
                    name: entry.clone(),
                    type_: ParameterType::map_entry_of((**key_type).clone(), (**from).clone()),
                    iterable: IrValue::Local(value.clone()),
                    body: vec![IrOp::Assign {
                        name: converted.clone(),
                        value: IrValue::Call {
                            target: "collections.set".to_string(),
                            args: vec![
                                IrValue::Local(converted.clone()),
                                IrValue::MemberAccess {
                                    target: Box::new(IrValue::Local(entry)),
                                    member: "key".to_string(),
                                    type_: (**key_type).clone(),
                                },
                                item,
                            ],
                            type_: expected.clone(),
                            loc,
                        },
                        loc,
                    }],
                    loc,
                },
                IrOp::Return {
                    value: Some(IrValue::Local(converted)),
                    loc,
                },
            ]
        }
        _ => vec![IrOp::Return {
            value: Some(union_conversion(
                IrValue::Local(value.clone()),
                actual,
                expected,
                context,
            )),
            loc,
        }],
    };
    context.lambdas.push(IrFunction {
        name: name.clone(),
        visibility: "private".to_string(),
        kind: "func".to_string(),
        isolated: false,
        params: vec![IrParam {
            name: value,
            type_: actual.clone(),
            default: None,
            loc,
        }],
        returns: expected.clone(),
        body,
        file: context.current_file.clone(),
        loc,
        resource_owners: HashMap::new(),
    });
    name
}

pub(super) fn lower_constructor_args(
    record_type: &ParameterType,
    arguments: &[HirConstructorArg],
    fields: Option<&Vec<IrField>>,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<IrValue> {
    let Some(fields) = fields else {
        return arguments
            .iter()
            .map(|argument| lower_expression(constructor_arg_value(argument), locals, context))
            .collect();
    };
    // A by-field constructor names at least one field: `Ck[]` names none, so it is the
    // zero-argument positional form and keeps its arity check (owner's rule covers a
    // constructor that names SOME fields; coordinator review, bug-staging-24).
    if !arguments.is_empty()
        && arguments
            .iter()
            .all(|argument| matches!(argument, HirConstructorArg::Named { .. }))
    {
        // bug-1021: a by-field constructor may name SOME of the fields; each field it
        // leaves out takes its default — the value an uninitialized `MUT x AS T`
        // holds. That only applies when every name written IS a field: a stray name
        // keeps today's diagnostics (the arity and argument mismatches), not a
        // silent default for the field it was meant to be. A field with no default
        // is refused by the uninitialized-`MUT` rule on the synthesized default).
        let names_a_field = |name: &str| fields.iter().any(|field| field.name == name);
        let every_name_is_a_field = arguments.iter().all(|argument| match argument {
            HirConstructorArg::Named { name, .. } => names_a_field(name),
            HirConstructorArg::Positional(_) => false,
        });
        return fields
            .iter()
            .filter_map(|field| {
                let named = arguments.iter().find_map(|argument| match argument {
                    HirConstructorArg::Named { name, value, .. } if name == &field.name => {
                        Some(value)
                    }
                    _ => None,
                });
                match named {
                    Some(value) => Some(lower_expression_with_expected(
                        value,
                        Some(&field.type_),
                        locals,
                        context,
                    )),
                    None if every_name_is_a_field => {
                        Some(omitted_field_default(record_type, field, context))
                    }
                    None => None,
                }
            })
            .collect();
    }
    arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let expected = fields.get(index).map(|field| &field.type_);
            lower_expression_with_expected(
                constructor_arg_value(argument),
                expected,
                locals,
                context,
            )
        })
        .collect()
}

/// bug-1021: the value of a field a by-field constructor leaves out — the default of
/// its type, exactly what an uninitialized `MUT x AS T` holds. There is ONE
/// definition of that default, the one codegen materializes for a binding with no
/// initializer, so this does not restate it: it calls a synthesized function whose
/// whole body is `MUT <field> AS T` and `RETURN <field>`. A type with no default is
/// refused by the ordinary uninitialized-`MUT` rule (`TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`)
/// on that binding, which carries the field's name.
fn omitted_field_default(
    record_type: &ParameterType,
    field: &IrField,
    context: &mut LowerContext<'_>,
) -> IrValue {
    let key = (record_type.clone(), field.name.clone());
    let name = match context.field_defaults.get(&key) {
        Some(name) => name.clone(),
        None => {
            let name = format!("$lambda{}", context.next_lambda_id);
            context.next_lambda_id += 1;
            let loc = context.current_loc;
            context.field_defaults.insert(key, name.clone());
            context.lambdas.push(IrFunction {
                name: name.clone(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: field.type_.clone(),
                body: vec![
                    IrOp::Bind {
                        mutable: true,
                        name: field.name.clone(),
                        type_: field.type_.clone(),
                        value: None,
                        explicit_type: true,
                        loc,
                    },
                    IrOp::Return {
                        value: Some(IrValue::Local(field.name.clone())),
                        loc,
                    },
                ],
                file: context.current_file.clone(),
                loc,
                resource_owners: HashMap::new(),
            });
            name
        }
    };
    IrValue::Call {
        target: name,
        args: Vec::new(),
        type_: field.type_.clone(),
        loc: context.current_loc,
    }
}

pub(super) fn constructor_arg_value(argument: &HirConstructorArg) -> &HirExpression {
    match argument {
        HirConstructorArg::Positional(value) => value,
        HirConstructorArg::Named { value, .. } => value,
    }
}
