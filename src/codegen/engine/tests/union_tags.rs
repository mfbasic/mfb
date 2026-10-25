//! The `TypeModel` built from a NIR module (`engine/validation/validation.rs`): record
//! and union layouts, key shapes and builtin records.

use crate::codegen::engine::builder::TypeModel;
use crate::target::shared::nir::{NirModule, NirType, NirVariant};
use crate::types::ParameterType;

fn union(name: &str, includes: &[&str], variants: &[&str]) -> NirType {
    NirType {
        kind: "union".to_string(),
        visibility: "private".to_string(),
        name: name.to_string(),
        fields: Vec::new(),
        includes: includes.iter().map(|s| s.to_string()).collect(),
        variants: variants
            .iter()
            .map(|s| NirVariant {
                name: s.to_string(),
                fields: Vec::new(),
            })
            .collect(),
        members: Vec::new(),
    }
}

fn module(types: Vec<NirType>) -> NirModule {
    NirModule {
        target: "test".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        stdin_log_cap: crate::codegen::error::constants::STDIN_LOG_CAP_DEFAULT,
        debug: crate::codegen::debug::DebugOptions::OFF,
        project: "test".to_string(),
        entry: None,
        globals: Vec::new(),
        types,
        imports: Vec::new(),
        runtime_helpers: Vec::new(),
        functions: Vec::new(),
        link_functions: Vec::new(),
        link_cstructs: Vec::new(),
        native_resources: Vec::new(),
        native_libraries: Default::default(),
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        arena_memory: crate::ir::ArenaMemoryLimits::default(),
    }
}

fn record(name: &str, fields: &[(&str, ParameterType)]) -> NirType {
    NirType {
        kind: "record".to_string(),
        visibility: "private".to_string(),
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|(field, type_)| crate::target::shared::nir::NirField {
                name: (*field).to_string(),
                type_: type_.clone(),
                visibility: None,
            })
            .collect(),
        includes: Vec::new(),
        variants: Vec::new(),
        members: Vec::new(),
    }
}

/// plan-111-C Phase 2: the two key shapes most likely to differ between a
/// spelling-keyed and a type-keyed lookup.
///
/// A **nested container** (`List OF Map OF String TO Integer`) is the shape
/// where a naive re-key could decompose differently at each level; a
/// **stateful resource** (`File STATE Cursor`) is the one plan-111-A gave a
/// variant, so its spelling and its structure stopped being the same thing.
/// Both must resolve to the entry their declaration registered.
#[test]
fn a_type_model_resolves_nested_container_and_stateful_resource_keys() {
    let nested = ParameterType::parse("List OF Map OF String TO Integer");
    let stateful = ParameterType::parse("File STATE Cursor");
    let model = TypeModel::from_module(&module(vec![
        record("Holder", &[("items", nested.clone())]),
        record("Handle", &[("state", stateful.clone())]),
    ]))
    .expect("model builds");

    // The declarations are reachable by the type their own name denotes...
    let holder = model
        .record_fields
        .get(&ParameterType::declared("Holder"))
        .expect("Holder is registered");
    assert_eq!(
        holder[0].1, nested,
        "the nested container survives re-keying"
    );
    let handle = model
        .record_fields
        .get(&ParameterType::declared("Handle"))
        .expect("Handle is registered");
    assert_eq!(
        handle[0].1, stateful,
        "the stateful resource survives re-keying"
    );

    // ...and a key built from the SPELLING finds the same entry, which is the
    // property every emitter still holding a name depends on.
    assert!(model.record_fields.contains_key(&ParameterType::declared(
        &ParameterType::declared("Holder").name()
    )));

    // A composite is not a record key and must miss, both before and after.
    assert!(!model.record_fields.contains_key(&nested));
    assert!(!model.record_fields.contains_key(&stateful));
}

