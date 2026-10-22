use super::*;

mod more;

/// bug-844: a view's own `STATE` clause never changes which side the view is.
/// A stateful worker answered "parent", so its `send`/`receive`/`transfer`/
/// `accept` were routed to the parent's queues and every message was lost.
#[test]
fn a_stateful_thread_view_keeps_its_side() {
    let worker = ParameterType::parse("ThreadWorker OF String TO Integer STATE W");
    let parent = ParameterType::parse("Thread OF String TO Integer STATE P");
    assert!(matches!(worker, ParameterType::Stateful { .. }));
    assert!(is_worker_thread_handle(&worker));
    assert!(!is_worker_thread_handle(&parent));
    assert!(crate::codegen::engine::types::is_parent_thread_type(
        &parent
    ));
    assert!(!crate::codegen::engine::types::is_parent_thread_type(
        &worker
    ));
}

/// plan-113: every C ABI spelling round-trips `parse` → `name` byte-exact,
/// as a `ParameterType::C`. This is what keeps the `.ast`/`.ir` goldens and
/// the IR binary's LINK section unchanged — `name()` IS the wire codec.
#[test]
fn c_abi_types_round_trip_through_parameter_type() {
    for ctype in CAbiType::ALL {
        let spelling = ctype.name();
        assert_eq!(
            CAbiType::parse(spelling),
            Some(ctype),
            "`{spelling}` must parse back to itself"
        );
        let parsed = ParameterType::parse(spelling);
        assert_eq!(
            parsed,
            ParameterType::C(ctype),
            "`{spelling}` must parse to its C variant"
        );
        assert_eq!(
            parsed.name(),
            spelling,
            "`{spelling}` must render back byte-exact"
        );
        assert_eq!(parsed.c_abi(), Some(ctype));
    }
    // All 16 are distinct spellings, so the enum cannot alias two names.
    let mut spellings: Vec<&str> = CAbiType::ALL.iter().map(|c| c.name()).collect();
    spellings.sort_unstable();
    let count = spellings.len();
    spellings.dedup();
    assert_eq!(spellings.len(), count, "two CAbiType variants share a name");
}

/// A `CSTRUCT` name is NOT a `C` — that namespace is open (a slot may name
/// any `CSTRUCT` declared in the same `LINK` alias), so it must stay a
/// nominal. Six committed goldens carry one (`"ctype": "SfFileInfo"`).
#[test]
fn a_cstruct_name_stays_a_nominal() {
    for name in ["SfFileInfo", "TimeSpec", "CTm", "CTime"] {
        let parsed = ParameterType::parse(name);
        assert!(
            matches!(parsed, ParameterType::Named(_)),
            "`{name}` is a CSTRUCT name and must stay Named, got {parsed:?}"
        );
        assert_eq!(parsed.c_abi(), None);
        assert_eq!(parsed.name(), name);
    }
}

/// Near-misses are not C ABI types: the match is exact and case-sensitive.
#[test]
fn near_miss_spellings_are_not_c_abi_types() {
    for name in [
        "CPtrX",
        "cptr",
        "",
        "CInt",
        "CInt3",
        "CInt33",
        " CPtr",
        "CPtr ",
        "C",
        "CFloat32",
        "CIntPtr",
        "CSize",
        "List OF CPtr",
    ] {
        assert_eq!(
            ParameterType::parse(name).c_abi(),
            None,
            "`{name}` must not parse to a C ABI type"
        );
    }
    // A `List OF CPtr` decomposes, and only its ELEMENT is the C type.
    assert_eq!(
        ParameterType::parse("List OF CPtr"),
        ParameterType::list_of(ParameterType::C(CAbiType::Ptr))
    );
    assert_eq!(ParameterType::parse("List OF CPtr").name(), "List OF CPtr");
}

/// plan-113 audit finding: a C ABI spelling is a bare token, so a template
/// parameter may be named with one (`TYPE Box OF CPtr`) — measured to build
/// on the pre-plan-113 binary. `with_vars` must reclassify it to a `Var`
/// exactly as it did when the spelling was a `Named`; without this the
/// field `v AS CPtr` reports `SYMBOL_UNKNOWN_TYPE`.
#[test]
fn with_vars_reclassifies_a_c_abi_spelling_used_as_a_template_param() {
    let params = vec!["CPtr".to_string()];
    assert_eq!(
        ParameterType::parse("CPtr").with_vars(&params),
        ParameterType::var("CPtr")
    );
    // Nested in a container, and inside a type that is NOT a declared param.
    assert_eq!(
        ParameterType::parse("List OF CPtr").with_vars(&params),
        ParameterType::list_of(ParameterType::var("CPtr"))
    );
    assert_eq!(
        ParameterType::parse("CInt32").with_vars(&params),
        ParameterType::C(CAbiType::Int32)
    );
    // No declared params: an identity rebuild, C variants preserved.
    assert_eq!(
        ParameterType::parse("CInt32").with_vars(&[]),
        ParameterType::C(CAbiType::Int32)
    );
}

/// Every `"ctype"` string the committed golden corpus carries round-trips
/// byte-exactly through `ParameterType::parse`/`name`. This is the direct
/// proof of plan-113's "no golden churn" non-goal: the goldens are the wire
/// format, and `name()` is what re-renders them.
#[test]
fn every_golden_corpus_ctype_round_trips() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut seen: Vec<String> = Vec::new();
    collect_ctypes(&root, &mut seen);
    assert!(
        !seen.is_empty(),
        "no `\"ctype\"` strings found under {} — the corpus scan is broken, \
             not the corpus",
        root.display()
    );
    for spelling in &seen {
        assert_eq!(
            ParameterType::parse(spelling).name(),
            spelling.as_str(),
            "golden ctype `{spelling}` does not round-trip"
        );
    }
}

