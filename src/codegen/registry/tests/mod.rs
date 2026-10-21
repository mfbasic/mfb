use super::*;

mod qualify_shapes;
mod rendering_and_gates;
mod selection_and_thread_probes;
mod unify_shapes;

/// **A member's descriptor declares at least as many forms as its injected
/// source defines.**
///
/// bug-530: `encoding::utf8Encode` has TWO `__encoding_utf8Encode` bodies —
/// `AS List OF Byte` and `AS List OF Integer`, the language's only return-type
/// overload — and its descriptor carried ONE `Implementation`. Nothing failed:
/// overload selection happens over the source bodies in the monomorphizer, so
/// the only consumer that noticed was `mfb man`, which rendered a single
/// `Declaration` ending `AS List OF Byte` and hid the other form from the two
/// places a reader looks for a signature.
///
/// The check is one-directional on purpose. A descriptor may legitimately
/// declare MORE rows than there are same-named bodies — an overload routed to
/// its own differently-named body (`http::handleRequest`'s `__http_handleRequestSSL`)
/// or to a native seam has no `__pkg_member` `FUNC` at all. What is never right
/// is the reverse: a form that exists in the source and not on the page.
#[test]
fn no_member_defines_more_source_forms_than_its_descriptor_declares() {
    let mut checked = 0usize;
    for package in registry().packages() {
        let source = package.get_mfb();
        if source.is_empty() {
            continue;
        }
        for function in package.functions() {
            let needle = format!("FUNC __{}_{}(", package.import_name(), function.name);
            let defined = source.matches(&needle).count();
            if defined == 0 {
                continue;
            }
            checked += 1;
            assert!(
                defined <= function.implementations.len(),
                "{}.{}: the package source defines {defined} `{needle}` form(s) but \
                     the descriptor declares {} implementation(s), so `mfb man` cannot \
                     show them all",
                package.import_name(),
                function.name,
                function.implementations.len(),
            );
        }
    }
    assert!(
        checked > 100,
        "the census matched only {checked} members against their source"
    );
}

