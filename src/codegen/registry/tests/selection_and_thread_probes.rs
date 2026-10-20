//! plan-176-H: `registry::tests`' selection, constant/override boundary and thread-probe
//! tests. The shared builders live in `registry/tests/mod.rs`.

use super::*;

#[test]
fn constant_and_override_boundary_fns_fall_through_for_unowned_names() {
    // A throwaway `demo` package built in the test above is not wired into the
    // frozen `registry()`, so its constant/override names stay absent and the
    // dual-path in `builtins`/`ir::lower` falls through to the hand tables.
    assert!(!is_package_constant("demo.pi"));
    assert_eq!(constant_type_name("demo.pi"), None);
    assert_eq!(constant_value("demo.pi"), None);
    assert_eq!(constant_components("demo.zero3"), None);
    // A general override the frozen registry does NOT own falls through.
    assert_eq!(
        general_override_target("toString", &crate::types::ParameterType::parse("Nope")),
        None
    );
    // The migrated `vector` package DOES own `toString(vector.Float3)` now
    // (add_override) — keyed by that package-qualified identity.
    assert_eq!(
        general_override_target(
            "toString",
            &crate::types::ParameterType::parse("vector.Float3")
        ),
        Some("__vector_toString_float3")
    );
    // bug-668: and NOT by the bare leaf, which only a user `TYPE Float3` can
    // spell. Matching it hijacked the call to `__vector_toString_float3` in a
    // program that never imported `vector`.
    assert_eq!(
        general_override_target("toString", &crate::types::ParameterType::parse("Float3")),
        None
    );

    // A malformed (unqualified) name never panics.
    assert!(!is_package_constant("bare"));
    assert_eq!(constant_components("bare"), None);
}

#[test]
fn frozen_registry_exposes_the_csv_package() {
    let pkg = registry().resolve_package("csv").expect("csv registered");
    assert_eq!(pkg.import_name(), "csv");
    // parse, stringify, readRow (bug-809 removed parseStream).
    assert_eq!(pkg.functions().len(), 3);
    assert!(registry().resolve_package("nope").is_none());
}

#[test]
fn an_overload_is_two_implementations_differing_by_parameter_type() {
    let describe = func(
        "describe",
        vec![
            Implementation {
                params: vec![param("v", ParameterType::Integer)],
                return_type: ParameterType::String,
                errors: vec![],
                body: Body::Rewrite("__describe_int"),
            },
            Implementation {
                params: vec![param("v", ParameterType::String)],
                return_type: ParameterType::String,
                errors: vec![],
                body: Body::Rewrite("__describe_str"),
            },
        ],
    );
    let impls = &describe.implementations;
    assert_eq!(impls.len(), 2);
    assert_eq!(impls[0].params[0].ty.name(), "Integer");
    assert_eq!(impls[1].params[0].ty.name(), "String");
}

#[test]
fn select_binds_type_variables_and_substitutes_the_return() {
    use ParameterType::Integer;
    // get(List OF T, Integer) AS T  |  get(Map OF K TO V, K) AS V
    let get = func(
        "get",
        vec![
            generic_impl(
                vec![list_of(ParameterType::var("T")), Integer],
                ParameterType::var("T"),
            ),
            generic_impl(
                vec![
                    map_of(ParameterType::var("K"), ParameterType::var("V")),
                    ParameterType::var("K"),
                ],
                ParameterType::var("V"),
            ),
        ],
    );

    // List overload: element type of arg 0.
    assert_eq!(
        get.dispatch(&call(&["List OF Integer", "Integer"]))
            .unwrap()
            .return_type
            .name(),
        "Integer"
    );
    // Nested containers substitute structurally.
    assert_eq!(
        get.dispatch(&call(&["List OF List OF String", "Integer"]))
            .unwrap()
            .return_type
            .name(),
        "List OF String"
    );
    // Map overload: the value type; the key overload's `List OF T` param rejects a
    // `Map` arg, so this resolves against the second implementation.
    assert_eq!(
        get.dispatch(&call(&["Map OF String TO Integer", "String"]))
            .unwrap()
            .return_type
            .name(),
        "Integer"
    );
}