/// Recursively collect every `"ctype": "<value>"` value from the `.ast` and
/// `.ir` goldens under `dir`.
#[cfg(test)]
fn collect_ctypes(dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ctypes(&path, out);
            continue;
        }
        let extension = path.extension().and_then(|e| e.to_str());
        if !matches!(extension, Some("ast") | Some("ir")) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut rest = text.as_str();
        while let Some(at) = rest.find("\"ctype\": \"") {
            rest = &rest[at + "\"ctype\": \"".len()..];
            let Some(end) = rest.find('"') else { break };
            let value = &rest[..end];
            if !out.iter().any(|s| s == value) {
                out.push(value.to_string());
            }
            rest = &rest[end..];
        }
    }
}

/// bug-483: `is_builtin_named` answers the same for a builtin value type
/// whichever spelling the compiler is holding — the qualified id a member
/// SIGNATURE carries, and the bare leaf a record FIELD type and the injected
/// companion source still carry.
#[test]
fn is_builtin_named_accepts_both_spellings() {
    for spelling in ["Address", "net.Address"] {
        assert!(
            ParameterType::declared(spelling).is_builtin_named("net", "Address"),
            "`{spelling}` must answer to net's `Address`"
        );
    }
    // A different package's same-leaf type is NOT it. bug-481 was precisely
    // that a flat namespace let `http::Stream` and `process::Stream` collide;
    // this helper must not reintroduce the collision through the back door.
    assert!(
        !ParameterType::declared("http.Address").is_builtin_named("net", "Address"),
        "a leaf owned by another package must not match"
    );
    // Near-misses on the prefix boundary.
    assert!(!ParameterType::declared("subnet.Address").is_builtin_named("net", "Address"));
    assert!(!ParameterType::declared("netAddress").is_builtin_named("net", "Address"));
    assert!(!ParameterType::declared("Addressed").is_builtin_named("net", "Address"));
    // Not a nominal at all.
    assert!(!ParameterType::String.is_builtin_named("net", "Address"));
}

/// Round-trip a thread-type spelling through `parse` → `name` and assert it is
/// byte-identical — the core `ThreadHandle` guarantee (nested threads,
/// parenthesized groups, `RES … STATE …` planes all preserved).
fn round_trip(spelling: &str) {
    assert_eq!(
        ParameterType::parse(spelling).name(),
        spelling,
        "round-trip mismatch for `{spelling}`"
    );
}

/// plan-106-D: a grouped type name decomposes STRUCTURALLY at every level,
/// instead of surviving as a junk `Named`/`UserOf` that only round-tripped
/// because `name()` echoed the garbage back verbatim.
///
/// This is the deliberate exception to `parse`↔`name` byte-exactness: the
/// group is source-level parenthesization carrying no type information, and
/// it is the form every consumer already computed by calling
/// `strip_type_group` at its own position.
#[test]
fn a_grouped_type_name_decomposes_at_every_level() {
    assert_eq!(ParameterType::parse("(Integer)"), ParameterType::Integer);
    assert_eq!(
        ParameterType::parse("List OF (Map OF String TO Integer)"),
        ParameterType::list_of(ParameterType::map_of(
            ParameterType::String,
            ParameterType::Integer,
        ))
    );
    assert_eq!(
        ParameterType::parse("(((Integer)))"),
        ParameterType::Integer
    );
    // The group is normalized away, so the render is the bare spelling.
    assert_eq!(
        ParameterType::parse("List OF (Map OF String TO Integer)").name(),
        "List OF Map OF String TO Integer"
    );

    // A leading `(` that is NOT a whole-name group must not be peeled: the
    // depth check in `strip_type_group` is what keeps `(A) TO (B)` — and any
    // other shape whose first `(` closes before the end — intact.
    assert_eq!(
        ParameterType::parse("(A) TO (B)"),
        ParameterType::named("(A) TO (B)")
    );
    // A `FUNC(...)` type does not start with `(`, so the peel never sees it.
    assert_eq!(
        ParameterType::parse("FUNC(Integer) AS String").name(),
        "FUNC(Integer) AS String"
    );
}

#[test]
fn thread_handle_parses_into_variant() {
    assert_eq!(
        ParameterType::parse("Thread OF Integer TO String"),
        ParameterType::thread_handle(
            false,
            ParameterType::Integer,
            ParameterType::Nothing,
            ParameterType::String,
        )
    );
    assert_eq!(
        ParameterType::parse("ThreadWorker OF Integer TO String"),
        ParameterType::thread_handle(
            true,
            ParameterType::Integer,
            ParameterType::Nothing,
            ParameterType::String,
        )
    );
    assert_eq!(
        ParameterType::parse("Thread OF RES fs.File TO String"),
        ParameterType::thread_handle(
            false,
            ParameterType::Nothing,
            ParameterType::named("fs.File"),
            ParameterType::String,
        )
    );
    assert_eq!(
        ParameterType::parse("Thread"),
        ParameterType::named("Thread")
    );
    assert_eq!(
        ParameterType::parse("ThreadWorker"),
        ParameterType::named("ThreadWorker")
    );
}

#[test]
fn thread_handle_round_trips_byte_exact() {
    for spelling in [
        "Thread OF Integer TO String",
        "ThreadWorker OF Integer TO String",
        "Thread OF Integer RES fs.File TO String",
        "Thread OF RES fs.File TO String",
        "Thread OF RES fs.File STATE Cursor TO Integer",
        "Thread OF Integer RES fs.File STATE Cursor TO String",
        "Thread OF List OF Integer TO Map OF String TO Integer",
        "Thread OF Thread OF Integer TO String TO Boolean",
        "Thread OF Thread OF Integer RES fs.File TO String TO Boolean",
        "Thread OF Unknown TO String",
        "Thread OF Nothing TO String",
        // bug-463: the crossing cases — a message plane whose type has to be
        // MEASURED (`type_prefix_len`) past a `RES` marker, a `Set`, a
        // `FUNC(...)` or a user generic. Byte-exactness alone does NOT guard
        // these: with the bug present every one of them still round-tripped,
        // because the whole spelling collapsed to a `Named` and `Named(s).name()`
        // echoes `s` back verbatim. The tests that actually see it are
        // `res_collection_on_thread_plane_parses_structurally` and
        // `type_prefix_len_measures_every_parse_constructor`.
        "Thread OF List OF RES fs.File TO Integer",
        "Thread OF Map OF String TO RES fs.File TO Integer",
        "ThreadWorker OF List OF RES fs.File TO Integer",
        "Thread OF List OF RES fs.File STATE Cursor TO Integer",
        "Thread OF List OF RES fs.File RES fs.File TO Integer",
        "Thread OF Map OF RES fs.File TO Integer TO Integer",
        "Thread OF Set OF Integer TO Integer",
        "Thread OF Set OF RES fs.File TO Integer",
        "Thread OF FUNC(Integer) AS String TO Integer",
        "Thread OF ISOLATED FUNC(Integer) AS String TO Integer",
        "Thread OF Pair OF (Integer, String) TO Integer",
        "Thread OF List OF Set OF Integer TO Integer",
        "Thread OF Map OF Set OF Integer TO Integer TO Integer",
    ] {
        round_trip(spelling);
    }
}

