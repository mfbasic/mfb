use super::*;
use crate::target::shared::plan::{
    NativePlan, PlanCall, PlannedFunction, PlatformImport, StorageClass, StorageType,
};

fn void_type() -> StorageType {
    StorageType {
        name: "Nothing".to_string(),
        class: StorageClass::Void,
        size: 0,
        align: 1,
    }
}

fn function(symbol: &str, operations: Vec<&str>, calls: Vec<PlanCall>) -> PlannedFunction {
    PlannedFunction {
        name: symbol.trim_start_matches("_mfb_fn_").to_string(),
        symbol: symbol.to_string(),
        returns: void_type(),
        params: Vec::new(),
        local_slots: Vec::new(),
        labels: Vec::new(),
        operations: operations.into_iter().map(str::to_string).collect(),
        calls,
    }
}

fn call(target: &str, symbol: &str, kind: CallKind, literals: Vec<&str>) -> PlanCall {
    PlanCall {
        target: target.to_string(),
        symbol: symbol.to_string(),
        kind,
        string_literals: literals.into_iter().map(str::to_string).collect(),
    }
}

fn base_plan(target: &str) -> NativePlan {
    NativePlan {
        target: target.to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        project: "hello".to_string(),
        entry_symbol: Some("_mfb_fn_main".to_string()),
        runtime_symbols: Vec::new(),
        external_symbols: Vec::new(),
        platform_imports: Vec::new(),
        functions: vec![function("_mfb_fn_main", vec!["ret"], Vec::new())],
        link_symbols: Vec::new(),
    }
}

// The exhaustive plan drives every code/data/reloc/symbol branch: functions
// (each CallKind), runtime symbols with data refs, LINK symbols, imports,
// string literals, and a package (import) call producing an external symbol.
fn full_plan(target: &str) -> NativePlan {
    let mut plan = base_plan(target);
    plan.runtime_symbols = vec!["_mfb_rt_io_io_print".to_string()];
    plan.link_symbols = vec!["_mfb_linker_init".to_string()];
    plan.platform_imports = vec![PlatformImport {
        library: "libc.so.6".to_string(),
        symbol: "write".to_string(),
        required_by: "_mfb_rt_io_io_print".to_string(),
    }];
    plan.functions = vec![function(
        "_mfb_fn_main",
        vec!["call local", "call import", "call runtime", "call indirect"],
        vec![
            call("local", "_mfb_fn_helper", CallKind::Local, vec!["Hello"]),
            call("pkg.f", "_pkg_f", CallKind::Import, vec!["World", "Hello"]),
            call(
                "io.print",
                "_mfb_rt_io_io_print",
                CallKind::Runtime,
                Vec::new(),
            ),
            // Indirect call through a `FUNC`-typed local binding: no linker
            // symbol, and it must not become a relocation (bug-72).
            call("addTwo", "", CallKind::Indirect, Vec::new()),
        ],
    )];
    plan.functions
        .push(function("_mfb_fn_helper", vec!["ret"], Vec::new()));
    plan
}

#[test]
fn lowers_minimal_plan_to_static_elf_object() {
    let object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    assert_eq!(object.container, "elf");
    assert_eq!(object.status, "planOnly");
    assert_eq!(object.image_base, IMAGE_BASE);
    assert!(object.dylibs.is_empty());
    assert!(object.imported_symbols.is_empty());
    assert!(object.data_units.is_empty());
    // The synthetic _main entry plus the single planned function.
    assert!(object.defined_symbols.contains(&"_main".to_string()));
    assert!(object.defined_symbols.contains(&"_mfb_fn_main".to_string()));
    assert_eq!(object.sections[0].section.as_deref(), Some(".text"));
    assert_eq!(object.sections[1].section.as_deref(), Some(".rodata"));
}

#[test]
fn lowers_x86_target() {
    let object = lower_plan(&base_plan("linux-x86_64")).expect("x86 object plan");
    assert_eq!(object.target, "linux-x86_64");
    object.validate().expect("x86 validates");
}

#[test]
fn lowers_full_plan_covering_every_branch() {
    let object = lower_plan(&full_plan("linux-aarch64")).expect("full object plan");
    // Data units for the deduplicated string literals ("Hello", "World").
    assert_eq!(object.data_units.len(), 2);
    assert_eq!(object.data_units[0].value, "Hello");
    assert_eq!(object.data_units[1].value, "World");
    assert_eq!(object.data_units[0].section, ".rodata");
    // The second unit's offset follows the first (aligned to 8).
    assert_eq!(object.data_units[0].offset, 0);
    assert!(object.data_units[1].offset >= object.data_units[0].size);
    // Runtime + link symbols appear in code units and defined symbols.
    assert!(object
        .defined_symbols
        .contains(&"_mfb_rt_io_io_print".to_string()));
    assert!(object
        .defined_symbols
        .contains(&"_mfb_linker_init".to_string()));
    // Import call becomes an external symbol (packageCall relocation target).
    assert!(object.external_symbols.contains(&"_pkg_f".to_string()));
    assert!(object.imported_symbols.iter().any(|s| s.symbol == "write"));
    // Relocation kinds: internalCall (entry), packageCall,
    // dataReference (runtime symbol -> data unit).
    let kinds: std::collections::HashSet<_> =
        object.relocations.iter().map(|r| r.kind.as_str()).collect();
    assert!(kinds.contains("internalCall"));
    assert!(kinds.contains("packageCall"));
    assert!(kinds.contains("dataReference"));
    // An indirect call has no symbol, so it produces no relocation (bug-72):
    // there must be no `indirectCall` kind and nothing targeting the binding.
    assert!(!kinds.contains("indirectCall"));
    assert!(object.relocations.iter().all(|r| r.to != "addTwo"));
}

