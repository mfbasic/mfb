//! The optimizer pipeline and its `-O` dial (plan-100).
//!
//! The pass catalog lives in `planning/optimizations.md`; each row carries a
//! *scale level* (0-6) describing how much shape distortion it is allowed to
//! introduce. This module owns the process-wide selected level and the
//! [`level_enabled`] predicate each pass self-guards with, so one `-ON` lights
//! up rows across every seam (level is orthogonal to pipeline stage).
//!
//! The pipeline the dial gates:
//!
//! ```text
//! AST -> HIR -> IR -> NIR -> gated[Opt1(NIR)] -> Plan1(storage/symbols) -> MIR
//!     -> gated[ Plan2(CFG + SSA/def-use) -> Opt2(MIR) -> Out-of-SSA(MIR) ]
//!     -> FMA-combine (Level 0) -> regalloc -> gated[ machine peepholes ] -> code
//! ```
//!
//! [`opt1`] is the `NirModule -> NirModule` seam; [`opt2`] holds the MIR/machine
//! passes plus the reserved between-selection-and-regalloc MIR seam. The **two**
//! Level-1 rows that ship today (`forward_stores_to_loads`, `remove_fp_shuttles`)
//! live in [`opt2`] and are gated by [`level_enabled`].
//!
//! **The dial's contract: `-O0`..`-O5` change the emitted code, never the
//! observable results.** Only a pass that is behavior-preserving *by
//! construction* may ride the dial. A pass that can change a value or a trap is
//! **Level 0** if the language requires it, or **Level 6** if the user must opt
//! in — never in between. `fuse_scalar_fma` is the worked example: contraction
//! rounds once instead of twice, so it is Level 0 and lives ungated in
//! `crate::codegen::compiler::opt::fma_fusion` (plan-100 Corrections).

use std::sync::OnceLock;

pub(crate) mod opt1;
pub(crate) mod opt2;

/// The optimization scale level requested on the command line by `-O<N>`.
///
/// Levels `0..=5` are the cumulative *risk dial*: each step permits more shape
/// distortion at **preserved observable behavior**. Level `0` is below the dial:
/// always on, not gated in code (a `level_enabled(0)` guard could never fail),
/// for lowering the language *requires* -- FMA contraction, most-negative-literal
/// folding, branch relaxation, base selection, register allocation. Level `6` is
/// orthogonal --
/// the explicit opt-in for semantic-relaxing passes (fast-math, trap-order
/// relaxation) -- and is never reached by escalating the dial, not even at
/// `-O5`/"max".
///
/// The default here is deliberately **non-zero**: today's shipping codegen
/// already runs the Level-1 passes, so `-O1` is the default and `-O0` is the
/// new "optimizations off" path.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct OptLevel(pub(crate) u8);

impl Default for OptLevel {
    fn default() -> Self {
        OptLevel(1)
    }
}

/// Levels accepted by `-O`, for the error message on an unknown value.
///
/// The [`OptLevel`] type spans `0..=6` so later rows slot in without a type
/// change, but the parser accepts only the levels that actually select
/// something today. Every landed *dial* row is Level 1, so `-O1`..`-O5` would be
/// indistinguishable; `2..=5` open up as rows land, and `6` additionally
/// requires an explicit request (plan-100 Non-goals). Level 0 is accepted
/// because it means "dial passes off", not because it selects a row -- Level-0
/// rows run unconditionally and are never gated.
pub(crate) fn available_levels() -> &'static [&'static str] {
    &["0", "1"]
}

/// Parse an `-O` / `--optimize` value, listing the available levels on an
/// unknown one.
pub(crate) fn parse_level(value: &str) -> Result<OptLevel, String> {
    match value {
        "0" => Ok(OptLevel(0)),
        "1" => Ok(OptLevel(1)),
        other => Err(format!(
            "unknown -O level `{other}` (available: {})",
            available_levels().join(", ")
        )),
    }
}

static SELECTED: OnceLock<OptLevel> = OnceLock::new();

