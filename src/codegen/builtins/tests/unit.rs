use super::*;

/// Every documented builtin, as `package.function`, read from the man pages
/// (`src/docs/man/builtins/<package>/<function>.{md,txt}`).
///
/// Both extensions, deliberately. This used to filter on `txt` alone, so the
/// migrated Markdown pages — by then most of the corpus — were checked by
/// nothing, and the `> 100` floor hid it: the metric was **inverted**, since
/// every page migrated shrank the guarded set while the assertion kept
/// passing (bug-336-S3). The floor is now an exact-ish lower bound on the
/// whole corpus rather than a number the legacy half alone could satisfy.
fn documented_builtins() -> Vec<String> {
    // Man rendering became registry-driven (the Markdown `src/docs/man`
    // corpus was retired), so enumerate the builtin members from the clean-
    // room registry — the source of truth the man pages are now generated
    // from — rather than a directory of pages.
    let mut names = Vec::new();
    for package in crate::codegen::registry::registry().packages() {
        for function in package.functions() {
            names.push(format!("{}.{}", package.import_name(), function.name));
        }
    }
    assert!(
        names.len() > 100,
        "expected the registry builtin corpus, got {} members",
        names.len()
    );
    names
}

#[test]
fn qualified_builtin_type_requires_matching_package() {
    // bug-98: the member type must belong to the named package. A valid
    // pairing resolves to the bare type; a cross pairing (right type, wrong
    // package) must not.
    assert_eq!(
        qualified_builtin_type("net.Url"),
        Some("net.Url".to_string())
    );
    // `Url` is a net type, not an io/csv type — these must be rejected.
    assert_eq!(qualified_builtin_type("io.Url"), None);
    assert_eq!(qualified_builtin_type("crypto.Url"), None);
    // A non-builtin package is rejected outright.
    assert_eq!(qualified_builtin_type("csv.Thread"), None);
    // A bare (unqualified) name is not a qualified type.
    assert_eq!(qualified_builtin_type("Url"), None);
}

#[test]
fn no_named_argument_alias_repeats_across_positions() {
    // `call_param_names` resolves a name to the *first* position group that
    // lists it, with no backtracking. An alias appearing in two groups is
    // therefore unresolvable: it pins to the earlier position and collides
    // with that parameter (bug-28, `net.connectTcp`'s `timeoutMs`). A builtin
    // whose overloads genuinely disagree on a name's position must declare a
    // per-overload table instead.
    for name in documented_builtins() {
        let Some(groups) = call_param_names(&name) else {
            continue;
        };
        for (index, aliases) in groups.iter().enumerate() {
            for alias in aliases {
                let earlier = groups[..index].iter().any(|group| group.contains(alias));
                assert!(
                    !earlier,
                    "`{name}` lists the argument name `{alias}` at two positions; \
                         a named `{alias}` can never bind to position {index}"
                );
            }
        }
    }
}

#[test]
fn overloaded_param_name_tables_are_well_formed() {
    for name in documented_builtins() {
        let Some(overloads) = call_param_name_overloads(&name) else {
            continue;
        };
        // A per-overload table replaces the merged one; carrying both would
        // leave the merged table silently unused.
        assert!(
            call_param_names(&name).is_none(),
            "`{name}` declares both a merged and a per-overload param table"
        );
        for params in &overloads {
            for (index, param) in params.iter().enumerate() {
                assert!(
                    !params[..index].contains(param),
                    "`{name}` repeats the parameter `{param}` in one overload"
                );
            }
        }
        // Two overloads of the same arity must differ by name, or selection
        // between them would be arbitrary.
        for (index, params) in overloads.iter().enumerate() {
            for other in &overloads[..index] {
                assert!(
                    params.len() != other.len() || params != other,
                    "`{name}` declares the same overload twice"
                );
            }
        }
    }
}

