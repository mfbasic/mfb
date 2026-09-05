//! `codegen::engine::tests` module wiring.

// Both are test-only: `test_support` is the shared `#[cfg(test)]` platform stub
// (consumed by other packages' `#[cfg(test)]` suites via
// `crate::codegen::engine::tests::*`), and `tests` holds this tier's unit tests.
// Gating them keeps their helpers out of the non-test build.
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
pub(crate) use test_support::*;
#[cfg(test)]
mod tests;

// The per-package builtin codegen suites, declared from HERE rather than from
// `codegen::builtins::mod` on purpose. A `#[cfg(test)] mod` line makes the file
// that carries it report far worse than it is: a `cargo llvm-cov` profile merges
// the executed test binary with the instrumented-but-never-executed plain `mfb`
// binary, and the two inline differently once such a module appears, so lines
// that demonstrably run are reported uncovered. Measured, on the same suites:
// putting the line in `builtins/mod.rs` took that file from 99.51% (610/613) to
// 68.93% (610/885). This file's own path contains `/tests/`, so
// `coverage-common.sh` excludes it and the damage lands nowhere.
#[cfg(test)]
#[path = "../../builtins/tests/mod.rs"]
mod builtins_codegen;