/// plan-132 C3: a builtin record has its real layout in a program that never
/// imports its package — `tcp::localAddress` hands a `net.Address` to a file that
/// imports only `tcp` — with each field qualified the way the NIR spells it, and
/// without a bare alias that could capture a project type of the same leaf.
#[test]
fn a_builtin_record_has_its_layout_without_an_import() {
    let model = TypeModel::from_module(&module(Vec::new())).expect("model builds");
    let fields = |name: &str| {
        model
            .record_fields
            .get(&ParameterType::declared(name))
            .unwrap_or_else(|| panic!("`{name}` is not registered"))
            .clone()
    };
    assert_eq!(
        fields("net.Address"),
        vec![
            ("host".to_string(), ParameterType::String),
            ("port".to_string(), ParameterType::Integer),
        ]
    );
    let ping = fields("net.PingResult");
    assert_eq!(
        ping[0].1,
        ParameterType::declared("net.PingStatus"),
        "a same-package field type is qualified, as the NIR spells it"
    );
    assert_eq!(ping[1].1, ParameterType::declared("net.Address"));
    assert_eq!(
        fields("udp.Datagram")[0].1,
        ParameterType::declared("net.Address"),
        "a cross-package field type is qualified"
    );
    assert_eq!(fields("audio.AudioDevice").len(), 6);
    assert!(
        !model
            .record_fields
            .contains_key(&ParameterType::declared("Address")),
        "a registered builtin layout must not add a bare alias"
    );
}

/// The registry only fills gaps: a record the module itself registered under a
/// builtin record's key keeps the module's layout.
#[test]
fn a_module_record_keeps_its_own_layout_over_the_builtin_one() {
    let model = TypeModel::from_module(&module(vec![record(
        "net.Address",
        &[("only", ParameterType::Integer)],
    )]))
    .expect("model builds");
    assert_eq!(
        model.record_fields[&ParameterType::declared("net.Address")],
        vec![("only".to_string(), ParameterType::Integer)]
    );
}

/// Tags are globally-canonical: keyed by the (sorted) variant name, not by a
/// variant's position within any including union (bug-80). Variants Sq, Tri,
/// V1 sort to tags 0, 1, 2.
#[test]
fn canonical_tags_are_name_sorted() {
    let types = vec![
        union("UV", &[], &["V1"]),
        union("Shape", &["UV"], &["Sq"]),
        union("Wide", &["Shape"], &["Tri"]),
    ];
    let model = TypeModel::from_module(&module(types)).expect("must resolve");
    assert_eq!(
        model.union_variant_tags.get(&ParameterType::declared("Sq")),
        Some(&0)
    );
    assert_eq!(
        model
            .union_variant_tags
            .get(&ParameterType::declared("Tri")),
        Some(&1)
    );
    assert_eq!(
        model.union_variant_tags.get(&ParameterType::declared("V1")),
        Some(&2)
    );
}

/// A variant at *divergent* positions across two unions (`W1` follows `V1` in
/// `A` but is first in `L2`) is no longer rejected — the canonical scheme gives
/// it ONE stable tag everywhere (sorted: V1=0, W1=1), so a MATCH dispatches
/// consistently regardless of which union viewed it (bug-80).
#[test]
fn divergent_positions_resolve_to_one_canonical_tag() {
    let types = vec![
        union("UV", &[], &["V1"]),
        union("UW", &[], &["W1"]),
        union("A", &["UV", "UW"], &[]),
        union("L2", &["UW"], &[]),
    ];
    let model =
        TypeModel::from_module(&module(types)).expect("divergent positions must now resolve");
    assert_eq!(
        model.union_variant_tags.get(&ParameterType::declared("V1")),
        Some(&0)
    );
    assert_eq!(
        model.union_variant_tags.get(&ParameterType::declared("W1")),
        Some(&1)
    );
}

/// A package-qualified union's variants get one dense canonical tag each, and
/// their bare aliases share it — through `from_module_and_packages`, the
/// constructor every build uses.
///
/// plan-132 C5: `from_module_and_packages` used to call `from_module`, which had
/// already aliased the bare leaves, and then recompute the tags over a table
/// that held `A`, `B`, `pkg.A` and `pkg.B` — the order `finish` documents as
/// wrong — so the qualified variants took tags 2 and 3 (`json.JsonObj` was 10 of
/// 12, measured in `json_codegen_cover_rt`). Every tag consumer reads the model,
/// so the sparse space was consistent; this pins the one-pass order.
#[test]
fn a_qualified_union_is_tagged_once_through_the_package_constructor() {
    let types = vec![union("pkg.U", &[], &["pkg.A", "pkg.B"])];
    let model = TypeModel::from_module_and_packages(&module(types), &[])
        .expect("the package constructor builds with no packages");
    for (variant, tag) in [("pkg.A", 0), ("pkg.B", 1), ("A", 0), ("B", 1)] {
        assert_eq!(
            model
                .union_variant_tags
                .get(&ParameterType::declared(variant)),
            Some(&tag),
            "`{variant}` must carry canonical tag {tag}"
        );
    }
}