/// bug-596: a named call may leave out an overloaded builtin's TRAILING
/// optional parameters. `tls::connect`'s overloads each end in a defaulted
/// `allowSelfSigned`; exact-arity selection rejected every named call that
/// omitted it, including the documented `timeoutMs :=, serverName :=` form.
#[test]
fn a_named_call_may_omit_an_overloaded_builtins_trailing_optionals() {
    let overloads = call_param_name_overloads("tls.connect")
        .expect("tls.connect's overloads disagree on layout, so it has a per-overload table");
    let required = call_param_name_overload_required("tls.connect").expect("required counts");
    assert_eq!(
        required,
        vec![2, 1],
        "host/port are required; address is required"
    );
    let host_form = ["host", "port", "timeoutMs", "serverName", "allowSelfSigned"];
    let address_form = ["address", "timeoutMs", "serverName", "allowSelfSigned"];
    assert_eq!(
        select_param_name_overload(&overloads, &required, 2, &["timeoutMs", "serverName"]),
        Some(&host_form[..]),
        "positional host/port, then named timeoutMs and serverName"
    );
    assert_eq!(
        select_param_name_overload(&overloads, &required, 2, &["timeoutMs"]),
        Some(&host_form[..])
    );
    assert_eq!(
        select_param_name_overload(&overloads, &required, 1, &["timeoutMs", "serverName"]),
        Some(&address_form[..]),
        "the same names bind one slot earlier on the Address form"
    );
    // POSITIVE: a call naming every parameter still selects by exact arity.
    assert_eq!(
        select_param_name_overload(
            &overloads,
            &required,
            2,
            &["timeoutMs", "serverName", "allowSelfSigned"]
        ),
        Some(&host_form[..])
    );
}

/// bug-596's guard for bug-349: the fallback may only leave out DEFAULTED
/// slots. `datetime::instant`'s five overloads drop REQUIRED components off
/// the front, so `instant(days := 5)` must select nothing — prefix-filling the
/// 5-arg form would type-check its one argument against the 1-arg `seconds`
/// form and bind 5 days as 5 seconds. Found when the first cut of this fix
/// turned `bug349_instant_named_arg_arity_invalid`'s three errors into an
/// accepted program.
#[test]
fn the_fallback_never_leaves_out_a_required_parameter() {
    for (name, calls) in [
        (
            "datetime.instant",
            &[&["days"][..], &["days", "hours"][..]][..],
        ),
        ("datetime.duration", &[&["hours"][..]][..]),
    ] {
        let overloads = call_param_name_overloads(name).expect("per-overload table");
        let required = call_param_name_overload_required(name).expect("required counts");
        for names in calls {
            assert_eq!(
                select_param_name_overload(&overloads, &required, 0, names),
                None,
                "`{name}` with {names:?} must select no overload"
            );
        }
    }
    // Fail closed: with no required counts, no fallback at all.
    let table = vec![vec!["a", "b", "c"]];
    assert_eq!(select_param_name_overload(&table, &[], 0, &["a"]), None);
    assert_eq!(
        select_param_name_overload(&table, &[1], 0, &["a"]),
        Some(&["a", "b", "c"][..])
    );
}

/// bug-596's negative half: the prefix rule fills only TRAILING slots, so a
/// gap before a later supplied name is still not selected (it stays the
/// located "omits parameter" diagnostic), and exact arity still wins.
#[test]
fn a_named_call_never_selects_an_overload_with_a_gap_before_a_supplied_name() {
    let overloads = call_param_name_overloads("tls.connect").expect("per-overload table");
    let required = call_param_name_overload_required("tls.connect").expect("required counts");
    // Two positionals then `serverName`: on the host form that leaves a gap at
    // `timeoutMs`, so the host form is NOT selected. The Address form takes it
    // structurally — its second positional IS `timeoutMs`, which makes
    // `tls::connect(addr, 5000, serverName := "x")` legal — and the type check
    // then rejects a host/port call against it. Names alone cannot tell the
    // two apart; what matters is that the gapped host form is never chosen.
    let address_form = ["address", "timeoutMs", "serverName", "allowSelfSigned"];
    assert_eq!(
        select_param_name_overload(&overloads, &required, 2, &["serverName"]),
        Some(&address_form[..])
    );
    assert_eq!(
        select_param_name_overload(&overloads, &required, 2, &["allowSelfSigned"]),
        None
    );
    // Synthetic table: an exact-arity overload is preferred over a longer
    // one the same names would also prefix-fill.
    let table = vec![vec!["a", "b", "c"], vec!["a", "b"]];
    assert_eq!(
        select_param_name_overload(&table, &[1, 1], 1, &["b"]),
        Some(&["a", "b"][..])
    );
    // `tcp::connect(host := "h", timeoutMs := 5000)` omits `port`: still no
    // overload, so `func_tcp_invalid`'s pinned diagnostic is unchanged.
    let tcp = vec![
        vec!["host", "port"],
        vec!["host", "port", "timeoutMs"],
        vec!["address"],
        vec!["address", "timeoutMs"],
    ];
    assert_eq!(
        select_param_name_overload(&tcp, &[2, 3, 1, 2], 0, &["host", "timeoutMs"]),
        None
    );
}

