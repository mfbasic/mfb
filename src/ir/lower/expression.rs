//! plan-176-N: IR lowering of expressions. The lowering context and entry points live in
//! `ir/lower/mod.rs`.

use super::*;

pub(super) fn lower_expression(
    expression: &HirExpression,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    lower_expression_with_expected(expression, None, locals, context)
}

/// The predicate's function type for a `filter`/`forEach`-style callback whose
/// collection argument has type `collection_type`: take the element type and ask
/// `filter_predicate_type_typed` for the bare predicate's signature. `None` when
/// the argument is not a `List OF …` or the name is not a resolvable general
/// built-in predicate (bug-342 A6 — was written inline at three call sites).
///
/// plan-106-A: matches the `ListOf` variant instead of `strip_prefix("List OF ")`.
pub(super) fn filter_predicate_arg_type(
    predicate: &str,
    collection_type: &ParameterType,
) -> Option<ParameterType> {
    match collection_type {
        ParameterType::ListOf(element) => {
            crate::codegen::builtins::general::filter_predicate_type_typed(predicate, element)
        }
        _ => None,
    }
}

/// The constructor a **migrated** package's record constant `name`
/// (`"vector.zeroFloat3"`) inlines to, reconstructed from its
/// [`RegistryConstant`](crate::codegen::registry) — the flat per-field literals paired
/// with the element types read from the package's record fields, in order. `None` for a
/// scalar constant or a package that declares no record constant of that name (the
/// migrated `vector` package is the sole record-constant provider today).
pub(super) fn registry_record_constant(name: &str) -> Option<IrValue> {
    use crate::codegen::registry;
    let components = registry::constant_components(name)?;
    let type_name = registry::constant_type_name(name)?;
    // Each component's element type comes from the record's declared fields, in order.
    let (package, _) = name.split_once('.')?;
    // plan-106-A: the record's declared field types are cloned from the descriptor
    // rather than rendered and re-parsed. The record TYPE is a nominal, so it is
    // built with `named` (a record constant never names a generic).
    // bug-480 Phase 4b: `constant_type_name` returns the QUALIFIED identity
    // (`vector.Float3`) now, so prefixing the package again produced
    // `vector.vector.Float3`, resolved to nothing, and dropped the record constant
    // through to the scalar-fold path -- which panicked on a constant that has no
    // single literal value.
    let qualified_type = if type_name.name().contains('.') {
        type_name.name().into_owned()
    } else {
        format!("{package}.{type_name}")
    };
    let field_types: Vec<ParameterType> =
        match registry::registry().resolve_type(&qualified_type)? {
            registry::ResolvedType::Record(record) => {
                record.props.iter().map(|prop| prop.ty.clone()).collect()
            }
            _ => return None,
        };
    Some(IrValue::Constructor {
        type_: type_name,
        args: components
            .iter()
            .enumerate()
            .map(|(index, value)| IrValue::Const {
                type_: field_types
                    .get(index)
                    .cloned()
                    .unwrap_or(ParameterType::Unknown),
                value: value.to_string(),
            })
            .collect(),
    })
}

/// The function type to give a general built-in predicate named in a value
/// position, when `expected` is a concrete unary Boolean function type it
/// accepts (bug-368).
///
/// Mirrors the former source checker's `builtin_predicate_value_type`: both consult
/// `filter_predicate_type`, so the type the checker assigns and the type the
/// `FunctionRef` carries cannot diverge. A divergence would emit a wrapper under
/// one symbol and reference another.
/// plan-106-A: the unary-predicate shape is matched on the expected type's
/// [`Func`](ParameterType::Func) variant, deleting the private
/// `function_type_parts_for_predicate` splitter this used to call — a
/// `strip_prefix("FUNC(")` + `split_once(") AS ")` copy of the type grammar that
/// mis-cut a higher-order parameter (it split at the FIRST `") AS "`).
fn builtin_predicate_ref_type(name: &str, expected: &ParameterType) -> Option<ParameterType> {
    let ParameterType::Func(params, returns, _) = expected else {
        return None;
    };
    if params.len() != 1 || **returns != ParameterType::Boolean {
        return None;
    }
    crate::codegen::builtins::general::filter_predicate_type_typed(name, &params[0])
}

/// Lower `expression` against the type its position expects.
///
/// Union-variant subsumption is applied HERE, once, for every expression kind:
/// a value whose static type is a variant of the expected union comes back as
/// `UnionWrap { union, member, value }` (spec `architecture ir` § "Implicit
/// union auto-wrap"). Every expected-type position (call argument, list/map
/// literal element, constructor field, `WITH` field, assignment, binding and
/// global initializer, `RETURN`) reaches the wrap through this one function.
/// Wrapping only selected arms left a call result, field read, `collections::get`
/// or `WITH` result bare at an argument or element position, where codegen read
/// the variant record as a `{tag, size, data}` block (bug-990).
pub(super) fn lower_expression_with_expected(
    expression: &HirExpression,
    expected: Option<&ParameterType>,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    let base = lower_expression_unwrapped(expression, expected, locals, context);
    wrap_union_value(base, expression, expected, locals, context)
}