#[test]
fn thread_probe_waitfor_receive_send_poll() {
    use ParameterType::{Integer, Nothing, String, Unknown};
    let data_handle = |m, o| th(false, m, Unknown, o);
    // waitFor(Thread OF Msg TO Out) AS Out
    let wait_for = func(
        "waitFor",
        vec![generic_impl(
            vec![data_handle(
                ParameterType::var("Msg"),
                ParameterType::var("Out"),
            )],
            ParameterType::var("Out"),
        )],
    );
    let parent = th(false, Integer, Nothing, String);
    assert_eq!(
        wait_for
            .resolve(&CallShape {
                args: vec![parent.clone()]
            })
            .map(|s| s.return_type.name().into_owned()),
        Some("String".into()),
        "waitFor parent (strict)"
    );
    // worker handle rejected (kind mismatch)
    assert!(
        wait_for
            .resolve(&CallShape {
                args: vec![th(true, Integer, Nothing, String)]
            })
            .is_none(),
        "waitFor rejects worker"
    );

    // receive(Thread OF Msg TO Out) AS Msg — including the Unknown-message case.
    let receive = func(
        "receive",
        vec![generic_impl(
            vec![data_handle(
                ParameterType::var("Msg"),
                ParameterType::var("Out"),
            )],
            ParameterType::var("Msg"),
        )],
    );
    assert_eq!(
        receive
            .dispatch(&CallShape {
                args: vec![parent.clone()]
            })
            .map(|s| s.return_type.name().into_owned()),
        Some("Integer".into()),
        "receive Msg"
    );
    assert_eq!(
        receive
            .dispatch(&CallShape {
                args: vec![th(false, Unknown, Nothing, String)]
            })
            .map(|s| s.return_type.name().into_owned()),
        Some("Unknown".into()),
        "receive Unknown message"
    );

    // send(Thread OF Msg TO Out, Msg) AS Nothing — cross-param constraint.
    let send = func(
        "send",
        vec![generic_impl(
            vec![
                data_handle(ParameterType::var("Msg"), ParameterType::var("Out")),
                ParameterType::var("Msg"),
            ],
            Nothing,
        )],
    );
    assert_eq!(
        send.resolve(&CallShape {
            args: vec![parent.clone(), Integer]
        })
        .map(|s| s.return_type.name().into_owned()),
        Some("Nothing".into()),
        "send matching message"
    );
    assert!(
        send.resolve(&CallShape {
            args: vec![parent.clone(), String]
        })
        .is_none(),
        "send message mismatch rejected"
    );
    // Legacy resolve_send_unknown_message: handle message Unknown accepts any arg.
    assert_eq!(
        send.resolve(&CallShape {
            args: vec![th(false, Unknown, Nothing, String), Integer]
        })
        .map(|s| s.return_type.name().into_owned()),
        Some("Nothing".into()),
        "send Unknown-message accepts any arg (LEGACY)"
    );
}

#[test]
fn thread_probe_transfer_accept_resource_plane() {
    use ParameterType::{Integer, Nothing, String};
    let file = ParameterType::named("fs.File");
    // transfer(Thread OF Msg RES Res TO Out, Res) AS Nothing — resource-ONLY overload.
    let transfer = func(
        "transfer",
        vec![generic_impl(
            vec![
                th(
                    false,
                    ParameterType::var("Msg"),
                    ParameterType::var("Res"),
                    ParameterType::var("Out"),
                ),
                ParameterType::var("Res"),
            ],
            Nothing,
        )],
    );
    assert_eq!(
        transfer
            .resolve(&CallShape {
                args: vec![th(false, Integer, file.clone(), String), file.clone()]
            })
            .map(|s| s.return_type.name().into_owned()),
        Some("Nothing".into()),
        "transfer matching resource"
    );
    // A data-only handle has no resource plane: STRICT validation rejects it (the
    // resource overload's `res` Var can't bind `Nothing`) -> TYPE_CALL_ARGUMENT_MISMATCH.
    assert!(
        transfer
            .resolve(&CallShape {
                args: vec![th(false, Integer, Nothing, String), file.clone()]
            })
            .is_none(),
        "transfer on data-only handle rejected by strict validation (LEGACY)"
    );

    // accept(Thread OF Msg RES Res TO Out) AS Res — resource-ONLY overload.
    let accept = func(
        "accept",
        vec![generic_impl(
            vec![th(
                false,
                ParameterType::var("Msg"),
                ParameterType::var("Res"),
                ParameterType::var("Out"),
            )],
            ParameterType::var("Res"),
        )],
    );
    assert_eq!(
        accept
            .dispatch(&CallShape {
                args: vec![th(false, Integer, file.clone(), String)]
            })
            .map(|s| s.return_type.name().into_owned()),
        Some("fs.File".into()),
        "accept returns resource"
    );
    // A data-only handle is rejected by STRICT validation (the strict-`Nothing`
    // guard), exactly as legacy `resolve_call` returns None on a data-only thread.
    assert!(
        accept
            .resolve(&CallShape {
                args: vec![th(false, Integer, Nothing, String)]
            })
            .is_none(),
        "accept on data-only handle rejected by strict validation (LEGACY)"
    );
}