/// bug-644: a member that DECLARES an error is fallible by the registry, and the
/// hand-written infallible name list must not overrule it. `sum` sat on that
/// list after plan-96 gave it an `ErrOverflow` check, so the front end warned
/// TYPE_INLINE_TRAP_DEAD_HANDLER on a handler that runs. Every other `collections`
/// member the list names is checked against the registry here.
#[test]
fn no_member_declaring_an_error_is_infallible() {
    for member in [
        "contains",
        "hasKey",
        "keys",
        "values",
        "sum",
        "getOr",
        "append",
        "prepend",
        "removeKey",
        "replace",
        "add",
        "remove",
        "toList",
    ] {
        let target = format!("collections.{member}");
        if crate::codegen::registry::native_member_declares_error(&target) == Some(true) {
            assert!(
                !inline_builtin_is_infallible(
                    &target,
                    &[ParameterType::list_of(ParameterType::Integer)],
                    &NoTypeKinds
                ),
                "{target} declares an error, so an inline TRAP handler on it is live"
            );
        }
    }
}

#[test]
fn inline_builtin_fallibility_census() {
    // Infallible-for-TRAP: raise no user-trappable domain error (plan-21-A).
    // `toString` is fallible since the owner's 2026-10-04 decision (every
    // overload can raise `ErrOutOfMemory`); `tostring_is_fallible_on_every_overload`.
    for c in [
        "len",
        "typeName",
        "bits.band",
        "bits.bor",
        "bits.rl64",
        "bits.clz",
        "bits.popCount",
        "collections.contains",
        "collections.hasKey",
        "collections.getOr",
    ] {
        assert!(
            inline_builtin_is_infallible(c, &[], &NoTypeKinds),
            "expected infallible: {c}"
        );
    }
    // Allocating collections members declare ErrOutOfMemory (bug-698), so a
    // handler on them is live.
    for c in [
        "collections.keys",
        "collections.values",
        "collections.append",
        "collections.prepend",
        "collections.removeKey",
    ] {
        assert!(
            !inline_builtin_is_infallible(c, &[], &NoTypeKinds),
            "expected fallible (ErrOutOfMemory): {c}"
        );
    }
    // Fallible inline members: a real domain error (index/range/not-found), an
    // out-of-range shift count, or a failing callback.
    for c in [
        "bits.sl",
        "bits.sr",
        "bits.sra",
        "collections.get",
        "collections.set",
        "collections.insert",
        "collections.removeAt",
        "collections.find",
        "strings.mid",
        "strings.find",
        "collections.forEach",
        "collections.transform",
        "collections.filter",
        "collections.reduce",
        // bug-644: every `sum` overload fails — `Integer`/`Fixed` raise
        // `ErrOverflow`, and the `Float` total is observed inside the capture
        // and raises `ErrFloatOverflow`. It sat on the infallible list from
        // before plan-96 gave it an overflow check, so the front end warned
        // TYPE_INLINE_TRAP_DEAD_HANDLER on a live handler.
        "collections.sum",
        // bug-533: `strings.replace` and `collections.replace` dequalify to
        // one bare native target, and the `String` overload now refuses an
        // empty `old`. Untyped, the fail-closed rule answers "fallible" for
        // both spellings; the per-overload verdict is
        // `replace_is_fallible_only_on_a_string` below.
        "strings.replace",
        "collections.replace",
    ] {
        assert!(
            !inline_builtin_is_infallible(c, &[], &NoTypeKinds),
            "expected fallible: {c}"
        );
    }
    // Every inline member is classified one way or the other, and non-inline
    // callees (user functions) are not infallible built-ins.
    assert!(!inline_builtin_is_infallible("myFunc", &[], &NoTypeKinds));
    assert!(!inline_builtin_is_infallible(
        "math.sqrt",
        &[],
        &NoTypeKinds
    ));
}