#[test]
fn isolated_func_round_trips_and_decomposes() {
    // `thread::start`'s worker callback: the `ISOLATED ` marker round-trips, and the
    // nested `ThreadWorker` handle is reachable through the `Func` params.
    let spelling =
        "ISOLATED FUNC(ThreadWorker OF Integer RES fs.File TO String, Integer) AS String";
    assert_eq!(ParameterType::parse(spelling).name(), spelling);
    assert_eq!(
        ParameterType::parse("FUNC(Integer) AS String").name(),
        "FUNC(Integer) AS String"
    );
    match ParameterType::parse(spelling) {
        ParameterType::Func(params, ret, isolated) => {
            assert!(isolated);
            assert_eq!(ret.name(), "String");
            assert_eq!(
                params[0],
                ParameterType::thread_handle(
                    true,
                    ParameterType::Integer,
                    ParameterType::named("fs.File"),
                    ParameterType::String,
                )
            );
        }
        other => panic!("expected Func, got {other:?}"),
    }
}

#[test]
fn map_entry_and_result_parse_into_variants_and_round_trip() {
    // The two shapes `monomorph::helpers::unify_type` handled on strings but `parse`
    // previously folded into `Named`. They now decompose structurally and round-trip
    // byte-exact.
    assert_eq!(
        ParameterType::parse("MapEntry OF String TO Integer"),
        ParameterType::map_entry_of(ParameterType::String, ParameterType::Integer),
    );
    assert_eq!(
        ParameterType::parse("#Result OF Nothing"),
        ParameterType::result_of(ParameterType::Nothing),
    );
    assert_eq!(
        ParameterType::parse("#Result OF List OF Integer"),
        ParameterType::result_of(ParameterType::list_of(ParameterType::Integer)),
    );
    for spelling in [
        "MapEntry OF String TO Integer",
        "MapEntry OF String TO List OF Integer",
        "#Result OF Nothing",
        "#Result OF Integer",
        "#Result OF List OF String",
        "List OF MapEntry OF String TO Integer",
    ] {
        round_trip(spelling);
    }
}

#[test]
fn res_collection_element_parses_into_variant() {
    // A `RES`-marked collection element becomes a `Res` wrapping the inner type
    // (historically the marker was stripped, losing it on a `name()` round-trip).
    assert_eq!(
        ParameterType::parse("RES fs.File"),
        ParameterType::res(ParameterType::named("fs.File"))
    );
    assert_eq!(
        ParameterType::parse("List OF RES fs.File"),
        ParameterType::list_of(ParameterType::res(ParameterType::named("fs.File")))
    );
    assert_eq!(
        ParameterType::parse("Map OF String TO RES fs.File"),
        ParameterType::map_of(
            ParameterType::String,
            ParameterType::res(ParameterType::named("fs.File")),
        )
    );
    // The `STATE` clause stays on the ELEMENT and the `RES` wraps it — the
    // contract this line has always asserted. plan-111-A gave that element a
    // variant, so it is a `Stateful` rather than an opaque nominal; the
    // clause's *position* is what matters here and it has not moved.
    // `stateful_round_trips_byte_exact` pins the rendering, and
    // `structural_split_state_agrees_with_the_string_grammar` pins that the
    // container still reports no top-level clause of its own (bug-429).
    assert_eq!(
        ParameterType::parse("List OF RES File STATE Cursor"),
        ParameterType::list_of(ParameterType::res(ParameterType::Stateful {
            base: Box::new(ParameterType::named("File")),
            state: Box::new(ParameterType::named("Cursor")),
        }))
    );
}

/// plan-106-A: `with_state` must be *exactly* the structural equivalent of
/// parsing the concatenated spelling — that identity is what lets `ir::lower`
/// build a stateful resource type without a render→parse round trip.
#[test]
fn with_state_matches_parse_of_the_concatenated_spelling() {
    let states = [
        ParameterType::named("Cursor"),
        ParameterType::Integer,
        ParameterType::named("pkg.FileInfo"),
    ];
    let bases = [
        // Leaves: nominal, qualified nominal, every scalar shape, and the
        // non-nominal leaves that must still fold into one opaque `Named`.
        ParameterType::named("File"),
        ParameterType::named("fs.File"),
        ParameterType::Integer,
        ParameterType::String,
        ParameterType::Boolean,
        ParameterType::Nothing,
        ParameterType::Unknown,
        ParameterType::var("T"),
        // Containers: the clause lands on the element / the VALUE half.
        ParameterType::list_of(ParameterType::named("File")),
        ParameterType::set_of(ParameterType::named("File")),
        ParameterType::result_of(ParameterType::named("File")),
        ParameterType::Res(Box::new(ParameterType::named("File"))),
        ParameterType::list_of(ParameterType::Res(Box::new(ParameterType::named("File")))),
        ParameterType::map_of(ParameterType::String, ParameterType::named("File")),
        ParameterType::map_entry_of(ParameterType::String, ParameterType::named("File")),
        ParameterType::list_of(ParameterType::list_of(ParameterType::named("File"))),
        // Func / thread handle / user generic: the return, the `out` plane,
        // and the final type argument respectively.
        ParameterType::Func(
            vec![ParameterType::Integer],
            Box::new(ParameterType::named("File")),
            false,
        ),
        ParameterType::Func(
            vec![ParameterType::Integer],
            Box::new(ParameterType::named("File")),
            true,
        ),
        ParameterType::ThreadHandle {
            worker: false,
            msg: Box::new(ParameterType::Integer),
            res: Box::new(ParameterType::Nothing),
            out: Box::new(ParameterType::named("File")),
        },
        ParameterType::user_of("Pair", vec![ParameterType::Integer, ParameterType::String]),
    ];
    let mut checked = 0usize;
    for base in &bases {
        for state in &states {
            let spelling = format!("{} STATE {}", base.name(), state.name());
            assert_eq!(
                base.with_state(state),
                ParameterType::parse(&spelling),
                "with_state diverged from parse for `{spelling}`"
            );
            // …and the result still renders back to that same spelling.
            assert_eq!(base.with_state(state).name(), spelling);
            checked += 1;
        }
    }
    assert_eq!(checked, bases.len() * states.len());
}