#[test]
fn thread_probe_start_extraction_and_resource_echo() {
    use ParameterType::{Integer, Nothing, String};
    let file = ParameterType::named("fs.File");
    // start = TWO overloads (resource first, data second), the settled model.
    //   RESOURCE: ISOLATED FUNC(ThreadWorker OF Msg RES Res TO Out, In) AS Out
    //             -> Thread OF Msg RES Res TO Out
    //   DATA:     ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out
    //             -> Thread OF Msg TO Out  (worker/return carry NO res var)
    let opt_int = |name| Parameter {
        name,
        desc: "",
        aliases: &[],
        ty: Integer,
        default: DefaultValue::Optional,
    };
    let overload = |worker_res: ParameterType, ret_res: ParameterType| Implementation {
        params: vec![
            param(
                "f",
                ParameterType::func_isolated(
                    vec![
                        th(
                            true,
                            ParameterType::var("Msg"),
                            worker_res,
                            ParameterType::var("Out"),
                        ),
                        ParameterType::var("In"),
                    ],
                    ParameterType::var("Out"),
                ),
            ),
            param("data", ParameterType::var("In")),
            opt_int("inboundLimit"),
            opt_int("outboundLimit"),
        ],
        return_type: th(
            false,
            ParameterType::var("Msg"),
            ret_res,
            ParameterType::var("Out"),
        ),
        errors: vec![],
        body: Body::Intrinsic,
    };
    let start = func(
        "start",
        vec![
            overload(ParameterType::var("Res"), ParameterType::var("Res")),
            overload(Nothing, Nothing),
        ],
    );

    // Data-only worker: strict validation MUST accept (via the data overload), and
    // dispatch echoes a data-only parent handle (via the resource overload, whose
    // `Res` binds `Nothing` under lenient and elides).
    let data_fn =
        ParameterType::func_isolated(vec![th(true, Integer, Nothing, String), Integer], String);
    assert!(
        start
            .resolve(&CallShape {
                args: vec![data_fn.clone(), Integer]
            })
            .is_some(),
        "start STRICT-validates a data-only worker"
    );
    assert_eq!(
        start
            .dispatch(&CallShape {
                args: vec![data_fn, Integer]
            })
            .map(|s| s.return_type.name().into_owned()),
        Some("Thread OF Integer TO String".into()),
        "start echoes a data-only worker"
    );

    // Resourced worker: start echoes the resource plane onto the parent handle.
    let res_fn =
        ParameterType::func_isolated(vec![th(true, Integer, file, String), Integer], String);
    assert!(
        start
            .resolve(&CallShape {
                args: vec![res_fn.clone(), Integer]
            })
            .is_some(),
        "start STRICT-validates a resourced worker"
    );
    assert_eq!(
        start
            .dispatch(&CallShape {
                args: vec![res_fn, Integer]
            })
            .map(|s| s.return_type.name().into_owned()),
        Some("Thread OF Integer RES fs.File TO String".into()),
        "start echoes a resourced worker's plane"
    );

    // A plain (non-isolated) worker is NOT accepted where start demands ISOLATED.
    let plain_fn = ParameterType::func(vec![th(true, Integer, Nothing, String), Integer], String);
    assert!(
        start
            .resolve(&CallShape {
                args: vec![plain_fn, Integer]
            })
            .is_none(),
        "start rejects a non-isolated worker"
    );
}