/// **Every range/index parameter is spelled from the documented vocabulary.**
///
/// Parameter names are PUBLIC surface — each one is usable as a named argument
/// (`collections::findLastIndex(xs, isPos, start := 2)`) — and nothing in the
/// compiler reads them, so a name is a promise no build, test or golden checks.
/// Before bug-527 the end of a range was spelled five ways across the surface
/// (`endIndex`, `finish`, `last`, `count`, and a backward scan ORIGIN misnamed
/// `endIndex`), and `endIndex` itself carried two meanings on two packages. The
/// convention is written in `./mfb spec language builtin-functions` §18.5 and
/// `.ai/man-content.md` §2.1; this is the pin that fails when a new parameter
/// leaves it.
///
/// Three rules, each of which a real descriptor broke:
///
/// 1. The retired spellings of a range end are not used anywhere. `end` itself
///    cannot be a parameter or field name at all (it is a reserved keyword),
///    which is exactly why each author invented a different replacement; the
///    convention's answer is `end` plus the noun for what the bound is —
///    `endIndex`, `endTime`, `endPoint`, `endAngle`.
/// 2. An `end<Noun>` bound never stands alone: the same parameter list or record
///    also declares `start` or `start<SameNoun>`. A lone `end…` is a scan ORIGIN
///    wearing a bound's name, which is what `collections::findLastIndex` was.
/// 3. `start<Noun>` pairs with `end<Noun>` for the same noun. A `startIndex` with
///    no `endIndex` is a scan origin and should be a bare `start`.
///
/// Rules 2 and 3 are applied to every implementation's parameter list and to
/// every EXPORTED record's fields. A non-exported record is not public surface
/// and is exempt from the pairing rules (the `__regex_*` engine nodes carry a
/// `startPos` with no counterpart); rule 1 is tree-wide, because the retired
/// spellings are wrong wherever they appear.
#[test]
fn range_and_index_parameters_use_the_documented_vocabulary() {
    // The spellings of "the end of a range" the convention retired. `end` is in
    // the list even though the lexer already refuses it, so the reason it is
    // absent is recorded here rather than inferred from a parse error.
    const RETIRED: &[&str] = &["end", "endIdx", "endPos", "finish", "last", "stop"];

    /// The noun after a `start`/`end` prefix, or `None` when `name` is not of that
    /// shape. The capital is what separates `startPoint` (a bound) from
    /// `startsWith` (a different word that happens to begin the same way).
    fn noun<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
        let rest = name.strip_prefix(prefix)?;
        rest.chars()
            .next()
            .is_some_and(|first| first.is_ascii_uppercase())
            .then_some(rest)
    }

    fn check(where_: &str, names: &[&str], pair: bool, failures: &mut Vec<String>) {
        for name in names {
            if RETIRED.contains(name) {
                failures.push(format!(
                    "{where_}: `{name}` is a retired spelling of a range end; the \
                         convention spells it `start` + `end<Noun>` (`endIndex`, \
                         `endTime`, `endPoint`, `endAngle`)"
                ));
            }
            if !pair {
                continue;
            }
            if let Some(n) = noun(name, "end") {
                let paired = names
                    .iter()
                    .any(|other| *other == "start" || noun(other, "start") == Some(n));
                if !paired {
                    failures.push(format!(
                        "{where_}: `{name}` names the end of a range but there is \
                             no `start` or `start{n}` beside it; a bound never stands \
                             alone, and a scan origin is a bare `start`"
                    ));
                }
            }
            if let Some(n) = noun(name, "start") {
                let paired = names.iter().any(|other| noun(other, "end") == Some(n));
                if !paired {
                    failures.push(format!(
                        "{where_}: `{name}` has no `end{n}` beside it; a start with \
                             no bound is a scan origin and is spelled `start`"
                    ));
                }
            }
        }
    }

    let mut failures = Vec::new();
    let mut checked = 0usize;
    for package in registry().packages() {
        for function in package.functions() {
            for (index, implementation) in function.implementations().iter().enumerate() {
                let names: Vec<&str> = implementation
                    .params
                    .iter()
                    .map(|param| param.name)
                    .collect();
                let where_ = format!("{}::{} impl {index}", package.import_name(), function.name);
                check(&where_, &names, true, &mut failures);
                checked += names.len();
                for param in &implementation.params {
                    let where_ = format!("{where_} alias of `{}`", param.name);
                    check(&where_, param.aliases, false, &mut failures);
                    checked += param.aliases.len();
                }
            }
        }
        for record in package.records() {
            let names: Vec<&str> = record.props.iter().map(|prop| prop.name).collect();
            let where_ = format!("{}::{} record", package.import_name(), record.name);
            check(&where_, &names, record.export, &mut failures);
            checked += names.len();
        }
    }

    assert!(
        failures.is_empty(),
        "{} parameter name(s) leave the range/index vocabulary:\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
    assert!(
        checked > 1000,
        "the census only reached {checked} names, so it is not covering the surface"
    );
}

/// **Every `add_consuming_parameter` names a real member and a real parameter of
/// it.**
///
/// The lookup that reads this data
/// (`crate::codegen::resource::builtin_consuming_parameter_index`) returns `None`
/// for a member with no consuming parameter *and* for a member that does not exist
/// — the same answer. So a typo in either string is not a compile error, not a
/// runtime error, and not a wrong answer: it is **silence**, and the feature it
/// switches on simply never happens. That is the worst shape a mistake can have
/// here, because the behaviour it disables is a `TYPE_USE_AFTER_MOVE` that would
/// otherwise catch a use-after-close.
///
/// A real test rather than the `debug_assert!` in `add_function` beside it: CI
/// builds `--release` on all five platforms, so a `debug_assert!` runs nowhere in
/// it (plan-116-E **E6**).
#[test]
fn the_consuming_parameters_name_real_members() {
    for package in registry().packages() {
        for entry in package.consuming_params() {
            let function = package.function(entry.function).unwrap_or_else(|| {
                panic!(
                    "{}: add_consuming_parameter names `{}`, which is not a member \
                         of the package",
                    package.import_name(),
                    entry.function,
                )
            });
            assert!(
                function
                    .implementations
                    .iter()
                    .any(|imp| imp.params.iter().any(|p| p.name == entry.parameter)),
                "{}.{}: add_consuming_parameter names parameter `{}`, which no \
                     implementation declares (parameters are {:?})",
                package.import_name(),
                entry.function,
                entry.parameter,
                function
                    .implementations
                    .iter()
                    .flat_map(|imp| imp.params.iter().map(|p| p.name))
                    .collect::<Vec<_>>(),
            );
        }
    }
}