/// plan-106-C: on a LEAF base — every place a resource's STATE is actually
/// read — `split_state` undoes `with_state` exactly.
#[test]
fn split_state_is_the_inverse_of_with_state_on_leaves() {
    let states = [
        ParameterType::named("Cursor"),
        ParameterType::Integer,
        ParameterType::named("pkg.FileInfo"),
        // A STRUCTURED state — `fs.File STATE List OF Choice` is legal, and
        // wrapping it with `named` instead of parsing it is the exact bug
        // plan-106-B Correction 4 fixed.
        ParameterType::list_of(ParameterType::named("Choice")),
    ];
    let leaves = [
        ParameterType::named("File"),
        ParameterType::named("fs.File"),
        ParameterType::Integer,
        ParameterType::String,
        ParameterType::Unknown,
    ];
    let mut checked = 0usize;
    for base in &leaves {
        assert_eq!(
            base.split_state(),
            (base.clone(), None),
            "stateless leaf reported a STATE: {base:?}"
        );
        for state in &states {
            let attached = base.with_state(state);
            assert_eq!(
                attached.split_state(),
                (base.clone(), Some(state.clone())),
                "split_state did not invert with_state for `{}` STATE `{}`",
                base.name(),
                state.name()
            );
            assert_eq!(attached.without_state(), *base);
            assert_eq!(attached.state().as_ref(), Some(state));
            checked += 1;
        }
    }
    assert_eq!(checked, leaves.len() * states.len());

    // A `Var` is the one leaf that cannot round-trip, for the same sanctioned
    // reason the descriptor guard records: a type variable renders as its bare
    // name and `parse` cannot know a name is a variable without the declaring
    // scope, so it comes back a `Named`. Harmless — a STATE is attached to a
    // concrete resource, never to a type variable — but asserted so the limit
    // is explicit rather than discovered later.
    let var_with_state = ParameterType::var("T").with_state(&ParameterType::named("Cursor"));
    assert_eq!(
        var_with_state.split_state(),
        (
            ParameterType::named("T"),
            Some(ParameterType::named("Cursor"))
        )
    );
}

/// `split_state` is **top-level only**, and this pins that it does NOT
/// descend — the property bug-429 depends on.
///
/// `with_state` attaches to the innermost rendered child, so on a composite
/// the two are deliberately not inverses: `List OF RES File STATE Cursor`
/// carries its clause on the *element*, and the top-level split must report
/// `None` for it. Peeling it here would strip one side of a comparison while
/// the other has nothing to strip, which is precisely the asymmetry that
/// rejected a correct STATE-carrying resource union (bug-429) — and, when
/// this accessor was first written descending, broke
/// `bug427_list_union_state_rt` and `bug429_owned_list_union_drain_rt` with
/// "expected List OF RES Handle, got List OF RES Handle STATE Cursor".
#[test]
fn split_state_is_top_level_only() {
    for spelling in [
        "List OF RES File STATE Cursor",
        "Set OF RES File STATE Cursor",
        "#Result OF File STATE Cursor",
        "Map OF String TO RES File STATE Cursor",
        "RES File STATE Cursor",
    ] {
        let type_ = ParameterType::parse(spelling);
        assert_eq!(
            type_.split_state(),
            (type_.clone(), None),
            "`{spelling}` must not split — its STATE belongs to a nested type"
        );
        // Exactly what the name-domain helper this replaces answers.
        assert_eq!(crate::codegen::resource::state_type_name(spelling), None);
    }
}

/// `split_state` must agree with the name-domain helpers it replaces
/// (`codegen::resource::base_resource_name` / `state_type_name`) on every
/// spelling, including the composites above and the thread planes whose
/// ` STATE ` belongs to the plane, not the handle (plan-54).
#[test]
fn split_state_matches_the_name_domain_helpers() {
    for spelling in [
        "File",
        "File STATE Cursor",
        "fs.File STATE Cursor",
        "Integer",
        "RES File STATE Cursor",
        "List OF RES File STATE Cursor",
        "Map OF String TO RES File STATE Cursor",
        "Thread OF RES fs.File STATE Cursor TO Integer",
        "Thread OF Integer RES fs.File STATE Cursor TO String",
    ] {
        let type_ = ParameterType::parse(spelling);
        let (base, state) = type_.split_state();
        assert_eq!(
            state.as_ref().map(|s| s.name()).as_deref(),
            crate::codegen::resource::state_type_name(spelling),
            "STATE disagreement on `{spelling}`"
        );
        assert_eq!(
            base.name(),
            crate::codegen::resource::base_resource_name(spelling),
            "base disagreement on `{spelling}`"
        );
        assert_eq!(type_.name(), spelling);
    }
}

