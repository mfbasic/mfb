// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::list_element_is_fixed_width;
use crate::codegen::engine::builder::bind_deferred_relocation_libraries;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;
#[test]
fn negative_integer_const_materializes_as_its_u64_bit_pattern() {
    assert_eq!(
        native_immediate_value(
            &crate::types::ParameterType::declared("Integer"),
            "-9223372036854775808"
        ),
        Ok((i64::MIN as u64).to_string())
    );
    assert_eq!(
        native_immediate_value(&crate::types::ParameterType::declared("Integer"), "-1"),
        Ok(u64::MAX.to_string())
    );
    // Non-negative Integers are passed through unchanged.
    assert_eq!(
        native_immediate_value(&crate::types::ParameterType::declared("Integer"), "0"),
        Ok("0".to_string())
    );
    assert_eq!(
        native_immediate_value(
            &crate::types::ParameterType::declared("Integer"),
            "9223372036854775807"
        ),
        Ok(i64::MAX.to_string())
    );
}
/// One relocation carrying the deferred-library placeholder, for the binding
/// tests below.
fn deferred_reloc_function(symbol: &str) -> CodeFunction {
    CodeFunction {
        name: "probe".to_string(),
        symbol: "probe".to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions: Vec::new(),
        relocations: vec![CodeRelocation {
            from: "probe".to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(String::new()),
        }],
        stack_slots: Vec::new(),
    }
}

/// plan-56-A §4.2: a deferred relocation binds to whatever the platform import
/// map says — which is what makes a musl app build label its relocations with
/// the musl libc instead of `libc.so.6`.
#[test]
fn deferred_relocation_binds_from_the_platform_import_map() {
    let mut functions = vec![deferred_reloc_function("close")];
    let map: HashMap<String, String> = [("close".to_string(), "libc.musl-x86_64.so.1".to_string())]
        .into_iter()
        .collect();
    bind_deferred_relocation_libraries(&mut functions, &map).expect("binds");
    assert_eq!(
        functions[0].relocations[0].library.as_deref(),
        Some("libc.musl-x86_64.so.1")
    );
}

/// An undeclared symbol is a codegen bug and must surface as a plan-level error
/// rather than shipping a relocation labelled with no library at all. This is
/// the invariant the deleted `lib_for` asserted informally by existing.
#[test]
fn deferred_relocation_rejects_an_undeclared_symbol() {
    let mut functions = vec![deferred_reloc_function("getenv")];
    let err = bind_deferred_relocation_libraries(&mut functions, &HashMap::new())
        .expect_err("an undeclared symbol must be rejected");
    assert!(err.contains("getenv"), "{err}");
    assert!(err.contains("does not declare"), "{err}");
}

/// A relocation that already names a library, or names none at all, is left
/// untouched — so every non-Linux backend is unaffected by the placeholder pass.
#[test]
fn binding_leaves_already_resolved_and_none_relocations_alone() {
    let mut functions = vec![deferred_reloc_function("close")];
    functions[0].relocations[0].library = Some("libSystem.B.dylib".to_string());
    functions.push(deferred_reloc_function("close"));
    functions[1].relocations[0].library = None;

    bind_deferred_relocation_libraries(&mut functions, &HashMap::new())
        .expect("neither relocation is deferred, so no lookup happens");
    assert_eq!(
        functions[0].relocations[0].library.as_deref(),
        Some("libSystem.B.dylib")
    );
    assert_eq!(functions[1].relocations[0].library, None);
}

/// bug-365: `list_element_is_fixed_width` promises `entry[i].valueOffset ==
/// i * size`, so its `size` must be the payload alignment the rest of the
/// collection machinery packs to. If the two ever disagree, a list would be
/// packed at one stride and read at another — silent data corruption with no
/// diagnostic, which is exactly the class of defect bug-365 was.
///
/// `collection_payload_alignment_for_code` is the code-keyed mirror of
/// `CodeBuilder::collection_payload_alignment` (which needs a whole builder to
/// call), so asserting against it pins the same numbers.
#[test]
fn fixed_width_agrees_with_payload_alignment() {
    for spelling in [
        "Boolean", "Byte", "Scalar", "Integer", "Float", "Fixed", "Money",
    ] {
        let type_ = crate::types::ParameterType::parse(spelling);
        let size = list_element_is_fixed_width(&type_)
            .unwrap_or_else(|| panic!("{type_} must be fixed-width"));
        let code = collection_type_code(&type_)
            .unwrap_or_else(|| panic!("{type_} must have a collection type code"));
        assert_eq!(
            size,
            collection_payload_alignment_for_code(code),
            "{type_}: fixed-width stride and payload alignment disagree"
        );
    }
}

/// The variable-width types must stay out, or the three mutation sites would
/// index them at a constant stride they do not have. `String` is the live one —
/// `lower_sort_string_list_helper` deliberately permutes a string list's entries
/// without moving its data, so a `String` list is legitimately out of order.
#[test]
fn variable_width_element_types_are_not_fixed_width() {
    for type_ in [
        "String",
        "List OF Integer",
        "Map OF String TO Integer",
        "SomeRecord",
    ] {
        assert_eq!(
            list_element_is_fixed_width(&crate::types::ParameterType::declared(type_)),
            None,
            "{type_} must not claim a fixed stride"
        );
    }
}