/// bug-486 made `toString(<List OF Byte>)` fallible (it raises `ErrEncoding`).
/// Since bug-807 every overload declares `ErrOutOfMemory`, and the owner's rule
/// (2026-10-04) is that a call that can run out of memory is fallible: every
/// `toString` overload is fallible and raw-supported, so an inline `TRAP`
/// handler on it is live and never flagged `TYPE_INLINE_TRAP_DEAD_HANDLER`.
#[test]
fn tostring_is_fallible_on_every_overload() {
    let bytes = [ParameterType::list_of(ParameterType::Byte)];
    for args in [
        bytes.to_vec(),
        vec![ParameterType::Integer],
        vec![ParameterType::Float, ParameterType::Byte],
        vec![ParameterType::String],
        vec![ParameterType::list_of(ParameterType::Integer)],
        vec![ParameterType::Unknown],
        vec![],
    ] {
        assert!(
            !inline_builtin_is_infallible("toString", &args, &NoTypeKinds),
            "expected fallible: toString{args:?}"
        );
        assert!(inline_builtin_raw_supported("toString", &args));
        assert!(!inline_trap_unsupported("toString", &args, &NoTypeKinds));
    }

    // The cheap gate every consumer uses to decide whether to type its
    // arguments must agree with the rule it gates: a name the gate skips can
    // never be flipped by an argument type, or the consumer would skip the
    // typing and get the wrong verdict.
    for name in [
        "toString",
        "len",
        "typeName",
        "collections.get",
        "strings.mid",
        "bits.sl",
        "myFunc",
    ] {
        assert!(
            inline_builtin_fallibility_depends_on_args(name)
                || !arg_type_makes_inline_builtin_fallible(name, &bytes),
            "{name} is flipped by an argument type but the gate skips typing it"
        );
    }

    // `len` and `typeName` were audited and have no fallible overload, so a
    // byte-list argument must not flip them (`lower_len`'s two arms emit no
    // error return; `typeName` folds to a string constant at compile time).
    for name in ["len", "typeName"] {
        assert!(
            inline_builtin_is_infallible(name, &bytes, &NoTypeKinds),
            "expected infallible: {name}(List OF Byte)"
        );
    }
}

/// bug-533: the second argument-typed name, and the one that made the
/// name-keyed census a MISCOMPILE rather than a missed optimisation.
/// `strings::replace` and `collections::replace` dequalify to the same bare
/// native target `replace`. Since bug-533 the `String` overload refuses an
/// empty `old` with `ErrInvalidArgument`; the `List` overload cannot fail,
/// because an empty *element* is an ordinary value and its lowering returns
/// before the guard. With `replace` on the name-keyed infallible list the
/// front-end warned `TYPE_INLINE_TRAP_DEAD_HANDLER` on a LIVE handler, elided
/// it, and the guarded call aborted the program instead of recovering.
///
/// The four `strings` members that RAISE `ErrInvalidArgument` and used to
/// declare no error at all.
///
/// `strings::left`/`right` (negative `count`) and `padLeft`/`padRight`
/// (negative `width`, a `padChar` that is not exactly one scalar, an
/// unrepresentable result size) all raise through `raise_error_bare`, which
/// skips the "declares what it raises" assertion. Their descriptors said
/// `errors: vec![]`, so `native_member_declares_error` answered `Some(false)`
/// and the front-end proved the call infallible — eliding a LIVE inline
/// `TRAP` handler and letting `77050002` abort the program. Same shape as
/// bug-486 (`toString`) and bug-533 (`replace`).
///
/// The verdict is registry-derived, so declaring the error is the whole fix:
/// it makes the member fallible AND raw-supported through the same
/// `native_member_declares_error` query, and `try_abi_inline_lower` already
/// runs an `abi_inline` body under the raw capture.
#[test]
fn the_raising_strings_members_are_not_infallible() {
    for name in [
        "strings.left",
        "strings.right",
        "strings.padLeft",
        "strings.padRight",
        // bug-731: these three build a new String (case-mapped/trimmed) through
        // the same allocating path as every other result-building `strings`
        // member, so they can raise ErrOutOfMemory too — measured directly by
        // lowering their real body and scanning the emitted instructions for
        // the raise (`declared_errors_all.rs`'s `every_strings_member_declares_
        // exactly_the_errors_its_lowering_raises`), not inferred. They used to
        // be believed infallible; that was the bug.
        "strings.upper",
        "strings.lower",
        "strings.trim",
    ] {
        assert!(
            !inline_builtin_is_infallible(name, &[], &NoTypeKinds),
            "{name} raises ErrOutOfMemory (and, for left/right/padLeft/padRight, \
                 ErrInvalidArgument too); an infallible verdict elides a live handler"
        );
        assert!(
            inline_builtin_raw_supported(name, &[]),
            "{name} is fallible, so an inline TRAP needs a raw lowering"
        );
        assert!(!inline_trap_unsupported(name, &[], &NoTypeKinds));
        // The verdict does not depend on the argument types, so a consumer
        // that skips typing them still gets the fallible answer.
        assert!(!inline_builtin_fallibility_depends_on_args(name));
    }

    // The POSITIVE half: the neighbours that genuinely cannot fail keep their
    // infallible verdict. A fix that made every `strings` member fallible
    // would pass the assertions above and fail here.
    for name in ["strings.byteLen", "strings.contains"] {
        assert!(
            inline_builtin_is_infallible(name, &[], &NoTypeKinds),
            "{name} raises nothing trappable and must stay infallible"
        );
        assert!(!inline_builtin_raw_supported(name, &[]));
    }
}