/// plan-111-A Phase 2: from letter C onward a `ParameterType` is a map
/// *key*, so `hash` must agree with `eq` — the `HashMap`/`HashSet`
/// contract. The derive gives that by construction; this pins it, and pins
/// that variants sharing a payload shape (`ListOf`/`SetOf`,
/// `MapOf`/`MapEntryOf`, `Named`/`Var`, parent/worker `ThreadHandle`) stay
/// *distinct* keys — a collision there would silently merge two types'
/// entries in exactly the tables letter C re-keys.
#[test]
fn parameter_type_hash_agrees_with_eq() {
    use std::collections::hash_map::DefaultHasher;
    use std::collections::HashSet;
    use std::hash::{Hash, Hasher};

    fn digest(t: &ParameterType) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    // One spelling per parseable variant, drawn from the round-trip corpora
    // above (`thread_handle_round_trips_byte_exact`,
    // `map_entry_and_result_parse_into_variants_and_round_trip`,
    // `res_collection_round_trips_byte_exact`).
    const CORPUS: &[&str] = &[
        "AttributeString",
        "Boolean",
        "Byte",
        "Integer",
        "Fixed",
        "Float",
        "Money",
        "Nothing",
        "String",
        "Unknown",
        "List OF Integer",
        "Set OF Integer",
        "Map OF String TO Integer",
        "MapEntry OF String TO Integer",
        "#Result OF Integer",
        "#Result OF List OF String",
        "RES fs.File",
        "RES File STATE Cursor",
        "List OF RES fs.File",
        "List OF RES File STATE Cursor",
        "Map OF RES fs.File TO Integer",
        "CsvReader",
        "Pair OF (Integer, String)",
        "FUNC(Integer) AS String",
        "ISOLATED FUNC(Integer) AS String",
        "Thread OF Integer TO String",
        "ThreadWorker OF Integer TO String",
        "Thread OF Integer RES fs.File STATE Cursor TO String",
        "List OF List OF RES fs.File",
    ];

    // 1. Equal types hash equal. Two independent parses of one spelling are
    //    `eq`, so they must be indistinguishable as keys.
    for spelling in CORPUS {
        let a = ParameterType::parse(spelling);
        let b = ParameterType::parse(spelling);
        assert_eq!(a, b, "parse is not deterministic for `{spelling}`");
        assert_eq!(
            digest(&a),
            digest(&b),
            "hash disagrees with eq for `{spelling}`"
        );
    }

    // 2. Distinct types are distinct keys, across every container variant.
    let mut set: HashSet<ParameterType> = HashSet::new();
    for spelling in CORPUS {
        assert!(
            set.insert(ParameterType::parse(spelling)),
            "`{spelling}` is not a distinct key — it collided with an \
                 earlier corpus entry"
        );
    }
    assert_eq!(set.len(), CORPUS.len());

    // 3. A key built independently still finds its entry — the property
    //    every re-keyed table in letter C relies on.
    for spelling in CORPUS {
        assert!(
            set.contains(&ParameterType::parse(spelling)),
            "lookup missed for `{spelling}`"
        );
    }

    // 4. Same payload, different variant: these must NOT be one key.
    //    `Var`/`Arg` are never produced by `parse`, so they are built here.
    let same_payload_distinct = [
        ParameterType::list_of(ParameterType::Integer),
        ParameterType::set_of(ParameterType::Integer),
        ParameterType::result_of(ParameterType::Integer),
        ParameterType::map_of(ParameterType::String, ParameterType::Integer),
        ParameterType::map_entry_of(ParameterType::String, ParameterType::Integer),
        ParameterType::named("T"),
        ParameterType::var("T"),
        ParameterType::Arg(0),
        ParameterType::Arg(1),
        ParameterType::func(vec![ParameterType::Integer], ParameterType::String),
        ParameterType::func_isolated(vec![ParameterType::Integer], ParameterType::String),
        ParameterType::thread_handle(
            false,
            ParameterType::Integer,
            ParameterType::Nothing,
            ParameterType::String,
        ),
        ParameterType::thread_handle(
            true,
            ParameterType::Integer,
            ParameterType::Nothing,
            ParameterType::String,
        ),
    ];
    let mut lookalikes: HashSet<ParameterType> = HashSet::new();
    for t in &same_payload_distinct {
        assert!(
            lookalikes.insert(t.clone()),
            "`{}` collided with a same-payload lookalike",
            t.name()
        );
    }
    assert_eq!(lookalikes.len(), same_payload_distinct.len());
}

