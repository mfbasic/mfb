//! `perf` builtin codegen (timing helpers).

// --- codegen tier imports (migration) ---
pub(crate) mod perf;
#[cfg(test)]
mod tests_codegen;
pub(crate) use perf::*;
