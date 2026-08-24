//! The optimizer pipeline and its `-O` dial (plan-100).
//!
//! The pass catalog lives in `planning/optimizations.md`; each row carries a
//! *scale level* (0-6) describing how much shape distortion it is allowed to
//! introduce. This module owns the process-wide selected level and the
//! `level_enabled` predicate each pass self-guards with, so one `-ON` lights
//! up rows across every seam (level is orthogonal to pipeline stage).
//!
//! The pipeline the dial gates:
//!
//! ```text
//! AST -> HIR -> IR -> NIR -> gated[Opt1(NIR)] -> Plan1(storage/symbols) -> MIR
//!     -> gated[ Plan2(CFG + SSA/def-use) -> Opt2(MIR) -> Out-of-SSA(MIR) ]
//!     -> gated[ FMA-combine ] -> regalloc -> gated[ machine peepholes ] -> code
//! ```
//!
//! `opt1` is the `NirModule -> NirModule` seam; `opt2` holds the MIR/machine
//! passes plus the reserved between-selection-and-regalloc MIR seam. The three
//! Level-1 rows that ship today (`fuse_scalar_fma`, `forward_stores_to_loads`,
//! `remove_fp_shuttles`) live in `opt2` and are gated at level 1.

use std::sync::OnceLock;

/// The optimization scale level requested on the command line by `-O<N>`.
///
/// Levels `0..=5` are the cumulative *risk dial*: each step permits more shape
/// distortion at **preserved observable behavior**. Level `6` is orthogonal --
/// the explicit opt-in for semantic-relaxing passes (fast-math, trap-order
/// relaxation) -- and is never reached by escalating the dial, not even at
/// `-O5`/"max".
///
/// Unlike [`crate::codegen::engine::regalloc::RegallocKind`], whose default is
/// the first-listed strategy, the default here is deliberately **non-zero**:
/// today's shipping codegen already runs the Level-1 passes, so `-O1` is the
/// default and `-O0` is the new "optimizations off" path.
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
/// something today. Every landed row is Level 1, so `-O1`..`-O5` would be
/// indistinguishable; `2..=5` open up as rows land, and `6` additionally
/// requires an explicit request (plan-100 Non-goals).
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

/// Record the process-wide optimization level chosen on the command line. May
/// be called at most once per process; ignored if already set.
pub(crate) fn set_opt_level(level: OptLevel) {
    let _ = SELECTED.set(level);
}

/// The active optimization level, defaulting to [`OptLevel`]'s `1` -- the level
/// at which the three shipping Level-1 passes run, i.e. today's exact codegen.
pub(crate) fn active_opt_level() -> OptLevel {
    *SELECTED.get().unwrap_or(&OptLevel(1))
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
    fn parse_level_rejects_unlanded_levels() {
        for bogus in ["2", "5", "6", "x", "", "-1"] {
            let err = parse_level(bogus).expect_err("level should not parse yet");
            assert!(err.contains("available: 0, 1"), "{err}");
        }
    }
}
