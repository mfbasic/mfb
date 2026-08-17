//! `__REGEX_STEP_BUDGET` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Ceiling on backtracking steps for one search. Generous enough that no realistic
' pattern reaches it -- a linear scan over a 50 000-scalar subject costs about
' that many steps -- while still bounding the exponential cases to well under a
' second.
LET __REGEX_STEP_BUDGET AS Integer = 2000000"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_stepBudget", BODY));
}
