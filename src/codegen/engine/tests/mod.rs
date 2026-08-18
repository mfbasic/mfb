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
