use super::*;
use crate::target::shared::plan::{
    CallKind, NativePlan, PlanCall, PlannedFunction, PlatformImport, StorageClass, StorageType,
};

#[test]
fn lowers_libsystem_import_to_mach_o_object_plan() {
    let plan = NativePlan {
        target: "macos-aarch64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        project: "hello".to_string(),
        entry_symbol: Some("_mfb_fn_main".to_string()),
        runtime_symbols: vec!["_mfb_rt_io_io_print".to_string()],
        external_symbols: Vec::new(),
        platform_imports: vec![PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
            required_by: "_mfb_rt_io_io_print".to_string(),
        }],
        functions: vec![PlannedFunction {
            name: "main".to_string(),
            symbol: "_mfb_fn_main".to_string(),
            returns: StorageType {
                name: "Nothing".to_string(),
                class: StorageClass::Void,
                size: 0,
                align: 1,
            },
            params: Vec::new(),
            local_slots: Vec::new(),
            labels: Vec::new(),
            operations: vec!["eval runtimeCall io io.print(String(\"Hello World\"))".to_string()],
            calls: vec![PlanCall {
                target: "io.print".to_string(),
                symbol: "_mfb_rt_io_io_print".to_string(),
                kind: CallKind::Runtime,
                string_literals: vec!["Hello World".to_string()],
            }],
        }],
        link_symbols: Vec::new(),
    };

    let object = lower_plan(&plan).expect("object plan");
    assert_eq!(object.container, "mach-o");
    assert_eq!(object.status, "planOnly");
    assert_eq!(object.dylibs, vec!["/usr/lib/libSystem.B.dylib"]);
    assert_eq!(object.imported_symbols[0].symbol, "_write");
    assert_eq!(object.data_units[0].value, "Hello World");
    assert!(object
        .relocations
        .iter()
        .any(|relocation| relocation.kind == "externalCall"));
}

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

fn plan_call(target: &str, symbol: &str, kind: CallKind, literals: Vec<&str>) -> PlanCall {
    PlanCall {
        target: target.to_string(),
        symbol: symbol.to_string(),
        kind,
        string_literals: literals.into_iter().map(str::to_string).collect(),
    }
}

fn base_plan() -> NativePlan {
    NativePlan {
        target: "macos-aarch64".to_string(),
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

// Every code/data/reloc/symbol branch in one plan: functions (each CallKind),
// runtime symbols (with data refs + a matched import), a LINK symbol with its
// own import, deduplicated string literals, and multiple dylibs.
fn full_plan() -> NativePlan {
    let mut plan = base_plan();
    plan.runtime_symbols = vec!["_mfb_rt_io_io_print".to_string()];
    plan.link_symbols = vec!["_mfb_linker_init".to_string()];
    plan.platform_imports = vec![
        PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
            required_by: "_mfb_rt_io_io_print".to_string(),
        },
        PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_write".to_string(),
            required_by: "_mfb_rt_io_io_print".to_string(),
        },
        PlatformImport {
            library: "Network".to_string(),
            symbol: "_nw_path_monitor_create".to_string(),
            required_by: "_mfb_rt_io_io_print".to_string(),
        },
        PlatformImport {
            library: "libSystem".to_string(),
            symbol: "_dlopen".to_string(),
            required_by: "_mfb_linker_init".to_string(),
        },
    ];
    plan.functions = vec![
        function(
            "_mfb_fn_main",
            vec!["call local", "call import", "call runtime", "call indirect"],
            vec![
                plan_call("local", "_mfb_fn_helper", CallKind::Local, vec!["Hi"]),
                plan_call("pkg.f", "_pkg_f", CallKind::Import, vec!["Bye", "Hi"]),
                plan_call(
                    "io.print",
                    "_mfb_rt_io_io_print",
                    CallKind::Runtime,
                    Vec::new(),
                ),
                // Indirect call through a `FUNC`-typed local binding: no
                // linker symbol, and it must not become a relocation (bug-72).
                plan_call("addTwo", "", CallKind::Indirect, Vec::new()),
            ],
        ),
        function("_mfb_fn_helper", vec!["ret"], Vec::new()),
    ];
    plan
}

