// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::list_element_is_fixed_width;
use crate::codegen::engine::builder::bind_deferred_relocation_libraries;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;
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
    fn collect_builder_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir)
            .expect("read codegen dir")
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_dir() {
                collect_builder_sources(&path, out);
            } else if path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.ends_with(".rs")
                    && (n.starts_with("builder_")
                        || n.starts_with("func_")
                        || n.starts_with("gen_"))
            }) {
                out.push(path);
            }
        }
    }
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    collect_builder_sources(&dir, &mut entries);
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

fn checked_arena_used_after_alloc(
    block_base: u64,
    current_offset: u64,
    capacity: u64,
    size: u64,
    align: u64,
) -> Result<(u64, u64), u64> {
    let invalid_argument = crate::codegen::registry::runtime_error("ErrInvalidArgument")
        .expect("errorCode name")
        .0
        .parse::<u64>()
        .expect("invalid argument code");
    let out_of_memory = crate::codegen::registry::runtime_error("ErrOutOfMemory")
        .expect("errorCode name")
        .0
        .parse::<u64>()
        .expect("out of memory code");
    if align == 0 || !align.is_power_of_two() {
        return Err(invalid_argument);
    }
    let size = size.max(1);
    let payload_base = block_base
        .checked_add(ARENA_BLOCK_HEADER_SIZE as u64)
        .ok_or(out_of_memory)?;
    let raw = payload_base
        .checked_add(current_offset)
        .ok_or(out_of_memory)?;
    let mask = align - 1;
    let aligned = raw
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(out_of_memory)?;
    let end = aligned.checked_add(size).ok_or(out_of_memory)?;
    let used = end.checked_sub(payload_base).ok_or(out_of_memory)?;
    if used > capacity {
        return Err(out_of_memory);
    }
    Ok((aligned, used))
}

/// Executable reference model of the per-arena coalescing free-list, mirroring
/// the integer arithmetic of the emitted `arena_alloc` / `arena_insert_free`
/// assembly so the algorithm can be unit-tested without running native code. The
/// list is kept sorted by `start`; `nodes` holds `(start, size)` pairs.
#[derive(Default, Clone)]
struct FreeListSim {
    nodes: Vec<(u64, u64)>,
}

impl FreeListSim {
    /// `(size, align)` normalization shared by alloc and free: size 0 → 1, then
    /// round up to the 16-byte granule; align is raised to at least 16 so every
    /// chunk stays 16-aligned.
    fn normalize(size: u64, align: u64) -> (u64, u64) {
        let size = size.max(1);
        let size = (size + (ARENA_MIN_CHUNK - 1)) & !(ARENA_MIN_CHUNK - 1);
        let align = align.max(ARENA_MIN_CHUNK);
        (size, align)
    }

    /// Insert a fresh OS block's usable region (or any chunk) and coalesce.
    fn insert_free(&mut self, ptr: u64, size: u64) {
        let (size, _) = Self::normalize(size, ARENA_MIN_CHUNK);
        // address-ordered insertion slot
        let slot = self.nodes.partition_point(|(start, _)| *start < ptr);
        self.nodes.insert(slot, (ptr, size));
        // coalesce with the node before and after, if adjacent
        if slot + 1 < self.nodes.len() {
            let (nstart, nsize) = self.nodes[slot + 1];
            if ptr + size == nstart {
                self.nodes[slot].1 += nsize;
                self.nodes.remove(slot + 1);
            }
        }
        if slot > 0 {
            let (pstart, psize) = self.nodes[slot - 1];
            if pstart + psize == self.nodes[slot].0 {
                self.nodes[slot - 1].1 += self.nodes[slot].1;
                self.nodes.remove(slot);
            }
        }
    }

    /// First-fit + split. Returns the aligned pointer, or `None` if nothing fits
    /// (the caller would map a new block and retry).
    fn alloc(&mut self, size: u64, align: u64) -> Option<u64> {
        let (size, align) = Self::normalize(size, align);
        let mask = align - 1;
        for index in 0..self.nodes.len() {
            let (start, csize) = self.nodes[index];
            let aligned = (start + mask) & !mask;
            if aligned + size <= start + csize {
                let end = start + csize;
                let front = aligned - start;
                let tail = end - (aligned + size);
                self.nodes.remove(index);
                let mut insert_at = index;
                if front > 0 {
                    self.nodes.insert(insert_at, (start, front));
                    insert_at += 1;
                }
                if tail > 0 {
                    self.nodes.insert(insert_at, (aligned + size, tail));
                }
                return Some(aligned);
            }
        }
        None
    }

    fn free(&mut self, ptr: u64, size: u64) {
        let (size, _) = Self::normalize(size, ARENA_MIN_CHUNK);
        self.insert_free(ptr, size);
    }

    /// Total free bytes and the list length — used to assert coalescing keeps the
    /// list short and never loses or duplicates bytes.
    fn free_bytes(&self) -> u64 {
        self.nodes.iter().map(|(_, size)| *size).sum()
    }

    /// Invariant: strictly ascending, non-overlapping, never two coalescable
    /// (address-adjacent) neighbors left un-merged.
    fn assert_invariants(&self) {
        for window in self.nodes.windows(2) {
            let (astart, asize) = window[0];
            let (bstart, _) = window[1];
            assert!(astart < bstart, "free list not ascending: {:?}", self.nodes);
            assert!(
                astart + asize <= bstart,
                "free list overlaps: {:?}",
                self.nodes
            );
            assert!(
                astart + asize != bstart,
                "adjacent free chunks left un-coalesced: {:?}",
                self.nodes
            );
        }
    }
}