/// The rule fails CLOSED: only a provable `List` first argument is
/// infallible. Over-approximating keeps a handler that could not have run;
/// the other direction is the miscompile above.
#[test]
fn replace_is_fallible_only_on_a_string() {
    for name in ["strings.replace", "collections.replace"] {
        // The `String` overload is fallible, and therefore raw-supported so
        // an inline `TRAP` on it traps the real error rather than being
        // rejected for having no lowering.
        let strings = [
            ParameterType::String,
            ParameterType::String,
            ParameterType::String,
        ];
        assert!(
            !inline_builtin_is_infallible(name, &strings, &NoTypeKinds),
            "expected fallible: {name}(String, String, String)"
        );
        assert!(inline_builtin_raw_supported(name, &strings));
        assert!(!inline_trap_unsupported(name, &strings, &NoTypeKinds));

        // The `List` overload is fallible too since bug-698: it allocates the
        // result and declares `ErrOutOfMemory`, so an inline `TRAP` on it has a
        // live handler and a raw lowering, for every element type.
        for element in [
            ParameterType::Integer,
            ParameterType::String,
            ParameterType::Byte,
        ] {
            let list = [
                ParameterType::list_of(element.clone()),
                element.clone(),
                element,
            ];
            assert!(
                !inline_builtin_is_infallible(name, &list, &NoTypeKinds),
                "expected fallible (ErrOutOfMemory): {name}{list:?}"
            );
            assert!(inline_builtin_raw_supported(name, &list));
        }

        // Fail closed: an untyped or unknown first argument is fallible.
        for args in [vec![], vec![ParameterType::Unknown]] {
            assert!(
                !inline_builtin_is_infallible(name, &args, &NoTypeKinds),
                "expected fallible (fail closed): {name}{args:?}"
            );
        }

        // The cheap gate must agree, or a consumer would skip typing the
        // arguments and take the name-keyed answer.
        assert!(inline_builtin_fallibility_depends_on_args(name));
    }

    // The rule is `replace`'s alone: the sibling intrinsics that share the
    // dequalifying path must be untouched by a String first argument.
    let strings = [
        ParameterType::String,
        ParameterType::String,
        ParameterType::String,
    ];
    assert!(!inline_builtin_fallibility_depends_on_args("strings.mid"));
    assert!(!inline_builtin_fallibility_depends_on_args("strings.find"));
    assert!(!arg_type_makes_inline_builtin_fallible(
        "strings.mid",
        &strings
    ));
}

/// An oracle that knows exactly one enum, `Color` — the bug-679 twin of the
/// one `general::tests` uses for the plan-140 resolver cases.
struct ColorIsAnEnum;
impl TypeKinds for ColorIsAnEnum {
    fn is_enum(&self, t: &ParameterType) -> bool {
        t.is_named("Color")
    }
}