/// The per-kind lowering behind [`lower_expression_with_expected`], which adds
/// the union wrap. Call it only through that function.
fn lower_expression_unwrapped(
    expression: &HirExpression,
    expected: Option<&ParameterType>,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    match expression {
        HirExpression::String(value) => IrValue::Const {
            type_: ParameterType::String,
            value: value.clone(),
        },
        HirExpression::Number(value) => {
            let (canonical, literal_type) = numeric::classify_literal(value);
            // An explicit `f`/`F` *suffix* makes the literal intrinsically
            // Float/Fixed and wins over the expected type (plan-28-B §4.3). An
            // *unsuffixed* literal — including a `.`/exponent Float-shaped one — is
            // untyped and still coerces to a `Fixed`/`Byte` slot, so the expected
            // type wins there (the pre-existing rule). In plan-28-A no suffix or
            // exponent is lexed yet, so this is byte-identical to the previous
            // expected-first behavior.
            let is_suffixed = value.ends_with('f')
                || value.ends_with('F')
                || value.ends_with('m')
                || value.ends_with('M');
            // bug-820: a `Float`-suffixed literal stays a Float, but a Fixed slot
            // accepts a Float literal (`mfb spec architecture type-inference`,
            // "Literal Coercion"), so it is converted here, as `toFixed(Float)`
            // would convert it. Left a Float const, its f64 bits reached the
            // Q32.32 slot unconverted. A Money slot has no such coercion: the
            // Float const stays, and the verifier rejects it.
            if literal_type == numeric::LiteralType::Float
                && is_suffixed
                && expected == Some(&ParameterType::Fixed)
            {
                return IrValue::Const {
                    type_: ParameterType::Fixed,
                    value: numeric::fixed_constant_from_float_literal(&canonical),
                };
            }
            let type_ = if is_suffixed {
                match literal_type {
                    numeric::LiteralType::Fixed => ParameterType::Fixed,
                    numeric::LiteralType::Money => ParameterType::Money,
                    _ => ParameterType::Float,
                }
            } else if expected == Some(&ParameterType::Fixed) {
                ParameterType::Fixed
            } else if expected == Some(&ParameterType::Byte) {
                ParameterType::Byte
            } else if expected == Some(&ParameterType::Money) {
                // An unsuffixed decimal literal coerces to a Money slot
                // (`LET a AS Money = 1.25`), mirroring the Fixed/Byte paths
                // (plan-29-A §4.4).
                ParameterType::Money
            } else {
                match literal_type {
                    numeric::LiteralType::Float => ParameterType::Float,
                    numeric::LiteralType::Fixed => ParameterType::Fixed,
                    numeric::LiteralType::Money => ParameterType::Money,
                    numeric::LiteralType::Integer => ParameterType::Integer,
                }
            };
            IrValue::Const {
                type_,
                value: canonical,
            }
        }
        HirExpression::Scalar(code_point) => IrValue::Const {
            type_: ParameterType::named("Scalar"),
            value: code_point.to_string(),
        },
        HirExpression::Boolean(value) => IrValue::Const {
            type_: ParameterType::Boolean,
            value: value.to_string(),
        },
        HirExpression::Identifier(value) if value == "NOTHING" => IrValue::Const {
            type_: ParameterType::Nothing,
            value: "NOTHING".to_string(),
        },
        HirExpression::Identifier(value) => {
            let canonical_value = canonical_import_name(value, context);
            // A record constant (`vector::upFloat3`) inlines a record constructor at
            // every use site, copying by value (plan-06-vector.md §4.19). The migrated
            // `vector` package's registry entry supplies the per-field literals + the
            // record's element types. Handled before the scalar-fold path below, which
            // expects a single literal value.
            if let Some(constructor) = registry_record_constant(&canonical_value) {
                return constructor;
            }
            if builtins::is_package_constant(&canonical_value) {
                let type_ = builtins::package_constant_type(&canonical_value)
                    .unwrap_or(ParameterType::Unknown);
                let value = builtins::package_constant_value(&canonical_value)
                    .expect("recognized package constant has a value")
                    .to_string();
                return IrValue::Const { type_, value };
            }

            if locals.contains_key(value) {
                IrValue::Local(value.clone())
            } else if let Some(type_) = context
                .function_types
                .get(value)
                .or_else(|| context.function_types.get(&canonical_value))
            {
                IrValue::FunctionRef {
                    name: canonical_value,
                    type_: type_.clone(),
                }
            } else if context.binding_types.contains_key(value) {
                IrValue::Global(value.clone())
            } else if context.binding_types.contains_key(&canonical_value) {
                // bug-551: a read of an imported package's exported global. The
                // CANONICAL `package.Name` is what
                // `ir::package::apply_package_identity` rewrites to the merged
                // `<id>.package.Name` definition, so an aliased import must
                // lower to the canonical spelling, not the one written.
                IrValue::Global(canonical_value.clone())
            } else if let Some(type_) = expected.and_then(|expected| {
                // A general built-in predicate in a value position (bug-368).
                // These are lowered inline at a direct call site and so have no
                // entry in `function_types`; the out-of-line body is emitted on
                // demand from the `FunctionRef`s collected here
                // (`builtin_function_refs` -> `lower_builtin_function_wrapper`).
                // Without this arm the reference survived as a `Local` that
                // nothing defines, and surfaced to the user as the internal
                // `NIR local reference '<x>' does not resolve`.
                builtin_predicate_ref_type(value, expected)
            }) {
                IrValue::FunctionRef {
                    name: value.clone(),
                    type_,
                }
            } else {
                IrValue::Local(value.clone())
            }
        }
        HirExpression::Call {
            callee,
            arguments,
            line,
            column,
        } => {
            let canonical_callee = canonical_import_name(callee, context);
            let loc = IrSourceLoc {
                line: *line as u32,
                column: *column as u32,
            };
            // `error(code, message)` is a language built-in that produces a
            // read-only `Error` record stamped with the source location of this
            // call expression. Lower it to ordinary record constructors so the
            // rest of the pipeline treats `Error`/`ErrorLoc` as plain records.
            if canonical_callee == "error"
                && !context.function_params.contains_key(callee)
                && !context.function_params.contains_key(&canonical_callee)
            {
                let mut lowered = arguments
                    .iter()
                    .map(|argument| lower_expression(call_arg_value(argument), locals, context));
                // Typecheck guarantees error(code, message) has both args;
                // total lowering (plan-20-D) substitutes Unknown-typed const
                // placeholders when they are absent rather than panicking.
                let placeholder = || IrValue::Const {
                    type_: ParameterType::Unknown,
                    value: String::new(),
                };
                let code = lowered.next().unwrap_or_else(placeholder);
                let message = lowered.next().unwrap_or_else(placeholder);
                return build_error_value(code, message, &context.current_file, loc);
            }
            let normalized_builtin =
                normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
            // A bare general built-in predicate as the callback of a native
            // higher-order member (bug-368). Only this exact shape diverts: the
            // callback's parameter type is the list's element type, which is not
            // written at the call site, so the `FunctionRef` has to be built
            // here where both are in hand.
            //
            // Everything else — a lambda, a named FUNC, an already-typed
            // function value — MUST fall through to the general path below,
            // which supplies the expected type and sets `nonescaping_callback`.
            // Diverting those too silently dropped `forEach`'s licence for a
            // lambda to slot-reference a `MUT` capture.
            let builtin_predicate_arg =
                (crate::codegen::registry::callback_member(&canonical_callee)
                    && normalized_builtin.len() == 2)
                    .then(|| match normalized_builtin[1] {
                        HirExpression::Identifier(predicate) => {
                            expression_type(normalized_builtin[0], locals, context)
                                .and_then(|collection_type| {
                                    filter_predicate_arg_type(predicate, &collection_type)
                                })
                                .map(|predicate_type| IrValue::FunctionRef {
                                    name: predicate.clone(),
                                    type_: predicate_type,
                                })
                        }
                        _ => None,
                    })
                    .flatten();
            let args = if let Some(predicate_ref) = builtin_predicate_arg {
                vec![
                    lower_expression(normalized_builtin[0], locals, context),
                    predicate_ref,
                ]
            } else if context.function_params.contains_key(callee)
                || context.function_params.contains_key(&canonical_callee)
            {
                lower_local_call_arguments(callee, arguments, locals, context)
            } else {
                let mut args = normalized_builtin
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        let expected = call_argument_expected_type(
                            callee, index, arguments, expected, locals, context,
                        );
                        // License a `MUT` slot-reference capture for a lambda in a
                        // non-escaping callback position (e.g. `forEach`'s action).
                        // The lambda lowering consumes it; reset afterward so a
                        // non-lambda argument never carries it.
                        context.nonescaping_callback =
                            builtins::is_nonescaping_callback_arg(&canonical_callee, index);
                        let value = lower_expression_with_expected(
                            argument,
                            expected.as_ref(),
                            locals,
                            context,
                        );
                        context.nonescaping_callback = false;
                        value
                    })
                    .collect::<Vec<_>>();
                // plan-136-A: a LINK call that omits trailing defaulted arguments
                // passes their defaults. Before, the native thunk read an argument
                // that was never passed (`absval()` → an overflow error).
                let link_params = context.link_params;
                if let Some(params) = link_params
                    .get(callee)
                    .or_else(|| link_params.get(&canonical_callee))
                {
                    for param in params.iter().skip(args.len()) {
                        let Some(value) = lower_call_default(param, context) else {
                            break;
                        };
                        args.push(value);
                    }
                }
                args
            };
            // Pad optional trailing arguments (`tls.connect` defaults)
            // with constants so the fixed-ABI runtime helper always receives
            // every parameter (plan-03-net.md §4).
            // Pad optional trailing arguments so the fixed-ABI runtime helper
            // always receives every parameter (plan-72-BB: through the builtins
            // aggregate). A `List OF ...` default (crypto's AEAD `aad`) lowers to an
            // empty list literal and a `Map OF ...` default (http's `headers`) to an
            // empty map literal, not a scalar const; every other default is a const.
            let mut args = args;
            // plan-89-C: a Tier-A `strings::` query whose text argument is an
            // `AttributedString` reads its visible text. Wrap the leading argument
            // in `toString(a)` here — before the native vs source-companion-rewrite
            // split — so both lowerings receive a `String` and the result equals
            // `strings::q(toString(a))`. (Tier-B transforms are plan-89-D and return
            // `AttributedString` instead.)
            //
            // bug-534: `regex::`'s query members join the same tier on the same
            // terms — the query runs on the visible text and returns exactly what
            // the `String` overload returns. `regex` has no Tier-B, because a
            // `regex::replace` that preserved attributes would have to remap spans
            // across a pattern rewrite; it stays a type error instead.
            if (crate::codegen::builtins::strings::is_tier_a_query(&canonical_callee)
                || crate::codegen::builtins::regex::is_tier_a_query(&canonical_callee))
                && !args.is_empty()
                && normalized_builtin
                    .first()
                    .and_then(|arg| expression_type(arg, locals, context))
                    == Some(attributed_string_type())
            {
                let inner = args[0].clone();
                args[0] = IrValue::Call {
                    target: "toString".to_string(),
                    args: vec![inner],
                    type_: ParameterType::String,
                    loc,
                };
            }
            // plan-89-D: the attribute-preserving `padLeft`/`padRight` source-companion
            // bodies take a required `padChar`; fill the default single space for the
            // 2-arg form of an `AttributedString` call. The native `String` forms
            // default `padChar` in codegen, so this only affects the astrings-routed
            // calls and leaves the `String` IR unchanged.
            if matches!(
                canonical_callee.as_str(),
                "strings.padLeft" | "strings.padRight"
            ) && args.len() == 2
                && normalized_builtin
                    .first()
                    .and_then(|arg| expression_type(arg, locals, context))
                    == Some(attributed_string_type())
            {
                args.push(IrValue::Const {
                    type_: ParameterType::String,
                    value: " ".to_string(),
                });
            }
            // The `Fill` trailing params of a migrated (clean-room registry) call:
            // those of the overload every provided argument's type resolves to.
            // `args` may already carry a default an arm above appended (the
            // astrings `padChar`); it is a provided argument here, of a type this
            // selection need not know.
            let mut provided_types: Vec<ParameterType> = normalized_builtin
                .iter()
                .map(|arg| expression_type(arg, locals, context).unwrap_or(ParameterType::Unknown))
                .collect();
            provided_types.resize(args.len(), ParameterType::Unknown);
            let padding = crate::codegen::registry::default_argument_padding(
                &canonical_callee,
                &provided_types,
                context.type_index,
            );
            for (type_, value) in &padding {
                // plan-106-A: the `Fill` type arrives typed, so the empty-collection
                // vs scalar choice is a variant match, not a prefix test.
                match type_ {
                    ParameterType::ListOf(_) => args.push(IrValue::ListLiteral {
                        type_: type_.clone(),
                        values: Vec::new(),
                    }),
                    ParameterType::MapOf(_, _) => args.push(IrValue::MapLiteral {
                        type_: type_.clone(),
                        entries: Vec::new(),
                    }),
                    _ => args.push(IrValue::Const {
                        type_: type_.clone(),
                        value: (*value).to_string(),
                    }),
                }
            }
            // Dequalify migrated `collections::`/`strings::` native members back
            // to their bare lowering names (plan-01-functions.md §5): the native
            // code generator stays keyed on `get`/`transform`/`find`/... .
            // Migrated `collections::`/`strings::` members keep their qualified,
            // dot-containing target all the way to codegen (plan-01-functions.md
            // §5). The native code generator dispatches on the qualified name, so
            // the freed bare names (`get`, `transform`, ...) can be redefined by
            // user code without colliding with the native lowering.
            // `implementation_name` returns the `__pkg_name` form; the injected
            // package's function is lexed in internal mode, so its actual name
            // carries the internal sigil. Internalize the dispatch target to match.
            // `datetime::` is arity-aware: the overloaded constructors and
            // `parse` select a distinct internal name by argument count (§5.1.1).
            // Its OS-seam intrinsics return `None`, staying `datetime.*` runtime
            // helper calls.
            // A general built-in call (`toString(x)`, `len(x)`, …) over a built-in
            // package value type routes to that package's internal override helper
            // (plan-01-overload.md §B.2 / Phase 6), e.g. `toString(net::Url)` ->
            // `#net_urlToString`. User overrides need no routing here — the
            // monomorphizer already rewrote them to a concrete symbol (Phase 5).
            // plan-140-C: the §18.3 gap-fill rule applies here too — the
            // built-in stays authoritative for a type it accepts, so an enum
            // argument (which `toString` accepts) never routes to a package
            // helper that merely shares its bare type name (`color`'s `Color`).
            let package_override =
                if crate::codegen::builtins::general::is_overridable(&canonical_callee) {
                    arguments
                        .first()
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context))
                        .filter(|type_| {
                            builtins::resolve_call_return_type_with_kinds(
                                &canonical_callee,
                                std::slice::from_ref(type_),
                                false,
                                context.type_index,
                            )
                            .is_none()
                        })
                        .and_then(|type_| {
                            builtins::general_override_target(&canonical_callee, &type_)
                        })
                        .map(crate::internal_name::internalize)
                } else {
                    None
                };
            let resolved_target = package_override
                .or_else(|| {
                    // `tls::close` spans two record shapes; a `tls::Listener`
                    // operand routes to the listener-shaped internal close
                    // helper while `tls::close` stays the single user-facing
                    // name (plan-06-tls-server.md §4.1/§6.4). The target is a
                    // runtime helper, not a source companion, so it is not
                    // internalized.
                    if canonical_callee != "tls.close" {
                        return None;
                    }
                    arguments
                        .first()
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context))
                        // bug-459: the PACKAGE-QUALIFIED identity, not the bare
                        // name. A built-in resource has been qualified end to end
                        // since bug-441, so a bare `Listener` matched nothing and
                        // every explicit `tls::close(listener)` silently ran the
                        // SOCKET body -- `nw_connection_cancel` on an `nw_listener`,
                        // a SIGSEGV on macOS. Bare matching is also wrong in the
                        // other direction now: `tcp` declares a `Listener` too.
                        .filter(|type_| {
                            type_.name() == crate::codegen::builtins::tls::TLS_LISTENER_TYPE_ID
                        })
                        .map(|_| crate::codegen::builtins::tls::CLOSE_LISTENER.to_string())
                })
                .or_else(|| {
                    // `audio::` rewrites the overloads whose *body* differs while no user
                    // error is reachable onto their own internal runtime-helper name: the
                    // named-device opens, the timed `read`/`poll`, and the per-direction
                    // `close` (plan-33-A §5). Done at IR level (the `tls.closeListener`
                    // idiom) so the NIR carries the exact runtime-call name and the spec
                    // catalog / required-helper emission / import planning stay
                    // byte-identical. The target is a runtime helper, not a source
                    // companion, so it is not internalized.
                    if crate::codegen::registry::registry().owning_package(&canonical_callee)
                        != Some("audio")
                    {
                        return None;
                    }
                    // These per-package selectors match on type NAMES (exact
                    // record-type dispatch tables in codegen), so the argument
                    // types render at that seam.
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context)
                                .map(|type_| type_.name().into_owned())
                                .unwrap_or_default()
                        })
                        .collect();
                    crate::codegen::builtins::audio::runtime_overload_name(
                        &canonical_callee,
                        &arg_types,
                    )
                    .map(str::to_string)
                })
                .or_else(|| {
                    // `vector::` selects its type-specific internal FUNC from the call's
                    // argument record types (`vector.length(Float3)` ->
                    // `#vector_length_float3`). Each `vector.<op>` overload carries a
                    // `Body::Rewrite("__vector_<op>_<type>")` on the registry, but the
                    // generic coarse-nominal matcher cannot distinguish the nine record
                    // types, so `vector` keeps an EXACT selector over that overload data.
                    if crate::codegen::registry::registry().owning_package(&canonical_callee)
                        != Some("vector")
                    {
                        return None;
                    }
                    // These per-package selectors match on type NAMES (exact
                    // record-type dispatch tables in codegen), so the argument
                    // types render at that seam.
                    let arg_types: Vec<String> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context)
                                .map(|type_| type_.name().into_owned())
                                .unwrap_or_default()
                        })
                        .collect();
                    crate::codegen::builtins::vector::rewrite_target(&canonical_callee, &arg_types)
                        .map(crate::internal_name::internalize)
                })
                .or_else(|| {
                    // `term::drawText(row, column, AttributedString)` routes to the
                    // `__term_drawTextAttr` source-companion body, which applies the
                    // per-scalar bold/underline attributes over the native `String`
                    // drawText. A `String` third argument stays the native
                    // `term.drawText` runtime helper. The companion body is a source
                    // rewrite, so its target is internalized.
                    if canonical_callee != crate::codegen::builtins::term::DRAW_TEXT {
                        return None;
                    }
                    let text_arg_type = arguments
                        .get(2)
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context));
                    if text_arg_type != Some(attributed_string_type()) {
                        return None;
                    }
                    Some(crate::internal_name::internalize("__term_drawTextAttr"))
                })
                .or_else(|| {
                    // `term::measureText(AttributedString)` measures the visible
                    // text through the gated bridge body. A String stays on the
                    // native `term.measureText` ABI helper.
                    if canonical_callee != crate::codegen::builtins::term::MEASURE_TEXT {
                        return None;
                    }
                    let value_type = arguments
                        .first()
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context));
                    if value_type != Some(attributed_string_type()) {
                        return None;
                    }
                    Some(crate::internal_name::internalize("__term_measureTextAttr"))
                })
                .or_else(|| {
                    // plan-89-D: a Tier-B `strings::` transform whose text argument
                    // is an `AttributedString` routes to its `__astrings_*`
                    // attribute-preserving source-companion body (the native String
                    // transform stays for a String argument).
                    if !crate::codegen::builtins::strings::is_tier_b_transform(&canonical_callee) {
                        return None;
                    }
                    let first_arg_type = arguments
                        .first()
                        .map(call_arg_value)
                        .and_then(|argument| expression_type(argument, locals, context));
                    if first_arg_type != Some(attributed_string_type()) {
                        return None;
                    }
                    crate::codegen::builtins::strings::tier_b_transform_impl(&canonical_callee)
                        .map(crate::internal_name::internalize)
                })
                .or_else(|| {
                    // The migrated (clean-room registry) packages rewrite through the
                    // generic, overload-aware `registry::rewrite_target`: an arity-routed
                    // member (datetime's `instant`/`parse`) selects the overload matching
                    // the call's argument types and hands back that overload's target.
                    // `encoding`'s two return-type-overloaded names (`utf8Encode`/
                    // `utf8Decode`) are `Body::Intrinsic` and intentionally yield no
                    // target, so the canonical name reaches the monomorphizer.
                    // These per-package selectors match on type NAMES (exact
                    // record-type dispatch tables in codegen), so the argument
                    // types render at that seam.
                    let arg_types: Vec<crate::types::ParameterType> = arguments
                        .iter()
                        .map(call_arg_value)
                        .map(|argument| {
                            expression_type(argument, locals, context)
                                .unwrap_or_else(|| crate::types::ParameterType::named(""))
                        })
                        .collect();
                    // `strings`' seven scalar-seam members (`toScalars`/`isLetter`/…)
                    // migrated to `Body::Rewrite("__strings_*")`, so the generic
                    // `registry::rewrite_target` now hands back their internal target
                    // (plan-99 PART B), replacing the old
                    // `builtins::strings::implementation_name` fallback.
                    crate::codegen::registry::rewrite_target(&canonical_callee, &arg_types)
                        .map(crate::internal_name::internalize)
                })
                .unwrap_or_else(|| canonical_callee.clone());
            let result_type =
                expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown);
            IrValue::Call {
                // The resource plane reuses the proven data-channel runtime:
                // `thread::transfer`/`accept` lower exactly like `send`/`receive`
                // (the former source checker already enforced their resource semantics).
                target: thread_resource_plane_target(&resolved_target).to_string(),
                args,
                type_: result_type.clone(),
                loc,
            }
        }
        HirExpression::Lambda {
            params,
            body,
            assign_target,
        } => {
            // Consume the non-escaping callback licence so it applies only to this
            // lambda, not to lambdas nested inside its body.
            let nonescaping = context.nonescaping_callback;
            context.nonescaping_callback = false;
            let name = format!("$lambda{}", context.next_lambda_id);
            context.next_lambda_id += 1;
            let param_names = params
                .iter()
                .map(|param| param.name.clone())
                .collect::<HashSet<_>>();
            let mut captures = captured_locals(body, locals, &param_names);
            // The assignment target is a capture too even if it never appears on
            // the right-hand side (mirrors the type checker).
            if let Some(target) = assign_target {
                if !param_names.contains(target)
                    && !captures.iter().any(|capture| &capture.name == target)
                {
                    if let Some(type_) = locals.get(target) {
                        captures.push(CapturedLocal {
                            name: target.clone(),
                            type_: type_.clone(),
                        });
                    }
                }
            }
            // A `MUT` capture in a proven non-escaping position is a reference to the
            // parent's slot, not a by-value copy. Everything else
            // is an ordinary copy capture.
            let by_ref = captures
                .iter()
                .map(|capture| nonescaping && context.mutable_locals.contains(&capture.name))
                .collect::<Vec<_>>();
            // Lambdas carry the enclosing statement's span (the former source checker reports
            // lambda rules at the threaded statement line).
            let loc = context.current_loc;
            let mut lambda_locals = HashMap::new();
            let ir_params = params
                .iter()
                .map(|param| {
                    let type_ = lambda_param_type(param);
                    lambda_locals.insert(param.name.clone(), type_.clone());
                    IrParam {
                        name: param.name.clone(),
                        type_,
                        default: None,
                        loc,
                    }
                })
                .collect::<Vec<_>>();
            // A lambda body is its own function: an enclosing loop's variable is not
            // visible to an EXIT/CONTINUE inside it (restored below, before the push).
            let outer_loop_targets = std::mem::take(&mut context.loop_targets);
            let mut body_ops = captures
                .iter()
                .zip(by_ref.iter())
                .enumerate()
                .map(|(index, (capture, &by_ref))| IrOp::Bind {
                    mutable: by_ref,
                    name: capture.name.clone(),
                    type_: capture.type_.clone(),
                    value: Some(IrValue::Capture {
                        // A closure's environment is far smaller than `u32::MAX`
                        // slots; the cast cannot lose an index a program produces.
                        index: index as u32,
                        type_: capture.type_.clone(),
                        by_ref,
                    }),
                    loc,
                    explicit_type: false,
                })
                .collect::<Vec<_>>();
            for capture in &captures {
                lambda_locals.insert(capture.name.clone(), capture.type_.clone());
            }
            // An assignment-bodied lambda lowers to `target = <body>` followed by a
            // value-less return (it yields `Nothing`); a plain lambda returns its
            // body value.
            let returns = match assign_target {
                Some(target) => {
                    let value = lower_expression(body, &lambda_locals, context);
                    body_ops.push(IrOp::Assign {
                        name: target.clone(),
                        value,
                        loc,
                    });
                    body_ops.push(IrOp::Return { value: None, loc });
                    ParameterType::Nothing
                }
                None => {
                    let returns = expression_type(body, &lambda_locals, context)
                        .unwrap_or(ParameterType::Unknown);
                    let value = lower_expression(body, &lambda_locals, context);
                    body_ops.push(IrOp::Return {
                        value: Some(value),
                        loc,
                    });
                    returns
                }
            };
            context.loop_targets = outer_loop_targets;
            context.lambdas.push(IrFunction {
                name: name.clone(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: ir_params,
                returns: returns.clone(),
                body: body_ops,
                file: context.current_file.clone(),
                loc,
                resource_owners: HashMap::new(),
            });
            let params = params.iter().map(lambda_param_type).collect::<Vec<_>>();
            // A lambda is never `ISOLATED`, matching the un-prefixed
            // `FUNC(…) AS …` spelling this used to `format!`.
            let type_ = ParameterType::Func(params, Box::new(returns), false);
            if captures.is_empty() {
                IrValue::FunctionRef { name, type_ }
            } else {
                IrValue::Closure {
                    name,
                    type_: type_.clone(),
                    captures: captures
                        .iter()
                        .zip(by_ref.iter())
                        .map(|(capture, &by_ref)| {
                            if by_ref {
                                // Capture the parent slot's address (by-ref), so
                                // the callback observes and updates the live binding.
                                IrValue::LocalRef {
                                    name: capture.name.clone(),
                                    type_: capture.type_.clone(),
                                }
                            } else {
                                lower_expression(
                                    &HirExpression::Identifier(capture.name.clone()),
                                    locals,
                                    context,
                                )
                            }
                        })
                        .collect(),
                }
            }
        }
        HirExpression::Constructor {
            type_: constructor_type,
            arguments,
        } => {
            // plan-111-B: `canonical_import_type` rewrites the import ALIAS on
            // the head nominal STRUCTURALLY, so the result needs no
            // re-classification — the name-domain form had to render, edit the
            // spelling and `parse` it back so a user generic came out a `UserOf`
            // rather than an opaque nominal.
            let canonical_type = canonical_import_type(constructor_type, context);
            let fields = context
                .type_index
                .records
                .get(&canonical_type)
                .or_else(|| context.type_index.records.get(constructor_type))
                .or_else(|| context.type_index.variant_fields.get(&canonical_type))
                .or_else(|| context.type_index.variant_fields.get(constructor_type));
            IrValue::Constructor {
                type_: canonical_type.clone(),
                args: lower_constructor_args(&canonical_type, arguments, fields, locals, context),
            }
        }
        HirExpression::WithUpdate { target, updates } => {
            let type_ = expression_type(target, locals, context).unwrap_or(ParameterType::Unknown);
            let lowered_target = Box::new(lower_expression(target, locals, context));
            let lowered_updates = updates
                .iter()
                .map(|update| {
                    // Coerce a bare numeric literal to the record field's
                    // declared type, mirroring `lower_constructor_args` — else an
                    // unsuffixed literal updating a `Fixed`/`Money` field is typed
                    // `Integer` and reinterpreted as raw bits (bug-156).
                    let field_type = context.type_index.record_field_type(&type_, &update.field);
                    IrRecordUpdate {
                        field: update.field.clone(),
                        value: lower_expression_with_expected(
                            &update.value,
                            field_type.as_ref(),
                            locals,
                            context,
                        ),
                    }
                })
                .collect();
            IrValue::WithUpdate {
                type_,
                target: lowered_target,
                updates: lowered_updates,
            }
        }
        HirExpression::ListLiteral(values) => {
            let expected_element = match expected {
                Some(ParameterType::ListOf(element)) => Some((**element).clone()),
                _ => None,
            };
            let lowered = values
                .iter()
                .map(|value| {
                    lower_expression_with_expected(
                        value,
                        expected_element.as_ref(),
                        locals,
                        context,
                    )
                })
                .collect::<Vec<_>>();
            let element_type = expected_element.unwrap_or_else(|| {
                values
                    .first()
                    .and_then(literal_expression_type)
                    // bug-556: a list literal written INLINE at a call argument
                    // has no `expected` to inherit from, and its first element is
                    // usually not a *literal* — so this fell straight to
                    // `Unknown`, and every rule that reads the argument's element
                    // type was silently skipped. `check_builtin_comparability`
                    // deliberately lets an `Unknown` element pass (it must never
                    // reject on an unknown), so `collections::find([bag], bag)`
                    // was ACCEPTED while the identical `LET xs = [bag]` then
                    // `find(xs, bag)` was refused — the verdict depended on
                    // whether the author had named the list.
                    //
                    // `expression_type` is the same typer the binding path
                    // already uses and is in scope here; falling back to it makes
                    // the two spellings agree. It is a fallback, not a
                    // replacement: `expected_element` still wins, so an annotated
                    // or parameter-driven element type is unaffected.
                    .or_else(|| {
                        values
                            .first()
                            .and_then(|first| expression_type(first, locals, context))
                    })
                    .unwrap_or(ParameterType::Unknown)
            });
            IrValue::ListLiteral {
                type_: ParameterType::list_of(element_type),
                values: lowered,
            }
        }
        HirExpression::SetLiteral {
            element_type,
            elements,
        } => {
            let expected_element = match expected {
                Some(ParameterType::SetOf(element)) => (**element).clone(),
                _ => element_type.clone(),
            };
            let lowered = elements
                .iter()
                .map(|value| {
                    lower_expression_with_expected(value, Some(&expected_element), locals, context)
                })
                .collect::<Vec<_>>();
            IrValue::SetLiteral {
                type_: ParameterType::set_of(element_type.clone()),
                values: lowered,
            }
        }
        HirExpression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => {
            // bug-989: a map literal's type is its own annotation. When the
            // position expects that map's UNION form (`Map OF String TO Square
            // { … }` where a `Map OF String TO Shape` is expected) the entries
            // lower against the annotation, so they match the type the literal
            // carries, and the whole literal is then converted like any other
            // map value. Lowered against the expected type instead, the entries
            // were wrapped inside a literal still typed as the variant map.
            let annotation = ParameterType::map_of(key_type.clone(), value_type.clone());
            let expected = match expected {
                Some(expected)
                    if context
                        .type_index
                        .union_collection_converts(&annotation, expected) =>
                {
                    Some(&annotation)
                }
                other => other,
            };
            let (expected_key, expected_value) = match expected {
                Some(ParameterType::MapOf(key, value)) => {
                    (Some((**key).clone()), Some((**value).clone()))
                }
                _ => (None, None),
            };
            let expected_key = expected_key.as_ref();
            let expected_value = expected_value.as_ref();
            IrValue::MapLiteral {
                type_: ParameterType::map_of(key_type.clone(), value_type.clone()),
                entries: entries
                    .iter()
                    .map(|(key, value)| {
                        (
                            lower_expression_with_expected(key, expected_key, locals, context),
                            lower_expression_with_expected(value, expected_value, locals, context),
                        )
                    })
                    .collect(),
            }
        }
        HirExpression::MemberAccess { target, member } => {
            let member_type =
                expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown);
            // bug-554: a package-qualified enum-member read lowers to a member
            // access on the enum's TYPE name. bug-632: that name is the enum's
            // package-qualified identity (`recpkg.Colour`, binding resolved to
            // its package), which `ir::verify`'s `enums` table and the merged
            // package IR (`ir::package::qualify_package_types`) both key.
            if let Some(enum_type) = qualified_imported_enum(target, member, context) {
                return IrValue::MemberAccess {
                    target: Box::new(IrValue::Local(enum_type)),
                    member: member.clone(),
                    type_: member_type,
                };
            }
            IrValue::MemberAccess {
                target: Box::new(lower_expression(target, locals, context)),
                member: member.clone(),
                type_: member_type,
            }
        }
        HirExpression::Trapped { .. } => {
            // Inline traps are only constructed as the value of a binding,
            // assignment, or bare-expression statement, where `lower_statement`
            // desugars them directly; they never reach value lowering.
            unreachable!("inline TRAP must be lowered as a statement value")
        }
        HirExpression::Binary {
            left,
            operator,
            right,
            line,
            column,
        } => {
            let result_type =
                expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown);
            let loc = IrSourceLoc {
                line: *line as u32,
                column: *column as u32,
            };
            // plan-89-D: `AttributedString & AttributedString` concatenation routes
            // to the `__astrings_concat` source-companion body (text concatenated,
            // right operand's spans shifted by the left's scalar length).
            if *operator == BinaryOp::Concat && result_type == attributed_string_type() {
                return IrValue::Call {
                    target: crate::internal_name::internalize("__astrings_concat"),
                    args: vec![
                        lower_expression(left, locals, context),
                        lower_expression(right, locals, context),
                    ],
                    type_: attributed_string_type(),
                    loc,
                };
            }
            IrValue::Binary {
                op: operator.clone(),
                left: Box::new(lower_expression(left, locals, context)),
                right: Box::new(lower_expression(right, locals, context)),
                type_: result_type.clone(),
                loc,
            }
        }
        HirExpression::Unary {
            operator,
            operand,
            line,
            column,
        } => {
            // A negated decimal literal in an exact-numeric slot (`LET a AS Money =
            // -1.25`, `LET a AS Fixed = -1.25`) must lower its operand as a const of
            // that type, so the raw negate operates on the scaled i64 rather than an
            // f64 bit pattern, and the node is annotated to match the binding
            // (plan-29-A §4.4).
            //
            // `Fixed` was originally excluded here "so their goldens are unchanged".
            // That was silent corruption, not a neutral choice: the operand stayed a
            // *Float* const, so `LET a AS Fixed = -1.25` stored the f64 bit pattern
            // of 1.25, negated, into a Q32.32 slot and read back as
            // -1074528256.0 (bug-367). Every negative `Fixed` literal was affected;
            // the positive form was always correct, which is why it went unnoticed.
            let exact_literal_negation = *operator == UnaryOp::Negate
                && matches!(
                    expected,
                    Some(ParameterType::Money) | Some(ParameterType::Fixed)
                )
                && matches!(operand.as_ref(), HirExpression::Number(_));
            let lowered_operand = if exact_literal_negation {
                lower_expression_with_expected(operand, expected, locals, context)
            } else {
                lower_expression(operand, locals, context)
            };
            // The negation has its lowered literal's type. That is the slot's
            // type for an unsuffixed literal, and for `-1.5f` in a Fixed slot
            // (converted), but a suffix that keeps another type (`-1.5f` or
            // `-1.5F` in a Money slot) keeps it here too, so the verifier sees a
            // mismatch rather than a Money-annotated negation of a Float (bug-820).
            let result_type = match (&lowered_operand, exact_literal_negation) {
                (IrValue::Const { type_, .. }, true) => type_.clone(),
                _ => expression_type(expression, locals, context).unwrap_or(ParameterType::Unknown),
            };
            // bug-07: the minimum `Fixed` (`-2147483648.0`) parses as
            // `-(2147483648.0F)`, but the positive magnitude overflows the i64
            // raw (2^63), so the constant can never materialize on its own. Fold
            // the negation into the literal here — `fixed_raw_from_decimal`
            // handles the signed string correctly. The guard is exact: it fires
            // only when the positive magnitude overflows *and* the negated value
            // fits, which is true solely at the min boundary (raw == 2^63), so
            // every in-range negated literal keeps its `Unary` shape and no
            // existing codegen/golden shifts.
            if *operator == UnaryOp::Negate {
                if let IrValue::Const { type_, value } = &lowered_operand {
                    if matches!(type_, crate::types::ParameterType::Fixed)
                        && numeric::fixed_raw_from_decimal(value).is_err()
                        && numeric::fixed_raw_from_decimal(&format!("-{value}")).is_ok()
                    {
                        return IrValue::Const {
                            type_: ParameterType::Fixed,
                            value: format!("-{value}"),
                        };
                    }
                    // bug-286: the same fold for the most-negative `Integer`
                    // (`-9223372036854775808`). Syntaxcheck and `ir::verify`
                    // deliberately accept `-N` where `N == i64::MAX + 1`
                    // (spec §4.12), but without this arm the `Unary` shape
                    // survives to codegen, which materializes the u64 bit
                    // pattern and then negates it at runtime — an overflow that
                    // traps on every run. The guard is exact for the same
                    // reason the `Fixed`/`Money` guards are: it fires only when
                    // the positive magnitude does not fit an i64 *and* the
                    // negated form does, which is true solely at `i64::MIN`, so
                    // every in-range negated literal keeps its `Unary` shape.
                    if matches!(type_, crate::types::ParameterType::Integer)
                        && value.parse::<i64>().is_err()
                        && format!("-{value}").parse::<i64>().is_ok()
                    {
                        return IrValue::Const {
                            type_: ParameterType::Integer,
                            value: format!("-{value}"),
                        };
                    }
                    // The same fold for the most-negative Money
                    // (`-92233720368547.75808`), whose positive magnitude
                    // overflows the i64 raw (plan-29-B §4.2).
                    if matches!(type_, crate::types::ParameterType::Money)
                        && numeric::money_raw_from_decimal(value).is_err()
                        && numeric::money_raw_from_decimal(&format!("-{value}")).is_ok()
                    {
                        return IrValue::Const {
                            type_: ParameterType::Money,
                            value: format!("-{value}"),
                        };
                    }
                }
            }
            IrValue::Unary {
                op: operator.clone(),
                operand: Box::new(lowered_operand),
                type_: result_type.clone(),
                loc: IrSourceLoc {
                    line: *line as u32,
                    column: *column as u32,
                },
            }
        }
    }
}
