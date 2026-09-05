//! The corpus, lowered at every optimization level.
//!
//! `src/optimizer/**` is 36 files and ~1,250 lines below the floor, and the
//! reason is structural rather than a missing test: `optimizer::active_opt_level`
//! defaults to `-O1`, so every program any test lowers runs the Level-1 rows and
//! nothing above them. The `-O2` and `-O3` catalog rows — the loop plans, the
//! range plans, jump threading, tail duplication — are reached only by a build
//! that asked for them, and no in-process test ever did.
//!
//! The dial is a *thread*-local under `cfg(test)` (deliberately: a write-once
//! process global would let the first test decide the level for every other),
//! and this harness lowers on a thread it spawns — so `code_for_src_at` pushes
//! the level INSIDE that thread. Without that the level is silently ignored and
//! this whole file would measure `-O1` four times.
//!
//! The contract is the one an optimizer has to keep and the one its own per-pass
//! tests cannot state over a whole corpus: **raising the level does not change
//! what the program IS.** Concretely, at every level the program still lowers,
//! still validates — `code::lower_module` rejects a branch to a label nothing
//! defines and a relocation against a symbol nothing emits, which is exactly
//! what a pass that rewrites a CFG gets wrong — and still contains its own
//! functions with non-empty bodies. A pass that drops a live function, empties a
//! body, or leaves a dangling edge is a miscompile, and each of those is caught
//! here rather than by a golden that only runs at one level.

use std::collections::BTreeSet;

use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{code_for_src_at, fixture_src, CodeTarget};

/// Every level the dial accepts.
const LEVELS: [u8; 4] = [0, 1, 2, 3];

/// Programs chosen for the SHAPES the higher rows act on: counted loops, nested
/// loops with an invariant, index arithmetic the range plans can bound, long
/// branch chains for jump threading, and a join block worth tail-duplicating.
const SHAPES: &[&str] = &[
    "bounds-elim-rt",
    "control-flow-behavior",
    "control-flow-if",
    "bug118_match_guard_helper",
    "bug361_match_oneof_literals",
    "bulk-append-inplace",
    "mut-append-grow",
    "inplace-grow-free",
    "list-ops-codegen-rt",
    "nested-fixed-list-rt",
    "map-set-grow-rt",
    "set-algebra-rt",
    "reduce-accumulator-reclaim-rt",
    "json-behavior",
    "json-number-rendering-rt",
    "regex-posix-classes-rt",
    "float-fma-fusion",
    "record-field",
    "user-generic-nested-rt",
    "call-function-value-rt",
    "p121d-state-ops-rt",
    "func-bare-trap-loop-leak-rt",
    "get-borrow-match-rt",
    "recursive-get-then-grow-rt",
];

/// The program's OWN functions with a body — everything the source declared.
///
/// Runtime helpers (`runtime.*`), package bodies (`#pkg_*`) and constructors are
/// excluded because a higher level may legitimately delete one: folding every
/// `&` in a program at -O3 removes the last call to `runtime.stringConcat`, and
/// then the helper is dead. That is the optimizer working. A function the SOURCE
/// declared is a different matter.
fn program_functions(plan: &NativeCodePlan) -> BTreeSet<String> {
    plan.functions
        .iter()
        .filter(|f| f.instructions.len() > 1)
        .map(|f| f.name.clone())
        .filter(|name| {
            !name.starts_with("runtime.")
                && !name.starts_with("construct.")
                && !name.starts_with('#')
                && !name.starts_with("program.")
                && !name.starts_with('_')
        })
        .collect()
}

/// Every level lowers, validates, and keeps the program's own functions.
#[test]
fn the_corpus_survives_every_optimization_level() {
    for fixture in SHAPES {
        let source = fixture_src(fixture);
        let mut baseline: Option<BTreeSet<String>> = None;
        for level in LEVELS {
            let plan = code_for_src_at(
                &source,
                CodeTarget::LinuxX86_64,
                crate::target::NativeBuildMode::Console,
                level,
            )
            .unwrap_or_else(|err| panic!("{fixture} at -O{level}: {err}"));

            let functions = program_functions(&plan);
            assert!(
                functions.contains("main"),
                "{fixture} at -O{level}: the entry function must survive with a \
                 non-empty body"
            );
            match &baseline {
                None => baseline = Some(functions),
                Some(at_zero) => {
                    // The direction that matters is what went MISSING: a pass
                    // that drops a function the source declared, or empties its
                    // body, is a miscompile. (Gaining one is not: an inliner may
                    // split or clone.)
                    let missing: Vec<&String> = at_zero.difference(&functions).collect();
                    assert!(
                        missing.is_empty(),
                        "{fixture}: -O{level} lost {missing:?}, which -O0 emitted \
                         with a body. A pass that drops or empties a live function \
                         is a miscompile, not an optimization"
                    );
                }
            }
        }
    }
}

/// Raising the level actually changes the emitted code.
///
/// The other half, and the reason it is here: a level that never reaches the
/// lowering would pass every assertion above while optimizing nothing. That is
/// not hypothetical — it is exactly what the first draft of this file did,
/// because the dial is thread-local and the lowering runs on a thread the
/// harness spawns. Half the programs must lower differently at `-O3` than at
/// `-O0`, or the level is not arriving.
#[test]
fn raising_the_level_changes_the_emitted_code() {
    let mut changed = 0;
    for fixture in SHAPES {
        let source = fixture_src(fixture);
        let at = |level: u8| {
            code_for_src_at(
                &source,
                CodeTarget::LinuxX86_64,
                crate::target::NativeBuildMode::Console,
                level,
            )
            .unwrap_or_else(|err| panic!("{fixture} at -O{level}: {err}"))
            .functions
            .iter()
            .map(|f| f.instructions.len())
            .sum::<usize>()
        };
        if at(0) != at(3) {
            changed += 1;
        }
    }
    assert!(
        changed * 2 >= SHAPES.len(),
        "only {changed} of {} programs lower differently at -O3 than at -O0; the \
         optimization dial is not reaching the lowering",
        SHAPES.len()
    );
}
