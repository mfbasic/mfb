//! bug-760 C/D: the register allocator does not evict around every instruction
//! of a hot loop.
//!
//! plan-161-B (committed as "plan-158-B") let long-lived values take
//! caller-saved registers at `-O1` and up (`8b36c06e3`) and promoted every
//! private frame slot to a vreg (`d34fdae20`). `linear_scan.rs:run` assigns
//! first-come with no furthest-end spill choice, so the long-lived values fill
//! the register file and every short temporary takes the "commandeer one"
//! eviction arm: the victim is saved, the scratch used, the victim restored — a
//! `ldr R,[sp,#o]` immediately followed by `str R,[sp,#o]` of the same register
//! and slot around nearly every instruction. `float nbody` ran 7.5× slower,
//! `-O1` `sort` / `find` / `displayWidth` 2–3.7×.
//!
//! The pin is the count of those restore-then-resave pairs in the `-O1` AArch64
//! `.ncode` of the benchmark's own hot functions. The pre-regression allocator
//! (`49f619092`) emits 0 in `nbody`, 2 in `fixedMath`, 11 in the inline-builtin
//! `main` and 0 in the runtime's `arena_alloc`; the regressed one emits 100, 333,
//! 81 and 16. The bounds allow the pre-regression count with a little slack and
//! no more.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;

const TARGET: &str = "macos-aarch64";
const FLOAT_FIXED: &str = include_str!("regalloc_eviction/float_fixed.mfb");
const INLINE_BUILTINS: &str = include_str!("regalloc_eviction/inline_builtins.mfb");

/// Restore-then-resave pairs in `function`: a `sp`-based load into R directly
/// followed by a `sp`-based store of R to the same offset.
fn eviction_pairs(plan: &Value, function: &str) -> usize {
    let instructions = plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["name"].as_str() == Some(function))
        .unwrap_or_else(|| panic!("code plan has no function '{function}'"))["instructions"]
        .as_array()
        .expect("instructions array");
    instructions
        .windows(2)
        .filter(|pair| {
            let (load, store) = (&pair[0], &pair[1]);
            load["op"].as_str().is_some_and(|op| op.starts_with("ldr_"))
                && store["op"].as_str().is_some_and(|op| op.starts_with("str_"))
                && load["base"] == "sp"
                && store["base"] == "sp"
                && load["dst"] == store["src"]
                && load["offset"] == store["offset"]
        })
        .count()
}

fn plan(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let plan = common::build_ncode_opt(&project, TARGET, name, "1");
    let _ = std::fs::remove_dir_all(&project);
    plan
}

fn check(plan: &Value, bounds: &[(&str, usize)], failures: &mut Vec<String>) {
    for &(function, bound) in bounds {
        let pairs = eviction_pairs(plan, function);
        eprintln!("{function}: {pairs} eviction pair(s) (bound {bound})");
        if pairs > bound {
            failures.push(format!("{function}: {pairs} (bound {bound})"));
        }
    }
}

#[test]
fn hot_loops_do_not_evict_around_every_instruction() {
    let mut failures = Vec::new();
    let float_fixed = plan("bug760c_float_fixed", FLOAT_FIXED);
    check(
        &float_fixed,
        &[("nbody", 0), ("fixedMath", 4), ("runtime.arena_alloc", 0)],
        &mut failures,
    );
    let inline = plan("bug760d_inline_builtins", INLINE_BUILTINS);
    check(&inline, &[("main", 22)], &mut failures);
    assert!(
        failures.is_empty(),
        "bug-760 C/D: -O1 register allocation restores and re-saves the same slot back to \
         back (an eviction around one instruction):\n  {}",
        failures.join("\n  ")
    );
}