#[test]
fn lowers_full_plan_covering_every_branch() {
    let object = lower_plan(&full_plan()).expect("full object plan");
    object.validate().expect("full plan validates");
    // Two dylibs (libSystem, Network), first-seen order.
    assert_eq!(
        object.dylibs,
        vec![
            "/usr/lib/libSystem.B.dylib".to_string(),
            "/System/Library/Frameworks/Network.framework/Network".to_string(),
        ]
    );
    // Duplicate libSystem/_write import is deduplicated.
    assert_eq!(
        object
            .imported_symbols
            .iter()
            .filter(|s| s.symbol == "_write")
            .count(),
        1
    );
    // String literals deduplicated across calls ("Hi", "Bye").
    assert_eq!(object.data_units.len(), 2);
    assert_eq!(object.data_units[0].value, "Hi");
    assert_eq!(object.data_units[0].section, "__TEXT,__cstring");
    // cstring size is value.len()+1 (NUL), no 8-byte header (unlike Linux).
    assert_eq!(object.data_units[0].size, "Hi".len() + 1);
    // Runtime + link symbols are defined and code units.
    assert!(object
        .defined_symbols
        .contains(&"_mfb_rt_io_io_print".to_string()));
    assert!(object
        .defined_symbols
        .contains(&"_mfb_linker_init".to_string()));
    // packageCall -> external symbol.
    assert!(object.external_symbols.contains(&"_pkg_f".to_string()));
    let kinds: std::collections::HashSet<_> =
        object.relocations.iter().map(|r| r.kind.as_str()).collect();
    assert!(kinds.contains("internalCall"));
    assert!(kinds.contains("packageCall"));
    assert!(kinds.contains("externalCall"));
    assert!(kinds.contains("dataReference"));
    // An indirect call has no symbol, so it produces no relocation (bug-72):
    // there must be no `indirectCall` kind and nothing targeting the binding.
    assert!(!kinds.contains("indirectCall"));
    assert!(object.relocations.iter().all(|r| r.to != "addTwo"));
}

#[test]
fn to_json_emits_all_mach_o_sections() {
    let object = lower_plan(&full_plan()).expect("full object plan");
    let json = object.to_json();
    assert!(json.contains("\"format\": \"mfb-native-object-plan\""));
    assert!(json.contains("\"container\": \"mach-o\""));
    assert!(json.contains("LC_SEGMENT_64"));
    assert!(json.contains("LC_LOAD_DYLIB"));
    assert!(json.contains("__TEXT"));
    assert!(json.contains("__cstring"));
    assert!(json.contains("__LINKEDIT"));
    assert!(json.contains("read-execute"));
    assert!(json.contains("\"maxProtection\""));
    assert!(json.contains("\"stringTable\""));
    assert!(json.contains("\"kind\": \"externalCall\""));
    assert!(json.contains("\"kind\": \"imported\", \"section\": null"));
    // A linkedit section has a null section name.
    assert!(json.contains("\"section\": null, \"kind\": \"linkedit\""));
}

#[test]
fn to_json_minimal_plan_has_empty_collections() {
    let object = lower_plan(&base_plan()).expect("object plan");
    let json = object.to_json();
    assert!(json.contains("\"dylibs\": []"));
    assert!(json.contains("\"importedSymbols\": [\n  ]"));
    assert!(json.contains("\"externalSymbols\": []"));
}

#[test]
fn validate_rejects_wrong_target() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.target = "linux-aarch64".to_string();
    assert!(object
        .validate()
        .expect_err("bad target")
        .contains("does not match macos-aarch64"));
}

#[test]
fn validate_rejects_wrong_container_and_status() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.container = "elf".to_string();
    assert!(object
        .validate()
        .expect_err("bad container")
        .contains("plan-only Mach-O"));
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.status = "final".to_string();
    assert!(object
        .validate()
        .expect_err("bad status")
        .contains("plan-only Mach-O"));
}

#[test]
fn validate_rejects_entry_not_defined() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.entry = "_missing".to_string();
    assert!(object
        .validate()
        .expect_err("entry")
        .contains("is not a defined symbol"));
}

#[test]
fn validate_rejects_duplicate_defined_symbol_and_dylib() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.defined_symbols.push("_main".to_string());
    assert!(object
        .validate()
        .expect_err("dup symbol")
        .contains("duplicate defined symbol"));
    let mut object = lower_plan(&full_plan()).expect("object plan");
    object.dylibs.push("/usr/lib/libSystem.B.dylib".to_string());
    assert!(object
        .validate()
        .expect_err("dup dylib")
        .contains("duplicate dylib"));
}

#[test]
fn validate_rejects_code_unit_without_operations() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.code_units[0].operations.clear();
    assert!(object
        .validate()
        .expect_err("no ops")
        .contains("has no operations"));
}

#[test]
fn validate_rejects_missing_text_section() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object
        .sections
        .retain(|s| s.section.as_deref() != Some("__text"));
    assert!(object
        .validate()
        .expect_err("no text")
        .contains("requires __TEXT,__text"));
}

