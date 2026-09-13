//! `mfb build --debug` / `mfb test --debug` (plan-130).
//!
//! A `--debug` build carries a measurement report that `_mfb_shutdown` writes to
//! stderr as its last act. This module owns the build-side switch; the report
//! helper and the feature registry that plugs measurements into it build on it.
//!
//! The switch travels on [`NirModule`](crate::target::shared::nir::NirModule),
//! never as a process global: it is read only in codegen, and a global would
//! leak across the nested builds of source dependencies.

/// Per-build debug-report options, carried on `NirModule`.
///
/// A struct rather than a bare `bool` so a later per-feature selection
/// (`--debug=arena,perf`) is a new field, not another signature change across
/// every lowering entry point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DebugOptions {
    /// Whether this build carries the debug report at all.
    pub(crate) enabled: bool,
}

impl DebugOptions {
    /// A normal build: no report, no debug-only code or data.
    ///
    /// Test-only: production builds construct the value from the parsed
    /// `--debug` flag (`BuildOptions::debug`), so only fixtures that lower a
    /// module outside the CLI need a named "off".
    #[cfg(test)]
    pub(crate) const OFF: DebugOptions = DebugOptions { enabled: false };
}