/// bug-1092: every row of `PLACING_OPERANDS` names a real `collections` member whose
/// every implementation has all the listed argument positions — a typo or a stale
/// index would otherwise silently disable the containment edge.
#[test]
fn the_placing_operands_name_real_members() {
    for (target, indices) in crate::codegen::resource::PLACING_OPERANDS {
        let (package_name, member) = target.split_once('.').expect("qualified member");
        let package = registry()
            .packages()
            .iter()
            .find(|p| p.import_name() == package_name)
            .unwrap_or_else(|| panic!("{target}: no such package"));
        let function = package
            .function(member)
            .unwrap_or_else(|| panic!("{target}: not a member of the package"));
        for imp in function.implementations.iter() {
            for &index in *indices {
                assert!(
                    index < imp.params.len(),
                    "{target}: placed operand {index} is past the {} declared parameters",
                    imp.params.len(),
                );
            }
        }
    }
}

/// **Every registry row satisfies the shape its `add_*` builder asserts.**
///
/// bug-550: `add_record`, `add_union`, `add_enum`, `add_function` and
/// `add_constant` each carry a `debug_assert!` stating the shape they require,
/// and `Body::mfb`/`Body::mfb_with_fast_path` assert that the body actually
/// declares the `rewrite` symbol a call is redirected to. **None of them ran
/// anywhere**: CI builds `--release` on all five platforms, which is the whole
/// of bug-550.
///
/// They stay where they are — at the construction site they give the sharper
/// message — and this walks the built registry so the same properties are
/// checked by something that actually executes. That is the shape plan-116-E
/// **E6** already chose for `the_consuming_parameters_name_real_members`
/// beside it, and it is better than promoting them to `assert!`: the registry
/// is static data, so a test pays once at test time instead of on every user's
/// compile.
///
/// It walks 105 records, 8 unions, 22 enums, 123 constants, 587 functions and
/// 363 `Mfb` bodies — measured, because a shape test that iterates nothing
/// passes for free and reads exactly like one that works.
///
/// What each row's absence would cost, since a shape check is only worth its
/// message: an empty record/union/enum renders a type with no members that
/// fails far from its declaration; a function with no implementation is a name
/// the resolver finds and codegen cannot lower; a constant setting neither
/// `value` nor `components` (or both) is read by whichever accessor asks
/// first; and an `Mfb` body that does not declare its `rewrite` target
/// redirects every call to a symbol that does not exist.
#[test]
fn every_registry_row_has_the_shape_its_builder_requires() {
    for package in registry().packages() {
        let pkg = package.import_name();
        for record in package.records() {
            assert!(
                !record.props.is_empty(),
                "{pkg}: record `{}` has no fields",
                record.name
            );
        }
        for union in package.unions() {
            assert!(
                !union.variants.is_empty(),
                "{pkg}: union `{}` has no variants",
                union.name
            );
        }
        for r#enum in package.enums() {
            assert!(
                !r#enum.variants.is_empty(),
                "{pkg}: enum `{}` has no variants",
                r#enum.name
            );
        }
        for constant in package.constants() {
            assert!(
                constant.value.is_some() != constant.components.is_some(),
                "{pkg}: constant `{}` must set exactly one of `value` (scalar) \
                     / `components` (record)",
                constant.name
            );
        }
        for function in package.functions() {
            assert!(
                !function.implementations.is_empty(),
                "{pkg}: function `{}` has no implementations",
                function.name
            );
            for implementation in &function.implementations {
                if let Body::Mfb { body, rewrite, .. } = &implementation.body {
                    assert!(
                        body.contains(rewrite),
                        "{pkg}.{}: the MFBASIC body does not declare its rewrite \
                             target `{rewrite}`, so every call redirects to a symbol \
                             that is not there",
                        function.name
                    );
                }
            }
        }
    }
}