/// Whether `path` is codegen lowering source for the overflow-label scan below: a `.rs` file
/// whose own name, or any directory between `root` and it, starts with `builder_`, `func_` or
/// `gen_`. The directory half keeps a split carrier in the scan (plan-176-O): `builder_x.rs`
/// becomes `builder_x/mod.rs` + `builder_x/part.rs`, and `part.rs` carries no prefix itself.
fn is_lowering_source(root: &std::path::Path, path: &std::path::Path) -> bool {
    const PREFIXES: [&str; 3] = ["builder_", "func_", "gen_"];
    if path.extension().and_then(|e| e.to_str()) != Some("rs") {
        return false;
    }
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    rel.components().any(|part| {
        part.as_os_str()
            .to_str()
            .is_some_and(|n| PREFIXES.iter().any(|p| n.starts_with(p)))
    })
}

/// The old rule read only a file's own name, so a split carrier's parts left the scan
/// silently; these are the cases that pins (plan-176-O §4.0).
#[test]
fn is_lowering_source_takes_files_under_prefixed_directories() {
    let root = std::path::Path::new("/r/src/codegen");
    assert!(is_lowering_source(root, &root.join("a/builder_x/part.rs")));
    assert!(is_lowering_source(root, &root.join("a/builder_x/mod.rs")));
    assert!(is_lowering_source(root, &root.join("a/func_y/part.rs")));
    assert!(is_lowering_source(root, &root.join("a/builder_x.rs")));
    assert!(is_lowering_source(root, &root.join("a/gen_z.rs")));
    assert!(!is_lowering_source(root, &root.join("a/other/part.rs")));
    assert!(!is_lowering_source(
        root,
        &root.join("a/builder_x/notes.md")
    ));
    // A prefixed directory ABOVE the root does not count.
    let nested = std::path::Path::new("/r/builder_top/src/codegen");
    assert!(!is_lowering_source(nested, &nested.join("a/part.rs")));
}

/// bug-352: an allocation-size-overflow guard must NOT raise its error through
/// `emit_allocation_error_return`. That helper reads the error code out of the
/// result-tag register (`x0`), which is only valid *after* a failed
/// `_mfb_arena_alloc` call. An overflow label sits on a *pre-call* edge where the
/// checked-size helper has just deposited a partially-computed size into that same
/// register, so the register form would surface that size as the error code. The
/// correct idiom is `emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, …)`, which
/// materializes the code into a fresh register first.
///
/// This scans the codegen lowering source directly (over every current and future
/// `builder_*.rs` legacy carrier plus the clean-room `func_*.rs`/`gen_*.rs` member
/// lowerings the migration produces) so a fourth mis-wired site is a
/// compile-time-suite failure, not a silently-garbage error payload nobody can
/// reach to observe.
#[test]
fn no_overflow_label_returns_through_the_result_tag_register() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/codegen");
    let mut offenders = Vec::new();
    let mut checked_files = 0usize;
    let mut overflow_labels = 0usize;

    // The lowering corpus is scattered across the codegen tiers after the plan-95
    // migration, so walk recursively rather than reading a flat dir. It spans the
    // legacy `builder_*.rs` carriers and the clean-room `func_*.rs`/`gen_*.rs`
    // member lowerings that supersede them (crypto/io/strings migrations).
    fn collect_builder_sources(
        root: &std::path::Path,
        dir: &std::path::Path,
        out: &mut Vec<std::path::PathBuf>,
    ) {
        for entry in std::fs::read_dir(dir)
            .expect("read codegen dir")
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_dir() {
                collect_builder_sources(root, &path, out);
            } else if is_lowering_source(root, &path) {
                out.push(path);
            }
        }
    }
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    collect_builder_sources(&dir, &dir, &mut entries);
    entries.sort();

    for path in entries {
        checked_files += 1;
        let source = std::fs::read_to_string(&path).expect("read builder source");
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            // Match an emitted label whose name mentions "overflow" — the guard
            // labels for allocation-size wraps (`overflow`, `size_overflow`,
            // `ascii_size_overflow`).
            if line.contains("abi::label(&") && line.contains("overflow") {
                overflow_labels += 1;
                // The very next emitted return must not be the register form.
                if let Some(next) = lines.get(index + 1) {
                    if next.contains("emit_allocation_error_return") {
                        let file = path.file_name().unwrap().to_string_lossy();
                        offenders.push(format!("{file}:{}", index + 2));
                    }
                }
            }
        }
    }

    assert!(
        checked_files > 20,
        "expected the builder corpus, scanned {checked_files}"
    );
    assert!(
        overflow_labels >= 20,
        "expected the overflow labels, found {overflow_labels}"
    );
    assert!(
        offenders.is_empty(),
        "overflow label(s) return through the result-tag register \
         (emit_allocation_error_return reads x0, which holds the computed size \
         here, not an error code — use emit_error_code_return): {offenders:?}"
    );
}