/// plan-111-A Phase 3: `parse` builds a [`ParameterType::Stateful`] for a
/// resource's OWN top-level ` STATE T`, and for nothing else. These are the
/// nested-plane cases the deleted `split_state_clause` copies protected with
/// their `base.contains(' ')` guard; the guard did not go away, it moved
/// into `parse`.
#[test]
fn stateful_parses_top_level_only() {
    // The top-level case: a bare resource with its own clause.
    assert_eq!(
        ParameterType::parse("File STATE Cursor"),
        ParameterType::Stateful {
            base: Box::new(ParameterType::named("File")),
            state: Box::new(ParameterType::named("Cursor")),
        }
    );
    // A qualified base is still one token.
    assert_eq!(
        ParameterType::parse("fs.File STATE Cursor"),
        ParameterType::Stateful {
            base: Box::new(ParameterType::named("fs.File")),
            state: Box::new(ParameterType::named("Cursor")),
        }
    );
    // A composite STATE payload decomposes structurally too.
    assert_eq!(
        ParameterType::parse("File STATE List OF Integer"),
        ParameterType::Stateful {
            base: Box::new(ParameterType::named("File")),
            state: Box::new(ParameterType::list_of(ParameterType::Integer)),
        }
    );

    // Nested: the clause lands on the ELEMENT / the plane, never on the
    // container. Splitting these at the composite is the truncation bug-429
    // fixed (`ThreadWorker OF RES File STATE Cursor TO Integer` became
    // `ThreadWorker OF RES File`).
    let stateful_file = ParameterType::Stateful {
        base: Box::new(ParameterType::named("File")),
        state: Box::new(ParameterType::named("Cursor")),
    };
    assert_eq!(
        ParameterType::parse("RES File STATE Cursor"),
        ParameterType::res(stateful_file.clone())
    );
    assert_eq!(
        ParameterType::parse("List OF RES File STATE Cursor"),
        ParameterType::list_of(ParameterType::res(stateful_file.clone()))
    );
    assert_eq!(
        ParameterType::parse("Set OF RES File STATE Cursor"),
        ParameterType::set_of(ParameterType::res(stateful_file.clone()))
    );
    assert_eq!(
        ParameterType::parse("Map OF String TO RES File STATE Cursor"),
        ParameterType::map_of(
            ParameterType::String,
            ParameterType::res(stateful_file.clone())
        )
    );
    assert_eq!(
        ParameterType::parse("#Result OF Stream STATE Pending"),
        ParameterType::result_of(ParameterType::Stateful {
            base: Box::new(ParameterType::named("Stream")),
            state: Box::new(ParameterType::named("Pending")),
        })
    );
    assert_eq!(
        ParameterType::parse("Thread OF RES fs.File STATE Cursor TO Integer"),
        ParameterType::thread_handle(
            false,
            ParameterType::Nothing,
            // A thread's resource plane stores the inner type directly;
            // `name()` re-adds the `RES ` marker when it renders the plane.
            ParameterType::Stateful {
                base: Box::new(ParameterType::named("fs.File")),
                state: Box::new(ParameterType::named("Cursor")),
            },
            ParameterType::Integer,
        )
    );
    // A user generic carries the clause on its LAST argument, which is where
    // the trailing text abuts (`with_state`'s `UserOf` arm agrees).
    assert_eq!(
        ParameterType::parse("Pair OF (Integer, String) STATE Cursor"),
        ParameterType::user_of(
            "Pair",
            vec![
                ParameterType::Integer,
                ParameterType::Stateful {
                    base: Box::new(ParameterType::String),
                    state: Box::new(ParameterType::named("Cursor")),
                },
            ]
        )
    );

    // A base that is itself a phrase is NOT this type's own clause, so the
    // whole spelling stays one opaque nominal — byte-for-byte what the
    // pre-variant `parse` did with it.
    assert_eq!(
        ParameterType::parse("A B STATE C"),
        ParameterType::named("A B STATE C")
    );
    assert_eq!(ParameterType::parse("A B STATE C").split_state().1, None);
}

/// The structural [`ParameterType::split_state`] must answer exactly what
/// the string rule answered, for every shape — that equivalence is the
/// entire safety argument for replacing the render-and-re-split
/// implementation with a variant match.
#[test]
fn structural_split_state_agrees_with_the_string_grammar() {
    for spelling in [
        "File",
        "Integer",
        "File STATE Cursor",
        "fs.File STATE Cursor",
        "File STATE List OF Integer",
        "RES File STATE Cursor",
        "List OF RES File STATE Cursor",
        "Set OF RES File STATE Cursor",
        "Map OF String TO RES File STATE Cursor",
        "#Result OF Stream STATE Pending",
        "Thread OF RES fs.File STATE Cursor TO Integer",
        "ThreadWorker OF RES File STATE Cursor TO Integer",
        "Thread OF Integer RES fs.File STATE Cursor TO String",
        "Pair OF (Integer, String) STATE Cursor",
        "A B STATE C",
    ] {
        let type_ = ParameterType::parse(spelling);
        let (base, state) = type_.split_state();
        // The string rule, applied to the same spelling.
        let expected = split_state_clause(spelling);
        assert_eq!(
            state.as_ref().map(|s| s.name().into_owned()),
            expected.map(|(_, s)| s.to_string()),
            "STATE disagreement on `{spelling}`"
        );
        assert_eq!(
            base.name(),
            expected.map_or(spelling, |(b, _)| b),
            "base disagreement on `{spelling}`"
        );
    }
}

/// plan-111-A Phase 3: every stateful spelling in the tree survives
/// `parse` → `name` byte-exact. The variant renders what the opaque `Named`
/// rendered, which is what keeps the `.mfp` type table, the IR encoding and
/// every type-naming diagnostic unchanged (plan-111-A §Compatibility).
#[test]
fn stateful_round_trips_byte_exact() {
    for spelling in [
        "File STATE Cursor",
        "fs.File STATE Cursor",
        "pkg.Name STATE S",
        "File STATE List OF Integer",
        "RES File STATE Cursor",
        "List OF RES File STATE Cursor",
        "Set OF RES File STATE Cursor",
        "Map OF String TO RES File STATE Cursor",
        "List OF List OF RES File STATE Cursor",
        "#Result OF Stream STATE Pending",
        "Thread OF RES fs.File STATE Cursor TO Integer",
        "ThreadWorker OF RES File STATE Cursor TO Integer",
        "Thread OF Integer RES fs.File STATE Cursor TO String",
        "Pair OF (Integer, String) STATE Cursor",
        "FUNC(Integer) AS File STATE Cursor",
        // Not a top-level clause: still one opaque nominal, and it must
        // round-trip as one.
        "A B STATE C",
    ] {
        round_trip(spelling);
    }
}