#[test]
fn to_json_emits_all_sections_and_round_trips_shape() {
    let object = lower_plan(&full_plan("linux-aarch64")).expect("full object plan");
    let json = object.to_json();
    assert!(json.contains("\"format\": \"mfb-native-object-plan\""));
    assert!(json.contains("\"container\": \"elf\""));
    assert!(json.contains("\"target\": \"linux-aarch64\""));
    assert!(json.contains("\"loadCommands\""));
    assert!(json.contains("PT_LOAD"));
    assert!(json.contains("\".text\""));
    assert!(json.contains("\".rodata\""));
    assert!(json.contains("\"stringTable\""));
    assert!(json.contains("\"relocations\""));
    assert!(json.contains("\"externalSymbols\""));
    assert!(json.contains("\"library\": \"libc.so.6\""));
    assert!(json.contains("\"kind\": \"packageCall\""));
    // The imported symbol has a null section/value in the symbol table.
    assert!(json.contains("\"kind\": \"imported\", \"section\": null"));
}

#[test]
fn to_json_minimal_plan_has_empty_collections() {
    let object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    let json = object.to_json();
    assert!(json.contains("\"dylibs\": []"));
    assert!(json.contains("\"importedSymbols\": [\n  ]"));
    assert!(json.contains("\"externalSymbols\": []"));
}

#[test]
fn validate_rejects_unknown_target() {
    let object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    let mut broken = object;
    broken.target = "macos-aarch64".to_string();
    let err = broken.validate().expect_err("bad target");
    assert!(err.contains("not a supported Linux target"), "{err}");
}

#[test]
fn validate_rejects_wrong_container_and_status() {
    let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    object.container = "mach-o".to_string();
    assert!(object
        .validate()
        .expect_err("bad container")
        .contains("plan-only ELF"));
    let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    object.status = "final".to_string();
    assert!(object
        .validate()
        .expect_err("bad status")
        .contains("plan-only ELF"));
}

#[test]
fn validate_rejects_entry_not_defined() {
    let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    object.entry = "_missing".to_string();
    assert!(object
        .validate()
        .expect_err("entry not defined")
        .contains("not a defined symbol"));
}

#[test]
fn validate_rejects_duplicate_defined_symbol() {
    let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    object.defined_symbols.push("_main".to_string());
    assert!(object
        .validate()
        .expect_err("duplicate")
        .contains("duplicate defined symbol"));
}

#[test]
fn validate_rejects_relocation_source_not_defined() {
    let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    object.relocations.push(ObjectRelocation {
        from: "_ghost".to_string(),
        to: "_main".to_string(),
        kind: "internalCall".to_string(),
        section: ".text".to_string(),
    });
    assert!(object
        .validate()
        .expect_err("bad source")
        .contains("relocation source"));
}

#[test]
fn validate_rejects_relocation_target_not_defined() {
    let mut object = lower_plan(&base_plan("linux-aarch64")).expect("object plan");
    object.relocations.push(ObjectRelocation {
        from: "_main".to_string(),
        to: "_ghost".to_string(),
        kind: "internalCall".to_string(),
        section: ".text".to_string(),
    });
    assert!(object
        .validate()
        .expect_err("bad target")
        .contains("relocation target"));
}

#[test]
fn validate_accepts_relocation_to_imported_or_external_symbol() {
    // The full plan already carries an imported symbol ("write") reachable via
    // an externalCall-style relocation and a packageCall external symbol, so a
    // successful validate proves both the imported and external branches.
    let object = lower_plan(&full_plan("linux-aarch64")).expect("full object plan");
    object.validate().expect("full plan validates");
}

#[test]
fn push_relocation_deduplicates_identical_entries() {
    let mut relocations = Vec::new();
    let make = || ObjectRelocation {
        from: "a".to_string(),
        to: "b".to_string(),
        kind: "internalCall".to_string(),
        section: ".text".to_string(),
    };
    push_relocation(&mut relocations, make());
    push_relocation(&mut relocations, make());
    assert_eq!(relocations.len(), 1);
}

#[test]
fn push_unique_skips_repeats() {
    let mut values = Vec::new();
    push_unique(&mut values, "x".to_string());
    push_unique(&mut values, "x".to_string());
    push_unique(&mut values, "y".to_string());
    assert_eq!(values, vec!["x".to_string(), "y".to_string()]);
}

#[test]
fn align_rounds_up_to_multiple() {
    assert_eq!(align(0, 8), 0);
    assert_eq!(align(1, 8), 8);
    assert_eq!(align(8, 8), 8);
    assert_eq!(align(9, 8), 16);
}

#[test]
fn plan_without_entry_symbol_emits_no_entry_relocation() {
    let mut plan = base_plan("linux-aarch64");
    plan.entry_symbol = None;
    let object = lower_plan(&plan).expect("object plan");
    // No entry_symbol -> the synthetic _main carries an empty call and there is
    // no internalCall relocation from _main to a named entry.
    assert!(!object
        .relocations
        .iter()
        .any(|r| r.from == "_main" && r.kind == "internalCall"));
}