#[test]
fn bug_824_plain_callback_param_accepts_isolated_concrete() {
    use ParameterType::{Boolean, Integer, Nothing};
    // Shaped exactly like `collections::forEach(List OF T, FUNC(T) AS Nothing)`:
    // a plain (non-isolated) callback parameter. The owner rule (2026-10-04),
    // "an ISOLATED FUNC is a FUNC. a FUNC is NOT an ISOLATED FUNC," is bug-786's
    // rule for user calls; this is the same rule applied to a built-in's
    // registry dispatch, which `unify_at`'s `Func`/`Func` arm used to refuse by
    // requiring `isolated == concrete_isolated` exactly.
    let for_each = func(
        "forEach",
        vec![Implementation {
            params: vec![
                param("value", ParameterType::list_of(ParameterType::var("T"))),
                param(
                    "action",
                    ParameterType::func(vec![ParameterType::var("T")], Nothing),
                ),
            ],
            return_type: Nothing,
            errors: vec![],
            body: Body::Intrinsic,
        }],
    );
    let isolated_action = ParameterType::func_isolated(vec![Integer], Nothing);
    assert!(
        for_each
            .resolve(&CallShape {
                args: vec![ParameterType::list_of(Integer), isolated_action.clone()]
            })
            .is_some(),
        "a plain FUNC callback parameter accepts an ISOLATED FUNC of the same shape"
    );
    assert!(
        for_each
            .dispatch(&CallShape {
                args: vec![ParameterType::list_of(Integer), isolated_action]
            })
            .is_some(),
        "dispatch also accepts the ISOLATED FUNC concrete"
    );
    // The reverse direction is untouched: a plain FUNC still never satisfies an
    // ISOLATED FUNC parameter (pinned independently by
    // `thread_probe_start_extraction_and_resource_echo`'s "start rejects a
    // non-isolated worker" assertion above).
    let plain_action = ParameterType::func(vec![Boolean], Nothing);
    assert!(
        for_each
            .resolve(&CallShape {
                args: vec![ParameterType::list_of(Boolean), plain_action]
            })
            .is_some(),
        "sanity: a plain FUNC still satisfies a plain FUNC parameter"
    );
}

#[test]
fn thread_probe_parse_isolated_func_arg() {
    // The type checker hands `start`'s callback arg as `ISOLATED FUNC(...) AS ...`;
    // `parse` decomposes it into an isolated `Func` whose first param is a
    // `ThreadHandle`, and `name()` round-trips the `ISOLATED ` marker byte-exactly.
    let spelling = "ISOLATED FUNC(ThreadWorker OF Integer TO String, Integer) AS String";
    let parsed = ParameterType::parse(spelling);
    assert_eq!(parsed.name(), spelling, "ISOLATED FUNC round-trips");
    let ParameterType::Func(params, _, isolated) = &parsed else {
        panic!("parse decomposes ISOLATED FUNC into a Func (got {parsed:?})");
    };
    assert!(isolated, "the Func is marked isolated");
    assert!(
        matches!(&params[0], ParameterType::ThreadHandle { worker: true, .. }),
        "the first param is a ThreadWorker handle (got {:?})",
        params[0]
    );
}

/// bug-550 (`append([], x)` does not build): a GENERIC parameter's expected
/// type, resolved from the call.
///
/// `collections::append`'s parameter 0 is `List OF T`, which has no expected
/// type on its own — so `argument_types_typed`/`agreed_argument_type` both
/// decline and an inline `[]` written there used to lower as `List OF Unknown`.
/// Given the call's actual argument types the position DOES have an answer:
/// unification binds `T := Unknown` from the empty literal and refines it to
/// `Integer` from the item.
#[test]
fn resolved_parameter_type_infers_a_generic_position_from_the_call() {
    let unknown_list = list_of(ParameterType::Unknown);
    assert_eq!(
        resolved_parameter_type(
            "collections.append",
            0,
            &[unknown_list.clone(), ParameterType::Integer],
            None,
            &NoTypeKinds,
        ),
        Some(list_of(ParameterType::Integer)),
        "T refines Unknown -> Integer, so parameter 0 is List OF Integer"
    );

    // Both `append` overloads unify with `([], List OF String)` — the element
    // form binding `T := List OF String`, the concatenating form `T := String` —
    // and they disagree about parameter 0. With nothing to break the tie the
    // answer is NONE, never a guess: the call keeps lowering exactly as before.
    let string_list = list_of(ParameterType::String);
    assert_eq!(
        resolved_parameter_type(
            "collections.append",
            0,
            &[unknown_list.clone(), string_list.clone()],
            None,
            &NoTypeKinds,
        ),
        None,
        "an ambiguous generic position declines"
    );

    // The declared type of what the call feeds breaks that tie. `append`
    // returns `Arg(0)`, so the expected return IS parameter 0's pattern:
    // seeding `T := String` leaves only the concatenating overload.
    assert_eq!(
        resolved_parameter_type(
            "collections.append",
            0,
            &[unknown_list.clone(), string_list.clone()],
            Some(&string_list),
            &NoTypeKinds,
        ),
        Some(string_list.clone()),
        "the expected return type seeds the unification"
    );

    // A MONOMORPHIC position is not this function's business — the two
    // existing providers answer it, and a position they deliberately decline
    // must keep going through the existing selection path.
    assert_eq!(
        resolved_parameter_type(
            "strings.upper",
            0,
            &[ParameterType::String],
            None,
            &NoTypeKinds,
        ),
        None,
        "a non-generic parameter declines"
    );
}