/// plan-111-B: `contains_state` is defined as
/// `name().contains(" STATE ")` and computed structurally. Pin the equality
/// over every shape, including the ones where the clause is NESTED (which is
/// where a naive `matches!(t, Stateful { .. })` would silently disagree) and
/// the composite-base spelling `parse` declines to split.
#[test]
fn contains_state_agrees_with_the_rendered_name() {
    let mut checked = 0usize;
    for spelling in [
        "File",
        "Integer",
        "Unknown",
        "File STATE Cursor",
        "fs.File STATE Cursor",
        "File STATE List OF Integer",
        "RES File STATE Cursor",
        "List OF RES File STATE Cursor",
        "Set OF RES File STATE Cursor",
        "Map OF String TO RES File STATE Cursor",
        "Map OF RES File STATE Cursor TO Integer",
        "MapEntry OF String TO File STATE Cursor",
        "List OF List OF RES File STATE Cursor",
        "#Result OF Stream STATE Pending",
        "Thread OF RES fs.File STATE Cursor TO Integer",
        "ThreadWorker OF RES File STATE Cursor TO Integer",
        "Thread OF Integer RES fs.File STATE Cursor TO String",
        "Pair OF (Integer, String) STATE Cursor",
        "FUNC(Integer) AS File STATE Cursor",
        "FUNC(File STATE Cursor) AS Integer",
        // `parse` declines to split a composite base, so this stays one
        // opaque `Named` — and must still read as stateful.
        "A B STATE C",
        "List OF A B STATE C",
        // Negatives that merely look close.
        "StateMachine",
        "List OF StateMachine",
        "Thread OF Integer TO String",
    ] {
        let type_ = ParameterType::parse(spelling);
        assert_eq!(
            type_.contains_state(),
            type_.name().contains(" STATE "),
            "contains_state disagrees with the rendered name for `{spelling}`"
        );
        checked += 1;
    }
    assert_eq!(checked, 25);
    // Var and Arg are never produced by `parse`; neither can carry a clause.
    assert!(!ParameterType::var("T").contains_state());
    assert!(!ParameterType::Arg(0).contains_state());
    assert!(!ParameterType::Unknown.contains_state());
}

#[test]
fn res_collection_round_trips_byte_exact() {
    // The whole point of the `Res` variant: these spellings round-tripped LOSSILY
    // before (the `RES ` marker was dropped), which is why `resolve_call` had to
    // echo the raw argument string. Now `parse`→`name` is byte-exact.
    for spelling in [
        "RES fs.File",
        "RES File STATE Cursor",
        "List OF RES fs.File",
        "List OF RES File STATE Cursor",
        "Set OF RES fs.File",
        "Map OF String TO RES fs.File",
        "Map OF RES fs.File TO Integer",
        "List OF List OF RES fs.File",
        // bug-463: a `RES` collection nested inside a thread plane. Same caveat
        // as in `thread_handle_round_trips_byte_exact` — round-tripping is
        // necessary but NOT sufficient; these passed with the bug present.
        "Thread OF List OF RES fs.File TO Integer",
        "Map OF Thread OF List OF RES fs.File TO Integer TO Integer",
    ] {
        round_trip(spelling);
    }
}

/// bug-463: a `RES`-marked collection on a thread's MESSAGE plane must decompose
/// structurally, exactly as it already does on the OUTPUT plane.
///
/// The output plane is not measured — `split_thread_types` takes "everything after
/// the last ` TO `" — but the message plane must be measured to find where it ends,
/// and `type_prefix_len` had no `RES ` arm. It therefore stopped inside the element
/// (`List OF RES` = 11 bytes), the ` TO ` split failed, and the ENTIRE spelling
/// survived as one opaque `Named` through the whole front end: the
/// thread-sendability rule that governs the program (`ir::verify::resources`,
/// `ParameterType::Res(_) => false`) never ran, and the resolver rejected the junk
/// name for an unrelated reason instead.
///
/// This asserts the parsed VARIANT, not the round trip, because a round trip cannot
/// see this class of bug at all: `Named(s).name() == s` echoes the garbage back.
#[test]
fn res_collection_on_thread_plane_parses_structurally() {
    for spelling in [
        "Thread OF List OF RES fs.File TO Integer",
        "Thread OF Map OF String TO RES fs.File TO Integer",
        "ThreadWorker OF List OF RES fs.File TO Integer",
        "Thread OF List OF RES fs.File STATE Cursor TO Integer",
        "Thread OF List OF RES fs.File RES fs.File TO Integer",
        "Thread OF Map OF RES fs.File TO Integer TO Integer",
        "Thread OF Set OF RES fs.File TO Integer",
        // The contrast cases, which already worked: they must keep working.
        "Thread OF List OF Integer TO Integer",
        "Thread OF Integer RES fs.File TO Integer",
        "Thread OF Integer TO List OF RES fs.File",
    ] {
        assert!(
            matches!(
                ParameterType::parse(spelling),
                ParameterType::ThreadHandle { .. }
            ),
            "`{spelling}` did not parse into ThreadHandle; got {:?}",
            ParameterType::parse(spelling),
        );
    }
    // The message plane is intact, not merely non-`Named`.
    assert_eq!(
        ParameterType::parse("Thread OF List OF RES fs.File TO Integer"),
        ParameterType::thread_handle(
            false,
            ParameterType::list_of(ParameterType::res(ParameterType::named("fs.File"))),
            ParameterType::Nothing,
            ParameterType::Integer,
        ),
    );
    // A nested thread type inside a `Map` key is measured by the same helper.
    assert_eq!(
        ParameterType::parse("Map OF Thread OF List OF RES fs.File TO Integer TO Integer"),
        ParameterType::map_of(
            ParameterType::thread_handle(
                false,
                ParameterType::list_of(ParameterType::res(ParameterType::named("fs.File"))),
                ParameterType::Nothing,
                ParameterType::Integer,
            ),
            ParameterType::Integer,
        ),
    );
}