// The dial is a write-once process global, which is right for a compiler run
// but wrong for a test binary: every `#[test]` shares one process, so the first
// `set_opt_level` would decide the level for all the others. Tests instead push
// a *thread*-local level for the duration of a closure (`cargo test` gives each
// test its own thread), leaving `SELECTED` alone. Load-bearing: without it the
// `-O0`-disables-the-pass tests below and in `opt2` cannot exist at all, since
// setting the real global would silently disable the passes for every other
// codegen unit test in the binary.
#[cfg(test)]
thread_local! {
    static TEST_LEVEL: std::cell::Cell<Option<OptLevel>> = const { std::cell::Cell::new(None) };
}

/// Run `body` with the dial pinned to `level` on this thread only.
#[cfg(test)]
pub(crate) fn with_opt_level<T>(level: OptLevel, body: impl FnOnce() -> T) -> T {
    let previous = TEST_LEVEL.with(|slot| slot.replace(Some(level)));
    let result = body();
    TEST_LEVEL.with(|slot| slot.set(previous));
    result
}

/// Record the process-wide optimization level chosen on the command line. May
/// be called at most once per process; ignored if already set.
pub(crate) fn set_opt_level(level: OptLevel) {
    let _ = SELECTED.set(level);
}

/// The active optimization level, defaulting to [`OptLevel`]'s `1` -- the level
/// at which the two shipping Level-1 passes run, i.e. today's exact codegen.
pub(crate) fn active_opt_level() -> OptLevel {
    #[cfg(test)]
    if let Some(level) = TEST_LEVEL.with(|slot| slot.get()) {
        return level;
    }
    *SELECTED.get().unwrap_or(&OptLevel(1))
}

/// Whether a catalog row tagged `row_level` runs under the active dial.
///
/// This is the per-row seam filter every gated pass self-guards with at its
/// function entry, so the guard covers all call sites and travels with the
/// pass. Level is orthogonal to stage: one `-ON` lights up rows in Opt1, Opt2
/// and the post-regalloc machine peepholes alike.
pub(crate) fn level_enabled(row_level: u8) -> bool {
    row_level <= active_opt_level().0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_level_is_one() {
        assert_eq!(OptLevel::default(), OptLevel(1));
    }

    #[test]
    fn parse_level_accepts_zero_and_one() {
        assert_eq!(parse_level("0"), Ok(OptLevel(0)));
        assert_eq!(parse_level("1"), Ok(OptLevel(1)));
    }

    #[test]
    fn level_enabled_is_row_level_at_most_active() {
        with_opt_level(OptLevel(0), || {
            assert!(level_enabled(0));
            assert!(!level_enabled(1));
        });
        with_opt_level(OptLevel(1), || {
            assert!(level_enabled(0));
            assert!(level_enabled(1));
            assert!(!level_enabled(2));
        });
        // Level 6 is the semantic-relaxation opt-in: reaching the top of the
        // numeric dial must never light it up.
        with_opt_level(OptLevel(5), || {
            assert!(level_enabled(5));
            assert!(!level_enabled(6));
        });
    }

    /// The override is scoped: leaving `with_opt_level` restores what the
    /// thread saw before, so one test cannot leak a level into another.
    #[test]
    fn with_opt_level_restores_the_previous_level() {
        let before = active_opt_level();
        with_opt_level(OptLevel(0), || {
            assert_eq!(active_opt_level(), OptLevel(0));
            with_opt_level(OptLevel(1), || assert_eq!(active_opt_level(), OptLevel(1)));
            assert_eq!(active_opt_level(), OptLevel(0));
        });
        assert_eq!(active_opt_level(), before);
    }

    #[test]
    fn parse_level_rejects_unlanded_levels() {
        for bogus in ["2", "5", "6", "x", "", "-1"] {
            let err = parse_level(bogus).expect_err("level should not parse yet");
            assert!(err.contains("available: 0, 1"), "{err}");
        }
    }
}