/// bug-988: an oracle in which `U` is a union whose one variant is `V`.
struct UnionUOverV;

impl TypeKinds for UnionUOverV {
    fn is_enum(&self, _t: &ParameterType) -> bool {
        false
    }

    fn union_accepts(&self, union: &ParameterType, actual: &ParameterType) -> bool {
        union.is_named("U") && actual.is_named("V")
    }
}

#[test]
fn a_top_level_variable_bound_to_a_union_accepts_its_variant() {
    let union = ParameterType::named("U");
    let variant = ParameterType::named("V");
    let other = ParameterType::named("W");
    let union_list = list_of(union.clone());
    // (a) `append(List OF U, V)`: `T = U` from the list, the item `V` is a
    // variant, so the item's expected type is the union and lowering wraps it.
    assert_eq!(
        resolved_parameter_type(
            "collections.append",
            1,
            &[union_list.clone(), variant.clone()],
            Some(&union_list),
            &UnionUOverV,
        ),
        Some(union.clone()),
    );
    assert_eq!(
        resolve_call_typed_with_kinds(
            "collections.append",
            &[union_list.clone(), variant.clone()],
            true,
            &UnionUOverV,
        ),
        Some(union_list.clone()),
    );
    // Without the oracle nothing changes: identity rebinding rejects it.
    assert_eq!(
        resolve_call_typed(
            "collections.append",
            &[union_list.clone(), variant.clone()],
            true
        ),
        None,
    );
    // (b) bug-989: a `List OF V` VALUE at `append`'s `List OF T` is accepted
    // too, with the union's list as its expected type, so lowering converts it.
    assert_eq!(
        resolved_parameter_type(
            "collections.append",
            1,
            &[union_list.clone(), list_of(variant.clone())],
            Some(&union_list),
            &UnionUOverV,
        ),
        Some(union_list.clone()),
    );
    assert_eq!(
        resolve_call_typed_with_kinds(
            "collections.append",
            &[union_list.clone(), list_of(variant.clone())],
            true,
            &UnionUOverV,
        ),
        Some(union_list.clone()),
    );
    // (c) A type that is not a variant of `U` stays rejected.
    assert_eq!(
        resolve_call_typed_with_kinds(
            "collections.append",
            &[union_list.clone(), other],
            true,
            &UnionUOverV,
        ),
        None,
    );
    // A built-in package's union needs no declared-type oracle: the default
    // `union_accepts` reads the registry (`canvas::DrawItem`, bug-988's report).
    let draw_items = list_of(ParameterType::named("canvas.DrawItem"));
    assert_eq!(
        resolve_call_typed_with_kinds(
            "collections.append",
            &[draw_items.clone(), ParameterType::named("canvas.Rectangle")],
            true,
            &NoTypeKinds,
        ),
        Some(draw_items),
    );
    // The bucket fixes `T` (owner decision): `T` is `V` from the list, and the
    // union item does not fit it. Never `T = U` with the list converted — in any
    // mode, for the item form and the two-list form.
    for strict in [true, false] {
        for item in [union.clone(), union_list.clone()] {
            assert_eq!(
                resolve_call_typed_with_kinds(
                    "collections.append",
                    &[list_of(variant.clone()), item],
                    strict,
                    &UnionUOverV,
                ),
                None,
            );
        }
    }
    assert_eq!(
        resolved_parameter_type(
            "collections.append",
            0,
            &[list_of(variant.clone()), union.clone()],
            None,
            &UnionUOverV,
        ),
        None,
    );
    // The same for every member whose first argument is the collection.
    let index = ParameterType::Integer;
    let variant_list = list_of(variant.clone());
    let variant_map = map_of(ParameterType::String, variant.clone());
    let calls: Vec<(&str, Vec<ParameterType>)> = vec![
        (
            "collections.prepend",
            vec![variant_list.clone(), union.clone()],
        ),
        (
            "collections.insert",
            vec![variant_list.clone(), index.clone(), union.clone()],
        ),
        (
            "collections.set",
            vec![variant_list.clone(), index.clone(), union.clone()],
        ),
        (
            "collections.set",
            vec![variant_map.clone(), ParameterType::String, union.clone()],
        ),
        (
            "collections.getOr",
            vec![variant_list.clone(), index.clone(), union.clone()],
        ),
        (
            "collections.getOr",
            vec![variant_map, ParameterType::String, union.clone()],
        ),
        (
            "collections.repeat",
            vec![variant_list.clone(), union.clone(), index],
        ),
        (
            "collections.contains",
            vec![variant_list.clone(), union.clone()],
        ),
        ("collections.find", vec![variant_list, union.clone()]),
    ];
    for (member, args) in calls {
        for strict in [true, false] {
            assert_eq!(
                resolve_call_typed_with_kinds(member, &args, strict, &UnionUOverV),
                None,
                "{member} strict={strict}"
            );
        }
    }
}