/// bug-679: `toInt(<enum>)` is the ordinal the value already holds — a
/// register move that declares no error (plan-140-B) — so it is infallible.
/// (`toString(<enum>)` is not its twin: since bug-807 every `toString`
/// overload allocates and is fallible.) Every other `toInt` overload really can
/// fail (bad parse, overflow) and must stay fallible: the name is fallible by
/// default and this one overload is the exception, which is the mirror of
/// bug-486's rule rather than a copy of it.
#[test]
fn toint_is_infallible_only_on_an_enum() {
    let color = [ParameterType::named("Color")];
    assert!(
        inline_builtin_is_infallible("toInt", &color, &ColorIsAnEnum),
        "toInt(<enum>) cannot fail"
    );

    // Without an oracle the answer is the over-approximating one: a site that
    // cannot tell an enum from a record keeps today's verdict.
    assert!(!inline_builtin_is_infallible("toInt", &color, &NoTypeKinds));

    // Every other overload stays fallible, under either oracle. `Shape` is a
    // declared type the oracle does NOT call an enum, so a "is a declared
    // type" spelling heuristic would wrongly pass it.
    for args in [
        vec![ParameterType::String],
        vec![ParameterType::Float],
        vec![ParameterType::Fixed],
        vec![ParameterType::Money],
        vec![ParameterType::Byte],
        vec![ParameterType::named("Shape")],
        vec![ParameterType::Unknown],
        // The two-argument radix form, even over the enum type.
        vec![ParameterType::String, ParameterType::Integer],
        vec![ParameterType::named("Color"), ParameterType::Integer],
        vec![],
    ] {
        assert!(
            !inline_builtin_is_infallible("toInt", &args, &ColorIsAnEnum),
            "expected fallible: toInt{args:?}"
        );
    }

    // The cheap gate every consumer uses to decide whether to type its
    // arguments must name `toInt`, or a consumer would pass an empty slice
    // and never reach the rule above.
    assert!(inline_builtin_fallibility_depends_on_args("toInt"));

    // The rule GRANTS infallibility; it must not also start removing it, or
    // `inline_builtin_raw_supported` would claim a raw lowering `toInt` does
    // not have on this path.
    assert!(!arg_type_makes_inline_builtin_fallible("toInt", &color));
    assert!(!inline_builtin_raw_supported("toInt", &color));
    assert!(!inline_trap_unsupported("toInt", &color, &NoTypeKinds));
}

#[test]
fn inline_builtin_raw_supported_set() {
    // The fallible inline members with a raw-`Result` inline lowering
    // (plan-21-B): an inline TRAP on them compiles instead of being rejected.
    for c in [
        "collections.get",
        "collections.set",
        "collections.insert",
        "collections.removeAt",
        "collections.find",
        "strings.find",
        "strings.mid",
        "bits.sl",
        "bits.sr",
        "bits.sra",
    ] {
        assert!(
            inline_builtin_raw_supported(c, &[]),
            "expected raw-supported: {c}"
        );
        assert!(
            !inline_trap_unsupported(c, &[], &NoTypeKinds),
            "raw-supported must not be unsupported: {c}"
        );
    }
    // The callback members are now raw-supported too (plan-26-B).
    for c in [
        "collections.forEach",
        "collections.transform",
        "collections.filter",
        "collections.reduce",
    ] {
        assert!(
            inline_builtin_raw_supported(c, &[]),
            "expected raw-supported: {c}"
        );
        assert!(
            !inline_trap_unsupported(c, &[], &NoTypeKinds),
            "raw-supported must not be unsupported: {c}"
        );
    }
    // The infallible members are NOT raw-supported (nothing to capture) but are
    // still trappable via the always-`Ok` path — so also not unsupported.
    for c in ["collections.contains", "len", "bits.band"] {
        assert!(
            !inline_builtin_raw_supported(c, &[]),
            "expected NOT raw-supported: {c}"
        );
        assert!(
            !inline_trap_unsupported(c, &[], &NoTypeKinds),
            "infallible must not be unsupported: {c}"
        );
    }
}

/// The full import-gated package set. Kept in one place so the `is_builtin_import`
/// predicate and the `mfb spec language builtin-functions` §18 list cannot drift
/// apart (plan-33-D Phase 2 — the earlier `money` omission recurred because no
/// such test existed).
///
/// plan-122-A: this was a hand-written *copy* of the predicate's arm, and a copy
/// is exactly what the test was supposed to make impossible — it silently omitted
/// `tcp` and `udp`, so §18's package sentence omitted them too while §18's own
/// transport paragraph documented `IMPORT tcp`/`IMPORT udp`. It is now an alias
/// for the production list, so the §18 comparison below is against the predicate
/// itself and no third spelling exists to drift.
const ALL_BUILTIN_PACKAGES: &[&str] = super::BUILTIN_IMPORTS;