/// **A consuming parameter sits at the same index in every overload that has it.**
///
/// The index is derived from the name by the first implementation that declares it,
/// so two overloads disagreeing would make the checker consume the wrong argument in
/// one of them — and consuming the wrong argument marks the wrong binding moved,
/// which is a false `TYPE_USE_AFTER_MOVE` on correct code or silence on broken code
/// depending on which way it lands. An overload that does not declare the parameter
/// at all is fine and contributes nothing.
#[test]
fn the_consuming_parameter_index_agrees_across_overloads() {
    for package in registry().packages() {
        for entry in package.consuming_params() {
            let Some(function) = package.function(entry.function) else {
                continue; // reported by the test above
            };
            let indices: Vec<usize> = function
                .implementations
                .iter()
                .filter_map(|imp| imp.params.iter().position(|p| p.name == entry.parameter))
                .collect();
            if let Some(first) = indices.first() {
                assert!(
                    indices.iter().all(|i| i == first),
                    "{}.{}: parameter `{}` sits at differing indices across \
                         overloads ({indices:?}); the move checker would consume the \
                         wrong argument in at least one of them",
                    package.import_name(),
                    entry.function,
                    entry.parameter,
                );
            }
        }
    }
}

/// **`canvas::setGroup`'s `items` is a consuming parameter, and it resolves to
/// argument 1.**
///
/// Pinned by value rather than only by the two structural tests above, because those
/// two are both satisfied by an empty list. A registration silently lost — a merge
/// dropping the `add_consuming_parameter` line, a rename — would leave them green.
#[test]
fn set_group_items_is_the_consuming_parameter() {
    assert_eq!(
        crate::codegen::resource::builtin_consuming_parameter_index("canvas.setGroup"),
        Some(1),
        "canvas::setGroup(name, items) — `items` is argument 1",
    );
    assert_eq!(
        crate::codegen::resource::builtin_consuming_parameter_index("canvas.present"),
        None,
        "`present` must NOT consume: a published scene never keeps an image open",
    );
}

/// **plan-116-I Phase 1's premise: a builtin record property may carry a `RES`
/// type, and it survives every registry seam a record field passes through.**
///
/// This is the letter's one unverified assumption, and it had no precedent to copy
/// from — `grep -rn 'ParameterType::res(' src/codegen/builtins/` finds **nothing**.
/// Every `Res` in a builtin descriptor today is a *parameter*
/// (`tcp/func_poll.rs`, `udp/func_poll.rs`); no record property has ever declared
/// one. plan-114-D lifted the language ban on resource-typed record fields; whether
/// the *registry* could describe one was never exercised.
///
/// The three seams a record field crosses, in the order it crosses them:
///
/// 1. **Export** — `RegistryRecord::render` writes the record out as MFBASIC source
///    that is injected and then compiled. If `ParameterType::display` did not spell the
///    `RES`, the emitted `TYPE` would declare a *value* field of the resource's
///    name, and the program would compile against the wrong type rather than fail.
/// 2. **Qualification** — a nominal leaf is rewritten to its owning package. The
///    hazard here is specific and has bitten before: a rewrite that rebuilds the
///    type from `name()` can return `Named("RES canvas.Image")` — a *nominal whose
///    text spells a resource* — instead of `Res(Named("canvas.Image"))`. The two
///    render identically and behave differently, which is the failure
///    `every_named_in_the_catalog_round_trips` exists to catch elsewhere.
/// 3. **The catalog's own leaf check** — every `Named` in a descriptor must satisfy
///    `parse(n) == Named(n)`, and the walk recurses through `Res`.
///
/// Asserted on a scratch record rather than on `Picture`/`Text`, because Phase 1
/// changes no surface: the real fields are still `ImageRef`/`FontRef` until Phase 2.
#[test]
fn a_builtin_record_property_may_carry_a_res_type() {
    let image = ParameterType::res(ParameterType::named("canvas.Image"));

    // 1. Export. The field must be spelled `RES canvas::Image` — package-separated
    //    with `::`, since that is what the injected source is parsed as.
    let record = RegistryRecord {
        name: "ResProbe",
        export: true,
        description: "",
        props: vec![RecordProp {
            name: "image",
            ty: image.clone(),
            description: "",
        }],
    };
    let rendered = record.render();
    assert!(
        rendered.contains("image AS RES canvas::Image"),
        "a `Res` property must export as a RES field; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("image AS canvas::Image"),
        "the RES was dropped, so the field exports as a VALUE of the resource's \
             name — which compiles, against the wrong type:\n{rendered}"
    );

    // 2. The wrapper survives, as a wrapper. `name()` alone cannot tell the two
    //    apart, so this asserts the VARIANT rather than the spelling.
    assert!(
        matches!(image, ParameterType::Res(_)),
        "the probe type is not a Res to begin with"
    );
    assert_eq!(image.name(), "RES canvas.Image");
    assert_ne!(
        image,
        ParameterType::named("RES canvas.Image"),
        "a Named that merely SPELLS a resource is a different value from a Res, \
             and the two render identically — this is the substitution that would make \
             every assertion above pass while the field was not a resource at all"
    );

    // 3. The catalog's leaf rule, applied to the inner nominal: a `Named` must
    //    parse back to itself, or the descriptor is spelling structure in a name.
    let ParameterType::Res(inner) = &image else {
        unreachable!("checked above");
    };
    assert_eq!(
        ParameterType::parse(&inner.name()),
        **inner,
        "the resource's nominal does not round-trip through `parse`"
    );
}

