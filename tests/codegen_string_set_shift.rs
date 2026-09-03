//! plan-121-F: a length-changing `set` on a variable-width list element shifts
//! inside the block instead of rebuilding it.
//!
//! ## Why codegen inspection and not just the runtime fixture
//!
//! `p121f-string-set-readback-rt` proves the result is *correct*, and it did so
//! before this change too — the old rebuild path was correct, merely slow. So a
//! green fixture cannot distinguish "the shift path ran" from "the rebuild path
//! ran". Only the emitted code can, which is what this file reads.
//!
//! The pairing rule from plan-121-C/D applies: each positive is matched by a
//! shape that must NOT take the path, because a fast path that fires too widely
//! is a miscompile while one that fires too narrowly is merely slow.

mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

/// Count `label` instructions whose name contains `needle` in the named function.
fn label_count(plan: &Value, symbol: &str, needle: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| {
            instr["op"].as_str() == Some("label")
                && instr["name"]
                    .as_str()
                    .is_some_and(|name| name.contains(needle))
        })
        .count()
}

fn function<'a>(plan: &'a Value, symbol: &str) -> &'a Value {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))
}

fn ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let plan = common::build_ncode(&project, TARGET, name);
    let _ = std::fs::remove_dir_all(&project);
    plan
}

const STRING_SET: &str = "IMPORT collections\n\
     FUNC mutate(v AS String, i AS Integer) AS Integer\n\
    \x20 MUT xs AS List OF String = [\"a\", \"bb\", \"ccc\"]\n\
    \x20 xs = collections::set(xs, i, v)\n\
    \x20 RETURN len(xs)\n\
     END FUNC\n\
     FUNC main AS Integer\n\
    \x20 RETURN mutate(\"z\", 0)\n\
     END FUNC\n";

const INTEGER_SET: &str = "IMPORT collections\n\
     FUNC mutate(v AS Integer, i AS Integer) AS Integer\n\
    \x20 MUT xs AS List OF Integer = [1, 2, 3]\n\
    \x20 xs = collections::set(xs, i, v)\n\
    \x20 RETURN len(xs)\n\
     END FUNC\n\
     FUNC main AS Integer\n\
    \x20 RETURN mutate(9, 0)\n\
     END FUNC\n";

#[test]
fn a_variable_width_set_emits_the_in_block_shift() {
    let plan = ncode("p121f_shift", STRING_SET);
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_shift") >= 1,
        "plan-121-F: a `set` on a `List OF String` must emit the in-block shift \
         path. Without it a length-changing write rebuilds the whole list via \
         removeAt + insert -- three allocations and two full copies per call, \
         which is why it measured O(N^1.6) rather than the O(N) a data shift costs."
    );
}

#[test]
fn the_shift_emits_both_directions_and_the_overflow_grow() {
    let plan = ncode("p121f_shift_dirs", STRING_SET);
    // Widening and narrowing are NOT the same code: the tail moves up in one and
    // down in the other, so one needs a backward copy and the other a forward
    // one. A forward copy used for the widening case smears the first tail bytes
    // over the region whenever the shift distance is less than the tail length --
    // and still looks correct on a 1-2 element list.
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_widen") >= 1,
        "plan-121-F: the widening direction must emit its BACKWARD copy."
    );
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_narrow") >= 1,
        "plan-121-F: the narrowing direction must emit its FORWARD copy."
    );
    // Both offset fixups must be present. Every payload after the written one
    // moves, so every one of their entries must move with it; a missing fixup
    // reads correctly up to `index` and returns garbage after.
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_widenfix") >= 1,
        "plan-121-F: widening must fix up the entry offsets that moved up."
    );
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_narrowfix") >= 1,
        "plan-121-F: narrowing must fix up the entry offsets that moved down."
    );
    // The overflow branch is the half the measurement caught: without a
    // GEOMETRIC grow, every widening overflows, rebuilds tight, and overflows
    // again on the next call -- leaving the widening cost unchanged.
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_grow_dcap") >= 1,
        "plan-121-F: a `dataCapacity` overflow must take the GEOMETRIC data grow, \
         not the tight rebuild. A tight rebuild guarantees the next widening \
         overflows too, which is exactly the behaviour that made an in-block \
         shift alone measure no faster (Correction F1)."
    );
}

/// Must-not-change. A fixed-width element is always replaced by one of exactly
/// its own size, so the same-size overwrite always applies and there is no span
/// to widen or narrow. Emitting the shift here would be dead code at best; taking
/// it would be a shift computed from an entry table that does not exist.
#[test]
fn a_fixed_width_set_does_not_emit_the_shift() {
    let plan = ncode("p121f_shift_fixed", INTEGER_SET);
    assert_eq!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_widen"),
        0,
        "plan-121-F: a `List OF Integer` `set` is entry-free and always \
         same-size, so it must not emit the variable-width shift."
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_mutate", "set_grow_dcap"),
        0,
        "plan-121-F: a fixed-width `set` never changes `dataLength`, so it has \
         no overflow to grow for."
    );
}
