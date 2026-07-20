use super::builder_collection_layout::list_element_is_fixed_width;
use super::*;

#[test]
fn free_list_bump_then_reuse() {
    // One 4064-byte block (4096 mapped − 32 header). Allocate a few chunks;
    // with no frees the list is one shrinking trailing entry (bump behavior).
    let mut sim = FreeListSim::default();
    sim.insert_free(0x1020, 4064);
    let a = sim.alloc(24, 8).unwrap(); // → rounded 32
    let b = sim.alloc(16, 8).unwrap();
    let c = sim.alloc(100, 8).unwrap(); // → rounded 112
    assert_eq!(a, 0x1020);
    assert_eq!(b, 0x1040);
    assert_eq!(c, 0x1050);
    assert_eq!(sim.nodes.len(), 1, "bump leaves one trailing free entry");
    sim.assert_invariants();
    // Free the middle chunk; first-fit reuses that 16-byte hole next.
    sim.free(b, 16);
    sim.assert_invariants();
    let d = sim.alloc(16, 8).unwrap();
    assert_eq!(d, b, "low-address hole is reused before the trailing entry");
}

#[test]
fn free_list_coalesces_neighbors() {
    let mut sim = FreeListSim::default();
    sim.insert_free(0x1000, 0x1000);
    let a = sim.alloc(64, 16).unwrap();
    let b = sim.alloc(64, 16).unwrap();
    let c = sim.alloc(64, 16).unwrap();
    let free_before = sim.free_bytes();
    // Free a and c (non-adjacent) → two holes; then b merges all three.
    sim.free(a, 64);
    sim.free(c, 64);
    sim.assert_invariants();
    sim.free(b, 64);
    sim.assert_invariants();
    assert_eq!(sim.free_bytes(), free_before + 3 * 64);
    // After full coalescing the block is whole again: a single entry.
    assert_eq!(sim.nodes.len(), 1);
    assert_eq!(sim.nodes[0].0, a);
}

#[test]
fn free_list_same_shape_churn_stays_short() {
    // A loop that allocs/frees the same shape each pass must not grow the
    // list: the freed chunk coalesces straight back into its neighbor.
    let mut sim = FreeListSim::default();
    sim.insert_free(0x2000, 0x4000);
    for _ in 0..1000 {
        let p = sim.alloc(48, 16).unwrap();
        sim.free(p, 48);
        assert!(sim.nodes.len() <= 1, "churn must keep the list ~1 entry");
    }
    sim.assert_invariants();
    assert_eq!(sim.free_bytes(), 0x4000);
}

#[test]
fn free_list_never_merges_across_blocks() {
    // Two separate blocks (header gap between them). Freeing the last chunk
    // of the low block must not merge into the high block.
    let mut sim = FreeListSim::default();
    sim.insert_free(0x1020, 4064); // block A usable
    sim.insert_free(0x3020, 4064); // block B usable (non-contiguous)
    let a = sim.alloc(4064, 16).unwrap(); // consume all of A
    assert_eq!(a, 0x1020);
    assert_eq!(sim.nodes.len(), 1, "only B remains free");
    sim.free(a, 4064);
    sim.assert_invariants();
    assert_eq!(sim.nodes.len(), 2, "A and B stay distinct (header gap)");
}

#[test]
fn free_list_over_aligns_to_16_with_front_split() {
    let mut sim = FreeListSim::default();
    sim.insert_free(0x1010, 0x1000); // start 16-aligned but not 64-aligned
    let p = sim.alloc(32, 64).unwrap();
    assert_eq!(p % 64, 0);
    assert!(p > 0x1010, "front padding split into its own free chunk");
    sim.assert_invariants();
    // Freeing reconstitutes by merging with the front padding chunk.
    let before = sim.free_bytes();
    sim.free(p, 32);
    sim.assert_invariants();
    assert_eq!(sim.free_bytes(), before + 32);
}

#[test]
fn arena_rejects_invalid_alignment() {
    assert_eq!(
        checked_arena_used_after_alloc(0x1000, 0, 128, 8, 0),
        Err(77050002)
    );
    assert_eq!(
        checked_arena_used_after_alloc(0x1000, 0, 128, 8, 3),
        Err(77050002)
    );
}

#[test]
fn arena_handles_zero_size_allocations() {
    assert_eq!(
        checked_arena_used_after_alloc(0x1000, 0, 128, 0, 8),
        Ok((0x1020, 1))
    );
}

#[test]
fn arena_checks_alignment_rounding_and_capacity() {
    assert_eq!(
        checked_arena_used_after_alloc(0x1003, 5, 128, 8, 16),
        Ok((0x1030, 21))
    );
    assert_eq!(
        checked_arena_used_after_alloc(0x1000, 120, 128, 16, 16),
        Err(77010001)
    );
}

#[test]
fn arena_checks_arithmetic_overflow() {
    assert_eq!(
        checked_arena_used_after_alloc(u64::MAX - 8, 0, 128, 8, 8),
        Err(77010001)
    );
    assert_eq!(
        checked_arena_used_after_alloc(0x1000, 0, u64::MAX, u64::MAX, 8),
        Err(77010001)
    );
}

/// bug-286: both backends' immediate encoders parse `u64`, so a const printed
/// with a leading `-` is rejected outright ("invalid immediate"). Once
/// `ir::lower` folds the most-negative `Integer` literal into a signed
/// `Const{Integer, "-9223372036854775808"}`, that const has to reach the
/// encoder as its u64 bit pattern — the same treatment `Fixed` and `Money`
/// already give their `i64::MIN` raws. Non-negative Integers must stay
/// byte-identical so no existing codegen shifts.
#[test]
fn negative_integer_const_materializes_as_its_u64_bit_pattern() {
    assert_eq!(
        native_immediate_value("Integer", "-9223372036854775808"),
        Ok((i64::MIN as u64).to_string())
    );
    assert_eq!(
        native_immediate_value("Integer", "-1"),
        Ok(u64::MAX.to_string())
    );
    // Non-negative Integers are passed through unchanged.
    assert_eq!(native_immediate_value("Integer", "0"), Ok("0".to_string()));
    assert_eq!(
        native_immediate_value("Integer", "9223372036854775807"),
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
    for type_ in [
        "Boolean", "Byte", "Scalar", "Integer", "Float", "Fixed", "Money",
    ] {
        let size = list_element_is_fixed_width(type_)
            .unwrap_or_else(|| panic!("{type_} must be fixed-width"));
        let code = collection_type_code(type_)
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
            list_element_is_fixed_width(type_),
            None,
            "{type_} must not claim a fixed stride"
        );
    }
}