#[test]
fn reduce_widens_a_variant_initial_value_and_never_narrows_a_union() {
    let union = ParameterType::named("U");
    let variant = ParameterType::named("V");
    let union_list = list_of(union.clone());
    let fold = |accumulator: &ParameterType, item: &ParameterType| {
        ParameterType::func(vec![accumulator.clone(), item.clone()], accumulator.clone())
    };
    for member in ["collections.reduce", "collections.reduceRight"] {
        // bug-988: `U` is the variant from the initial value, then the union
        // from the callback: the accumulator is the union, and the initial
        // value's expected type is the union, so it is wrapped.
        let args = [union_list.clone(), variant.clone(), fold(&union, &union)];
        assert_eq!(
            resolve_call_typed_with_kinds(member, &args, true, &UnionUOverV),
            Some(union.clone()),
            "{member}"
        );
        assert_eq!(
            resolved_parameter_type(member, 1, &args, None, &UnionUOverV),
            Some(union.clone()),
            "{member}"
        );
        // One direction only: a union initial value with a variant accumulator
        // is rejected, over a variant list and over a union list.
        for list in [list_of(variant.clone()), union_list.clone()] {
            let args = [list, union.clone(), fold(&variant, &variant)];
            assert_eq!(
                resolve_call_typed_with_kinds(member, &args, true, &UnionUOverV),
                None,
                "{member}"
            );
            assert_eq!(
                resolve_call_typed_with_kinds(member, &args, false, &UnionUOverV),
                None,
                "{member}"
            );
        }
        // A callback that RETURNS the variant for a union accumulator is not a
        // `FUNC(U, T) AS U`: callable types do not convert.
        let narrowing = ParameterType::func(vec![union.clone(), union.clone()], variant.clone());
        assert_eq!(
            resolve_call_typed_with_kinds(
                member,
                &[union_list.clone(), variant.clone(), narrowing],
                true,
                &UnionUOverV,
            ),
            None,
            "{member}"
        );
    }
}

#[test]
fn select_rejects_inconsistent_variable_binding() {
    // set(List OF T, T) AS List OF T — the element must match the list's element.
    let set = func(
        "set",
        vec![generic_impl(
            vec![list_of(ParameterType::var("T")), ParameterType::var("T")],
            list_of(ParameterType::var("T")),
        )],
    );
    assert!(set
        .dispatch(&call(&["List OF Integer", "Integer"]))
        .is_some());
    // T is bound to Integer by arg 0, then contradicted by a String element.
    assert!(set
        .dispatch(&call(&["List OF Integer", "String"]))
        .is_none());
}

#[test]
fn select_unknown_argument_is_a_wildcard() {
    use ParameterType::Integer;
    let get = func(
        "get",
        vec![generic_impl(
            vec![list_of(ParameterType::var("T")), Integer],
            ParameterType::var("T"),
        )],
    );
    // An Unknown collection leaves `T` unbound, so there is no concrete return.
    assert!(get.dispatch(&call(&["Unknown", "Integer"])).is_none());
    // An Unknown index still binds `T` from the concrete list.
    assert_eq!(
        get.dispatch(&call(&["List OF Integer", "Unknown"]))
            .unwrap()
            .return_type
            .name(),
        "Integer"
    );
}