/// bug-463 (widened): `type_prefix_len` must measure every constructor
/// [`ParameterType::parse`] can decompose. It is the ONLY thing standing between a
/// nested position — a thread message plane, a `Map` key, a generic argument — and
/// a silent collapse to an opaque `Named`.
///
/// Beyond the `RES` marker the bug report documented, the measurement was also
/// missing `Set`, `FUNC(...)`/`ISOLATED FUNC(...)` and user generics. Those gaps
/// were worse than the `RES` one: `Thread OF Set OF Integer TO Integer` is a
/// perfectly sendable, VALID program, and it was rejected outright with
/// `SYMBOL_UNKNOWN_TYPE`.
///
/// The property asserted is the one that makes a measurement correct: a spelling
/// that IS one complete type measures to its own full length.
#[test]
fn type_prefix_len_measures_every_parse_constructor() {
    for spelling in [
        // Leaves and the `(T)` group.
        "Integer",
        "fs.File",
        "(Integer)",
        "(Map OF String TO Integer)",
        // The `RES` ownership marker, at top level and nested, with `STATE`.
        "RES fs.File",
        "RES File STATE Cursor",
        "List OF RES fs.File",
        "List OF RES File STATE Cursor",
        "Set OF RES fs.File",
        "Map OF String TO RES fs.File",
        "Map OF RES fs.File TO Integer",
        "#Result OF RES fs.File",
        "MapEntry OF String TO RES fs.File",
        // `Set`, which the measurement never descended into at all.
        "Set OF Integer",
        "Set OF List OF Integer",
        "List OF Set OF Integer",
        "Map OF Set OF Integer TO Integer",
        // Function types.
        "FUNC(Integer) AS String",
        "FUNC() AS Nothing",
        "FUNC(FUNC(Integer) AS String, Integer) AS Boolean",
        "ISOLATED FUNC(Integer) AS String",
        "List OF FUNC(Integer) AS String",
        // User generics, including a nested one and a generic argument list.
        "Pair OF (Integer, String)",
        "Holder OF Integer",
        "Holder OF Pair OF (Integer, String)",
        "Pair OF (Integer, Map OF String TO Integer)",
        "List OF Pair OF (Integer, String)",
        // The builtin containers and threads, which already worked.
        "List OF Integer",
        "Map OF String TO Integer",
        "MapEntry OF String TO Integer",
        "#Result OF Integer",
        "Thread OF Integer TO String",
        "Thread OF RES fs.File STATE Cursor TO Integer",
        "Thread OF List OF RES fs.File TO Integer",
        "ThreadWorker OF Set OF Integer RES fs.File TO Integer",
    ] {
        assert_eq!(
            type_prefix_len(spelling),
            Some(spelling.len()),
            "`{spelling}` is one complete type but did not measure to its own length"
        );
    }

    // A measured prefix stops exactly where the type ends — the property the thread
    // message plane and the `Map` key depend on. Each pair is (input, its prefix).
    for (input, prefix) in [
        ("List OF RES fs.File TO Integer", "List OF RES fs.File"),
        ("Set OF Integer TO Integer", "Set OF Integer"),
        (
            "FUNC(Integer) AS String TO Integer",
            "FUNC(Integer) AS String",
        ),
        (
            "Pair OF (Integer, String) TO Integer",
            "Pair OF (Integer, String)",
        ),
        ("RES File STATE Cursor TO Integer", "RES File STATE Cursor"),
        (
            "Map OF RES fs.File TO Integer TO Integer",
            "Map OF RES fs.File TO Integer",
        ),
    ] {
        assert_eq!(
            type_prefix_len(input),
            Some(prefix.len()),
            "measuring `{input}` should stop after `{prefix}`"
        );
    }
}

/// bug-463: the thread message plane decomposes for every constructor, so the rule
/// that governs the program is the one that gets to reject (or accept) it — not an
/// accident of spelling in the resolver.
#[test]
fn thread_message_plane_measures_every_parse_constructor() {
    for spelling in [
        "Thread OF Set OF Integer TO Integer",
        "Thread OF List OF Set OF Integer TO Integer",
        "Thread OF Map OF Set OF Integer TO Integer TO Integer",
        "Thread OF FUNC(Integer) AS String TO Integer",
        "Thread OF ISOLATED FUNC(Integer) AS String TO Integer",
        "Thread OF Pair OF (Integer, String) TO Integer",
    ] {
        assert!(
            matches!(
                ParameterType::parse(spelling),
                ParameterType::ThreadHandle { .. }
            ),
            "`{spelling}` did not parse into ThreadHandle; got {:?}",
            ParameterType::parse(spelling),
        );
    }
    assert_eq!(
        ParameterType::parse("Thread OF Set OF Integer TO Integer"),
        ParameterType::thread_handle(
            false,
            ParameterType::set_of(ParameterType::Integer),
            ParameterType::Nothing,
            ParameterType::Integer,
        ),
    );
    assert_eq!(
        ParameterType::parse("Thread OF FUNC(Integer) AS String TO Integer"),
        ParameterType::thread_handle(
            false,
            ParameterType::func(vec![ParameterType::Integer], ParameterType::String),
            ParameterType::Nothing,
            ParameterType::Integer,
        ),
    );
    assert_eq!(
        ParameterType::parse("Thread OF Pair OF (Integer, String) TO Integer"),
        ParameterType::thread_handle(
            false,
            ParameterType::user_of("Pair", vec![ParameterType::Integer, ParameterType::String]),
            ParameterType::Nothing,
            ParameterType::Integer,
        ),
    );
}

/// bug-605: `display()` is the spelling a user reads — a package qualifier is
/// `::` at every depth — while `name()` keeps the internal dot.
#[test]
fn display_spells_package_qualifiers_as_source_does() {
    let color = || ParameterType::named("color.Color");
    assert_eq!(color().name(), "color.Color");
    assert_eq!(color().display(), "color::Color");
    assert_eq!(
        ParameterType::list_of(color()).display(),
        "List OF color::Color"
    );
    assert_eq!(
        ParameterType::map_of(ParameterType::String, ParameterType::named("net.Address")).display(),
        "Map OF String TO net::Address"
    );
    assert_eq!(
        ParameterType::res(ParameterType::named("canvas.Font")).display(),
        "RES canvas::Font"
    );
    assert_eq!(
        ParameterType::list_of(ParameterType::map_of(
            ParameterType::named("json.Json"),
            ParameterType::list_of(ParameterType::res(ParameterType::named("tcp.Socket"))),
        ))
        .display(),
        "List OF Map OF json::Json TO List OF RES tcp::Socket"
    );
    assert_eq!(ParameterType::Integer.display(), "Integer");
}
