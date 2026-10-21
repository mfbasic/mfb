//! Shared lookup for the suites that assert which lowering a program received.
//!
//! A label is the stable name of a code shape (`inplace_replace_count_*`), but its
//! trailing serial churns whenever an unrelated construct is added ahead of it, so
//! the suites compare STEMS: the label with its serial number dropped.

use std::collections::BTreeSet;

use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// `source` lowered for linux-x86_64 (named explicitly: CI measures there), shared
/// across the suite so an assertion per property does not recompile the program.
pub(super) fn plan(source: &str) -> &'static NativeCodePlan {
    code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
}

/// The label stems function `name` of `source` defines.
pub(super) fn stems_in(source: &str, name: &str) -> BTreeSet<String> {
    Stream::of(code_function(plan(source), name))
        .labels()
        .into_iter()
        .map(|(_, label)| {
            label
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .trim_end_matches('_')
                .to_string()
        })
        .collect()
}

/// The label stems `main` defines.
pub(super) fn stems(source: &str) -> BTreeSet<String> {
    stems_in(source, "main")
}

/// True when some stem starts with `family`.
pub(super) fn has(stems: &BTreeSet<String>, family: &str) -> bool {
    stems.iter().any(|stem| stem.starts_with(family))
}