#[test]
fn validate_rejects_overlapping_sections() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    // Force __text and a second section to overlap in the file.
    object.sections[0].size = 0x100;
    object.sections[1].file_offset = object.sections[0].file_offset;
    object.sections[1].size = 0x100;
    assert!(object.validate().expect_err("overlap").contains("overlap"));
}

#[test]
fn validate_rejects_relocation_source_and_target() {
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.relocations.push(ObjectRelocation {
        from: "_ghost".to_string(),
        to: "_main".to_string(),
        kind: "internalCall".to_string(),
        section: "__TEXT,__text".to_string(),
    });
    assert!(object
        .validate()
        .expect_err("bad source")
        .contains("relocation source"));
    let mut object = lower_plan(&base_plan()).expect("object plan");
    object.relocations.push(ObjectRelocation {
        from: "_main".to_string(),
        to: "_ghost".to_string(),
        kind: "internalCall".to_string(),
        section: "__TEXT,__text".to_string(),
    });
    assert!(object
        .validate()
        .expect_err("bad target")
        .contains("relocation target"));
}

#[test]
fn dylib_for_library_maps_known_and_rejects_unknown() {
    assert_eq!(
        dylib_for_library("libSystem").unwrap(),
        "/usr/lib/libSystem.B.dylib"
    );
    assert_eq!(
        dylib_for_library("AppKit").unwrap(),
        "/System/Library/Frameworks/AppKit.framework/AppKit"
    );
    assert_eq!(
        dylib_for_library("Foundation").unwrap(),
        "/System/Library/Frameworks/Foundation.framework/Foundation"
    );
    assert_eq!(
        dylib_for_library("libobjc").unwrap(),
        "/usr/lib/libobjc.A.dylib"
    );
    assert_eq!(dylib_for_library("libz").unwrap(), "/usr/lib/libz.1.dylib");
    assert_eq!(
        dylib_for_library("AudioToolbox").unwrap(),
        "/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox"
    );
    assert_eq!(
        dylib_for_library("CoreAudio").unwrap(),
        "/System/Library/Frameworks/CoreAudio.framework/CoreAudio"
    );
    assert_eq!(
        dylib_for_library("CoreFoundation").unwrap(),
        "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation"
    );
    assert!(dylib_for_library("nope")
        .expect_err("unknown")
        .contains("does not know dylib"));
}

#[test]
fn dylibs_rejects_unknown_library_import() {
    let mut plan = base_plan();
    plan.platform_imports = vec![PlatformImport {
        library: "MysteryFramework".to_string(),
        symbol: "_x".to_string(),
        required_by: "_mfb_fn_main".to_string(),
    }];
    let err = match lower_plan(&plan) {
        Ok(_) => panic!("expected unknown library to fail lowering"),
        Err(err) => err,
    };
    assert!(err.contains("does not know dylib"), "{err}");
}

#[test]
fn plan_without_entry_symbol_emits_no_entry_relocation() {
    let mut plan = base_plan();
    plan.entry_symbol = None;
    let object = lower_plan(&plan).expect("object plan");
    assert!(!object
        .relocations
        .iter()
        .any(|r| r.from == "_main" && r.kind == "internalCall"));
}

#[test]
fn helpers_dedup_and_align() {
    assert_eq!(align(0, 16), 0);
    assert_eq!(align(1, 16), 16);
    assert_eq!(align(16, 16), 16);
    assert_eq!(planned_code_size(0), 4);
    assert_eq!(planned_code_size(1), 4);
    assert_eq!(planned_code_size(5), 20);
    let mut values = Vec::new();
    push_unique(&mut values, "a".to_string());
    push_unique(&mut values, "a".to_string());
    assert_eq!(values.len(), 1);
    let mut imports = Vec::new();
    push_import(
        &mut imports,
        ObjectImport {
            library: "l".to_string(),
            symbol: "s".to_string(),
        },
    );
    push_import(
        &mut imports,
        ObjectImport {
            library: "l".to_string(),
            symbol: "s".to_string(),
        },
    );
    assert_eq!(imports.len(), 1);
}

#[test]
fn section_name_formats_with_and_without_section() {
    let with = SectionPlan {
        segment: "__TEXT".to_string(),
        section: Some("__text".to_string()),
        kind: "code".to_string(),
        vm_address: 0,
        file_offset: 0,
        size: 0,
        align: 4,
    };
    assert_eq!(section_name(&with), "__TEXT,__text");
    let without = SectionPlan {
        segment: "__LINKEDIT".to_string(),
        section: None,
        kind: "linkedit".to_string(),
        vm_address: 0,
        file_offset: 0,
        size: 0,
        align: 1,
    };
    assert_eq!(section_name(&without), "__LINKEDIT");
}