/// plan-111-C Phase 3's overload-resolution regression guard.
///
/// Collapsing the dual API means every overload is now selected from typed
/// arguments. The distinction that must survive is the one `leaf_matches`
/// draws in STRICT mode between a **resource** parameter and a **value**
/// nominal parameter — and it is asymmetric on purpose:
///
/// * a resource parameter demands exact base-resource identity, so a
///   resource UNION does not satisfy a concrete resource close-op parameter.
///   `fs::close(<some union>)` must stay rejected — the legacy exact-name
///   resolver caught it, and it is a use-after-free class error;
/// * a value nominal the registry does not declare (the bare `Json` below)
///   stays coarse; a registry value type is held to identity, its union still
///   accepting its variants
///   (`strict_matching_holds_builtin_value_types_to_their_identity`);
/// * and STATE/ownership are transparent either way (bug-427): a
///   `File STATE Cursor` argument satisfies a bare `File` parameter, and a
///   `RES ` marker matches through on either side.
///
/// Getting any of these backwards changes which overload wins, which
/// plan-111-C §Non-goals names as the single failure mode this letter must
/// not have.
#[test]
fn strict_matching_separates_resource_params_from_value_union_params() {
    let file = ParameterType::parse("fs.File");
    let stateful_file = ParameterType::parse("fs.File STATE Cursor");
    let res_file = ParameterType::parse("RES fs.File");
    let socket = ParameterType::parse("net.Socket");

    // A resource parameter: exact base identity, STATE- and RES-transparent.
    assert!(leaf_matches(&file, &file, true));
    assert!(
        leaf_matches(&file, &stateful_file, true),
        "bug-427: a stateful resource still satisfies its bare parameter"
    );
    assert!(
        leaf_matches(&file, &res_file, true),
        "the RES ownership marker is transparent to matching"
    );
    assert!(
        !leaf_matches(&file, &socket, true),
        "a DIFFERENT resource must not satisfy a concrete resource parameter"
    );
    assert!(
        !leaf_matches(&file, &ParameterType::parse("Json"), true),
        "a non-resource nominal must not widen into a concrete resource parameter"
    );
    assert!(
        !leaf_matches(&file, &ParameterType::String, true),
        "bug-443: a scalar never satisfies a nominal parameter in strict mode"
    );

    // A VALUE nominal parameter stays coarse, so a variant widens into it.
    let json = ParameterType::parse("Json");
    assert!(
        leaf_matches(&json, &ParameterType::parse("JsonNull"), true),
        "a value-union parameter must still accept a variant that widens into it"
    );
    assert!(
        !leaf_matches(&json, &ParameterType::String, true),
        "bug-443: but not a scalar"
    );

    // LENIENT mode (overload dispatch / return inference) stays coarse
    // everywhere, so an unresolved or nominally-spelled argument does not
    // perturb selection.
    assert!(leaf_matches(&file, &ParameterType::parse("Json"), false));
    assert!(leaf_matches(&json, &ParameterType::String, false));

    // Two unequal scalars never match, in either mode.
    assert!(!leaf_matches(
        &ParameterType::Integer,
        &ParameterType::String,
        true
    ));
    assert!(!leaf_matches(
        &ParameterType::Integer,
        &ParameterType::String,
        false
    ));
}

