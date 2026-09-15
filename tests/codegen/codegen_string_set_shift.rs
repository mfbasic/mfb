//! plan-121-F / bug-627: a length-changing `set` on a variable-width list element
//! is resolved inside the block, in O(1) amortized, instead of rebuilding it.
//!
//! plan-121-F replaced the `removeAt` + `insert` rebuild with an in-block shift of
//! every byte after the written element. bug-627 measured that shift as the
//! remaining cost: O(N) per write, so widening every element of a list front to
//! back was O(N²) (0.58 s → 6.69 s for 25,000 → 100,000 elements). A shorter
//! payload is now overwritten where it lies, a longer one is written where it lies
//! when it is the last payload in the data region, and otherwise at the data tail,
//! leaving its old span as dead bytes. A tail write that overflows `dataCapacity`
//! repacks the live payloads into a geometrically larger block.
//!
//! ## Why codegen inspection and not just the runtime fixture
//!
//! `p121f-string-set-readback-rt` proves the result is *correct*, and it did so
//! before both changes too — the rebuild and the shift were correct, merely slow.
//! So a green fixture cannot distinguish which path ran. Only the emitted code
//! can, which is what this file reads (`rt_list_set_widening_linear` times it).
//!
//! The pairing rule from plan-121-C/D applies: each positive is matched by a
//! shape that must NOT take the path, because a fast path that fires too widely
//! is a miscompile while one that fires too narrowly is merely slow.

#[path = "../common/mod.rs"]
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
fn a_variable_width_set_emits_the_in_block_resize() {
    let plan = ncode("p121f_shift", STRING_SET);
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_resize") >= 1,
        "plan-121-F: a `set` on a `List OF String` must emit the in-block length \
         change. Without it a length-changing write rebuilds the whole list via \
         removeAt + insert -- three allocations and two full copies per call, \
         which is why it measured O(N^1.6)."
    );
}

#[test]
fn the_resize_emits_every_case_and_the_overflow_repack() {
    let plan = ncode("p121f_shift_dirs", STRING_SET);
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_narrow") >= 1,
        "a shorter payload must be overwritten where it lies."
    );
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_extend") >= 1,
        "a longer payload that is the last in the data region must grow where it lies."
    );
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_relocate") >= 1,
        "any other longer payload must be written at the data tail."
    );
    // The overflow must grow GEOMETRICALLY: a tight rebuild guarantees the next
    // widening overflows too (plan-121-F Correction F1). It must also repack, or
    // the dead bytes the relocations leave accumulate without bound.
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_grow_dcap") >= 1,
        "plan-121-F: a `dataCapacity` overflow must take the GEOMETRIC data grow, \
         not the tight rebuild."
    );
    assert!(
        label_count(&plan, "_mfb_fn_mutate", "set_repack") >= 1,
        "bug-627: the overflow grow must repack the live payloads, dropping the dead \
         bytes relocated writes leave behind."
    );
}

/// bug-627: nothing on the `set` path may move the bytes or the entry offsets of
/// the OTHER elements. Each of those is an O(N) pass per write.
#[test]
fn a_variable_width_set_does_not_shift_the_other_elements() {
    let plan = ncode("b627_no_shift", STRING_SET);
    for needle in [
        "set_inplace_widen",
        "set_inplace_widenfix",
        "set_inplace_narrowfix",
        "set_inplace_shift",
    ] {
        assert_eq!(
            label_count(&plan, "_mfb_fn_mutate", needle),
            0,
            "bug-627: `{needle}` shifts the payloads or offsets after the written \
             element -- O(N) per write, O(N²) over a loop."
        );
    }
}

/// Must-not-change. A fixed-width element is always replaced by one of exactly
/// its own size, so the same-size overwrite always applies and there is no span
/// to resize. Emitting the resize here would be dead code at best; taking it
/// would read an entry table that does not exist.
#[test]
fn a_fixed_width_set_does_not_emit_the_resize() {
    let plan = ncode("p121f_shift_fixed", INTEGER_SET);
    assert_eq!(
        label_count(&plan, "_mfb_fn_mutate", "set_inplace_relocate"),
        0,
        "plan-121-F: a `List OF Integer` `set` is entry-free and always \
         same-size, so it must not emit the variable-width resize."
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_mutate", "set_grow_dcap"),
        0,
        "plan-121-F: a fixed-width `set` never changes `dataLength`, so it has \
         no overflow to grow for."
    );
}