#[test]
fn every_package_is_a_builtin_import() {
    for pkg in ALL_BUILTIN_PACKAGES {
        assert!(is_builtin_import(pkg), "is_builtin_import missing {pkg}");
    }
    assert!(!is_builtin_import("audioo"));
    assert!(!is_builtin_import("resource"));
}

#[test]
fn spec_section_18_package_list_matches_is_builtin_import() {
    // Extract the backtick-quoted package names from §18's "package set the
    // resolver recognizes is fixed:" sentence and assert it equals the
    // canonical set exactly (no missing, no extra).
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/docs/spec/language/18_builtin-functions.md"),
    )
    .expect("read §18 spec");
    let anchor = "The package set the resolver recognizes is fixed:";
    let start = doc.find(anchor).expect("§18 package-set sentence");
    // The sentence runs from the anchor to the citation marker that follows it.
    let rest = &doc[start..];
    let end = rest
        .find("[[src/codegen/builtins/mod.rs:is_builtin_import]]")
        .expect("§18 citation");
    let sentence = &rest[..end];
    let mut listed: Vec<String> = Vec::new();
    let mut chars = sentence.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            let mut name = String::new();
            for c in chars.by_ref() {
                if c == '`' {
                    break;
                }
                name.push(c);
            }
            listed.push(name);
        }
    }
    let mut expected: Vec<String> = ALL_BUILTIN_PACKAGES.iter().map(|s| s.to_string()).collect();
    listed.sort();
    expected.sort();
    assert_eq!(
        listed, expected,
        "§18 package list drifted from is_builtin_import; \
             update src/docs/spec/language/18_builtin-functions.md"
    );
}

#[test]
fn is_builtin_import_cases() {
    for pkg in [
        "app",
        "bits",
        "collections",
        "crypto",
        "csv",
        "datetime",
        "encoding",
        "errorCode",
        "fs",
        "http",
        "io",
        "json",
        "math",
        "money",
        "net",
        "regex",
        "strings",
        "term",
        "thread",
        "tls",
        "vector",
        "color",
    ] {
        assert!(is_builtin_import(pkg), "{pkg}");
    }
    assert!(!is_builtin_import("nope"));
    assert!(!is_builtin_import("resource"));
}

#[test]
fn is_builtin_type_aggregates() {
    // A thread type routes through the registry's type table. plan-106-C
    // deleted the `builtins::is_builtin_type` wrapper — the former source checker's parser
    // was its last caller, and that call discarded the answer — so these
    // assertions follow it to the registry.
    assert!(crate::codegen::registry::registry().is_builtin_type("Thread"));
    assert!(!crate::codegen::registry::registry().is_builtin_type("Integer"));
    assert!(!crate::codegen::registry::registry().is_builtin_type("List OF Integer"));
}

#[test]
fn general_override_target_cases() {
    // Keyed by the type's package-qualified IDENTITY. `URL_TYPE` is the bare
    // member id the descriptor row declares (`Url`); the identity a `net::Url`
    // value carries is `net.Url` (bug-480 Phase 4b).
    assert_eq!(
        general_override_target(
            "toString",
            &crate::types::ParameterType::parse(&format!(
                "net.{}",
                crate::codegen::builtins::net::URL_TYPE
            )),
        ),
        Some("__net_urlToString")
    );
    // bug-668: the BARE leaf is not the override's key. A bare `Url` can only
    // be a user `TYPE Url` — a consumer cannot spell the built-in type that
    // way — and answering `net`'s helper for it routed a program that never
    // imported `net` to a symbol no package emitted.
    assert_eq!(
        general_override_target(
            "toString",
            &crate::types::ParameterType::parse(crate::codegen::builtins::net::URL_TYPE),
        ),
        None
    );
    assert_eq!(
        general_override_target("toString", &crate::types::ParameterType::parse("Integer")),
        None
    );
    assert_eq!(
        general_override_target("len", &crate::types::ParameterType::parse("net.Url")),
        None
    );
}

#[test]
fn qualified_builtin_type_cases() {
    // net.Url -> bare Url type id.
    let url = qualified_builtin_type("net.Url");
    assert_eq!(url.as_deref(), Some("net.Url"));
    // Not a builtin package.
    assert_eq!(qualified_builtin_type("mymod.Thing"), None);
    // Builtin package, non-type member.
    assert_eq!(qualified_builtin_type("net.notAType"), None);
    // No dot at all.
    assert_eq!(qualified_builtin_type("Url"), None);
}