/// bug-638: a REGISTRY value type — a built-in record, enum or union — is a
/// nominal identity in strict mode, not a coarse "any nominal".
///
/// `datetime::toMillis(at AS datetime::Instant)` resolved for a
/// `datetime::DateTime` argument, so the call compiled and read the
/// `DateTime`'s leading words as an `Instant`. A record or enum parameter
/// takes exactly its own type; a union parameter takes itself or one of its
/// variants; and a callback parameter's nominal leaves are held to the same
/// rule. Lenient dispatch is unchanged.
#[test]
fn strict_matching_holds_builtin_value_types_to_their_identity() {
    let to_millis =
        |arg: &str, strict: bool| resolve_call("datetime.toMillis", &[arg.to_string()], strict);
    assert_eq!(
        to_millis("datetime.Instant", true).as_deref(),
        Some("Integer")
    );
    for wrong in [
        "datetime.DateTime",
        "datetime.Date",
        "datetime.Duration",
        "Instant",
    ] {
        assert_eq!(
            to_millis(wrong, true),
            None,
            "a `{wrong}` must not satisfy a `datetime.Instant` parameter"
        );
    }
    assert_eq!(
        to_millis("datetime.DateTime", false).as_deref(),
        Some("Integer"),
        "lenient dispatch stays coarse"
    );

    // An enum parameter takes only its own enum.
    let text = "String".to_string();
    assert!(resolve_call("crypto.hash", &["crypto.Hash".into(), text.clone()], true).is_some());
    assert_eq!(
        resolve_call(
            "crypto.hash",
            &["crypto.SymmetricCipher".into(), text],
            true
        ),
        None,
        "a different enum must not satisfy a `crypto.Hash` parameter"
    );

    // A union parameter takes itself and its variants, and nothing else.
    for accepted in ["json.Json", "json.JsonBool", "json.JsonObj"] {
        assert!(
            resolve_call("json.stringify", &[accepted.to_string()], true).is_some(),
            "a `{accepted}` must satisfy a `json.Json` parameter"
        );
    }
    assert_eq!(
        resolve_call("json.stringify", &["datetime.Instant".to_string()], true),
        None,
        "a record that is not a variant must not satisfy a union parameter"
    );
    // A variant spelled with another package's qualifier (`http.Stream`'s
    // `tcp::Socket`) widens too.
    assert!(resolve_call("http.done", &["tcp.Socket".to_string()], true).is_some());

    // A callback's nominal leaves are identities as well.
    let parse = |callback: &str| {
        resolve_call(
            "json.parse",
            &["String".to_string(), callback.to_string()],
            true,
        )
    };
    assert!(parse("FUNC(String, json.Json) AS json.Json").is_some());
    assert_eq!(
        parse("FUNC(String, datetime.Instant) AS datetime.Instant"),
        None,
        "a callback over a different record must not satisfy a `Json` reviver"
    );
}

/// No registered descriptor may declare a [`ParameterType::Named`] whose name
/// *spells something with structure*: for every `Named(n)` in the catalog,
/// `parse(n)` must be that same `Named`.
///
/// The failure this catches is silent. `ParameterType::named("List OF String")`
/// is a `Named` whose name merely *spells* a container — a different value from
/// `list_of(String)`, but with the identical rendering. Every pre-plan-106
/// consumer rendered `.name()` and re-parsed, which normalized it on the way
/// through; the typed accessors plan-106-A introduced read the raw variant, and
/// `ir::lower` then inferred the element type of `fs::pathJoin([a, b])` as
/// `Unknown` rather than `String` (caught by the byte-identity gate on
/// `rt-behavior/project/project-fs-createTempFile-package-valid`, its one
/// `.ir` golden).
///
/// `named` is for a BARE NOMINAL — a record, union, or user type. Anything with
/// structure must be built with the matching constructor.
///
/// Deliberately scoped to `Named` and not "everything round-trips": a
/// [`Var`](ParameterType::Var) and an [`Arg`](ParameterType::Arg) render as a
/// bare name and re-parse as `Named` **by design** — `parse` classifies grammar,
/// and it cannot know a name is a type variable without the declaring scope.
/// Those are sanctioned; a structure-spelling `Named` is not.
#[test]
fn descriptor_named_types_are_bare_nominals() {
    fn check(type_: &ParameterType, where_: &str, failures: &mut Vec<String>) {
        if let ParameterType::Named(symbol) = type_ {
            let name = symbol.resolve();
            let reparsed = ParameterType::parse(name);
            if !matches!(reparsed, ParameterType::Named(_)) {
                failures.push(format!(
                    "{where_}: `{name}` is a bare `Named` but its own spelling parses to \
                         {reparsed:?} — build it with the matching constructor, not `named`"
                ));
            }
        }
        // Recurse so a bad leaf nested in a real container is caught too.
        match type_ {
            ParameterType::ListOf(inner)
            | ParameterType::SetOf(inner)
            | ParameterType::ResultOf(inner)
            | ParameterType::Res(inner) => check(inner, where_, failures),
            ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
                check(key, where_, failures);
                check(value, where_, failures);
            }
            ParameterType::UserOf(_, args) => {
                for arg in args {
                    check(arg, where_, failures);
                }
            }
            ParameterType::Func(params, ret, _) => {
                for param in params {
                    check(param, where_, failures);
                }
                check(ret, where_, failures);
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                check(msg, where_, failures);
                check(res, where_, failures);
                check(out, where_, failures);
            }
            _ => {}
        }
    }

    let mut failures = Vec::new();
    let mut checked = 0usize;
    for package in registry().packages() {
        for function in &package.functions {
            for (index, implementation) in function.implementations.iter().enumerate() {
                for param in &implementation.params {
                    let where_ = format!(
                        "{}.{} impl {index} param `{}`",
                        package.import_name, function.name, param.name
                    );
                    check(&param.ty, &where_, &mut failures);
                    checked += 1;
                    if let DefaultValue::Fill { type_name, .. } = &param.default {
                        let where_ = format!("{where_} (Fill default)");
                        check(type_name, &where_, &mut failures);
                        checked += 1;
                    }
                }
                let where_ = format!(
                    "{}.{} impl {index} return",
                    package.import_name, function.name
                );
                check(&implementation.return_type, &where_, &mut failures);
                checked += 1;
            }
        }
        for record in &package.records {
            for prop in &record.props {
                let where_ = format!(
                    "{}.{} field `{}`",
                    package.import_name, record.name, prop.name
                );
                check(&prop.ty, &where_, &mut failures);
                checked += 1;
            }
        }
    }
    assert!(
        checked > 0,
        "the registry exposed no descriptor types to check"
    );
    assert!(
        failures.is_empty(),
        "{} descriptor type(s) are not structurally what they spell:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// --- test builders (named-literal wrappers with throwaway docs) ---

fn func(name: &'static str, implementations: Vec<Implementation>) -> RegistryFunction {
    RegistryFunction {
        name,
        intro: "i",
        desc: "d",
        example: "e",
        expected_arguments: None,
        internal_only: false,
        implementations,
    }
}

fn prop(name: &'static str, ty: ParameterType) -> RecordProp {
    RecordProp {
        name,
        ty,
        description: "field doc",
    }
}

fn rec(name: &'static str, export: bool, props: Vec<RecordProp>) -> RegistryRecord {
    RegistryRecord {
        name,
        export,
        description: "",
        props,
    }
}

fn variant(name: &'static str) -> UnionVariant {
    UnionVariant {
        name,
        description: "variant doc",
    }
}

fn uni(name: &'static str, export: bool, variants: Vec<UnionVariant>) -> RegistryUnion {
    RegistryUnion {
        includes: vec![],
        name,
        export,
        variants,
    }
}

fn res(name: &'static str, export: bool, close_function: &'static str) -> RegistryResource {
    RegistryResource {
        name,
        export,
        description: "resource doc",
        close_function,
        sendable: true,
        live_slots: &[],
        unsendable_reason: None,
        close_may_fail: true,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    }
}

// A source-backed member built from its `FUNC __name(...)` body; the rewrite
// symbol is derived from the body's declared name.
fn mfb_impl(body: &'static str) -> Implementation {
    let rewrite = body
        .strip_prefix("FUNC ")
        .and_then(|rest| rest.split(['(', ' ']).next())
        .expect("test body starts with `FUNC <name>`");
    Implementation {
        params: vec![],
        return_type: ParameterType::String,
        errors: vec![],
        body: Body::mfb(body, rewrite),
    }
}

fn param(name: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty,
        default: DefaultValue::None,
    }
}