#[test]
fn resource_helpers() {
    // File is a builtin resource type.
    assert!(is_resource_type(&crate::types::ParameterType::declared(
        "fs.File"
    )));
    assert!(!is_resource_type(&crate::types::ParameterType::declared(
        "Integer"
    )));
    assert!(resource_close_function(&crate::types::ParameterType::declared("fs.File")).is_some());
    assert!(resource_close_function(&crate::types::ParameterType::declared("Integer")).is_none());
    // is_thread_sendable_resource_type routes to resource module.
    let _ = is_thread_sendable_resource_type(&crate::types::ParameterType::declared("fs.File"));
}

#[test]
fn native_builtin_target_cases() {
    assert_eq!(native_builtin_target("strings.find"), Some("find"));
    assert_eq!(native_builtin_target("strings.mid"), Some("mid"));
    assert_eq!(native_builtin_target("strings.replace"), Some("replace"));
    assert_eq!(native_builtin_target("strings.other"), None);
    assert_eq!(native_builtin_target("collections.get"), Some("get"));
    assert_eq!(
        native_builtin_target("collections.transform"),
        Some("transform")
    );
    assert_eq!(native_builtin_target("collections.sum"), Some("sum"));
    assert_eq!(native_builtin_target("collections.sort"), None);
    assert_eq!(native_builtin_target("nope"), None);
}

#[test]
fn inline_trap_unsupported_cases() {
    // Post plan-26 every inline builtin is trappable — infallible ones via the
    // always-`Ok` path, fallible ones via a raw capture — so `inline_trap_
    // unsupported` (the codegen backstop for a future un-lowered builtin) is
    // false for all of them.
    for target in [
        "bits.sl",               // raw-supported fallible bits shift
        "bits.band",             // infallible bits op
        "len",                   // infallible general builtin
        "toString",              // raw-supported fallible general builtin (bug-807)
        "typeName",              // infallible general builtin
        "collections.contains",  // infallible collection query
        "collections.transform", // raw-supported callback member (plan-26-B)
        "collections.forEach",   // raw-supported callback member (plan-26-B)
        "collections.get",       // raw-supported index member (plan-21-B)
        "toInt",                 // conversion builtin (own raw lowering)
        "nope",                  // not a builtin at all
    ] {
        assert!(
            !inline_trap_unsupported(target, &[], &NoTypeKinds),
            "expected trappable (not unsupported): {target}"
        );
    }
}

#[test]
fn call_return_type_name_aggregates() {
    // general
    assert_eq!(call_return_type_name("toInt").as_deref(), Some("Integer"));
    // strings::find contributes a return type through the aggregate.
    assert_eq!(
        call_return_type_name("strings.find").as_deref(),
        Some("Integer")
    );
    assert_eq!(call_return_type_name("nope").as_deref(), None);
}

#[test]
fn is_nonescaping_callback_arg_cases() {
    assert!(is_nonescaping_callback_arg("forEach", 1));
    assert!(is_nonescaping_callback_arg("collections.forEach", 1));
    assert!(!is_nonescaping_callback_arg("forEach", 0));
    assert!(!is_nonescaping_callback_arg("transform", 1));
}

#[test]
fn is_builtin_call_aggregates() {
    assert!(is_builtin_call("collections.get")); // collections
    assert!(is_builtin_call("len")); // general
    assert!(is_builtin_call("thread.start")); // thread
    assert!(is_builtin_call("toInt")); // via call_return_type_name
    assert!(!is_builtin_call("nope"));
}

#[test]
fn is_builtin_member_and_package_constant() {
    assert!(is_package_constant("math.pi"));
    assert!(is_builtin_member("math.pi"));
    assert!(is_builtin_member("len"));
    assert!(!is_builtin_member("nope"));
    assert!(!is_package_constant("nope"));
}

#[test]
fn package_constant_type_and_value() {
    assert!(package_constant_type("math.pi").is_some());
    assert!(package_constant_type("nope").is_none());
    assert!(package_constant_value("math.pi").is_some());
    assert!(package_constant_value("nope").is_none());
}

#[test]
fn call_param_names_aggregates() {
    // general
    assert!(call_param_names("len").is_some());
    // collections
    assert!(call_param_names("collections.get").is_some());
    // thread
    assert!(call_param_names("thread.start").is_some());
    assert!(call_param_names("nope").is_none());
}