#[test]
fn package_constants_and_overrides_round_trip_through_the_builders() {
    // Scalar constant, record constant, and an override on one throwaway package.
    let mut pkg = RegistryPackage::new("demo", "i", "d");
    pkg.add_constant(RegistryConstant {
        name: "pi",
        type_name: "Float",
        value: Some("3.14159"),
        components: None,
        message: None,
        symbol: None,
    })
    .add_constant(RegistryConstant {
        name: "zero3",
        type_name: "Float3",
        value: None,
        components: Some(&["0.0", "0.0", "0.0"]),
        message: None,
        symbol: None,
    })
    .add_override(RegistryOverride {
        builtin: "toString",
        arg_type: "Float3",
        helper: "__demo_toString_float3",
    });

    // Accessors expose exactly what was added.
    assert_eq!(pkg.constants().len(), 2);
    assert_eq!(pkg.overrides().len(), 1);

    // The scalar constant answers `value` (not `components`).
    let scalar = pkg.constants().iter().find(|c| c.name == "pi").unwrap();
    assert_eq!(scalar.type_name, "Float");
    assert_eq!(scalar.value, Some("3.14159"));
    assert_eq!(scalar.components, None);

    // The record constant answers `components` (not `value`).
    let record = pkg.constants().iter().find(|c| c.name == "zero3").unwrap();
    assert_eq!(record.type_name, "Float3");
    assert_eq!(record.value, None);
    assert_eq!(record.components, Some(&["0.0", "0.0", "0.0"][..]));

    // The override is keyed by (builtin, arg_type).
    let ov = &pkg.overrides()[0];
    assert_eq!(
        (ov.builtin, ov.arg_type, ov.helper),
        ("toString", "Float3", "__demo_toString_float3")
    );
}

/// A call shape from concrete argument type names.
fn call(args: &[&str]) -> CallShape {
    CallShape {
        args: args.iter().map(|arg| ParameterType::parse(arg)).collect(),
    }
}

// Concise `ParameterType` constructors for the generic-resolution tests.
fn list_of(elem: ParameterType) -> ParameterType {
    ParameterType::list_of(elem)
}
fn map_of(key: ParameterType, value: ParameterType) -> ParameterType {
    ParameterType::map_of(key, value)
}

/// A minimal generic overload: parameter types (with type variables) and a return
/// type, no docs/body/errors.
fn generic_impl(params: Vec<ParameterType>, return_type: ParameterType) -> Implementation {
    Implementation {
        params: params.into_iter().map(|ty| param("x", ty)).collect(),
        return_type,
        errors: vec![],
        body: Body::Intrinsic,
    }
}

// --- thread ThreadHandle probe: empirical reproduction of the legacy resolver ---
//
// (`src/builtins/thread.rs:719-1006`) against the new `parse`/`unify`/`substitute`.
// These assert the LEGACY behavior and are the regression guard for the settled
// two-overload model (plan §"PART B obstacle resolution"): the resource plane is a
// SIGNATURE-LEVEL overload split on the existing `RES` spelling (`start` = a
// resource overload tried first + a data overload; `accept`/`transfer` =
// resource-only), the strict-`Nothing` guard rejects a data-only handle from the
// resource overload FOR us, and the `Var`-refines-`Unknown` rule (bug-443 latent
// fix) makes `send(Thread OF Unknown TO Out, Integer)` resolve.
fn th(worker: bool, msg: ParameterType, res: ParameterType, out: ParameterType) -> ParameterType {
    ParameterType::thread_handle(worker, msg, res, out)
}
