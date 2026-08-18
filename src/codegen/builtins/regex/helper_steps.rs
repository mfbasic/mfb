//! `__regex_steps` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-315: is this node a "simple" one-scalar matcher -- Lit, Any or Class? Such
' a child consumes exactly one scalar, sets no captures and needs no continuation,
' so a greedy repeat over it can be consumed with a LOOP instead of one native
' stack frame per iteration. That recursion is what made `^a*$` SIGSEGV somewhere
' between 800 and 1000 scalars.
' bug-315: a global backtracking budget. The matcher is a pure backtracker with
' no memoization, so a nested/ambiguous quantifier such as `^(a+)+$` explores
' exponentially many input partitions -- `aaaa...X` at 24 scalars already ran for
' minutes. The engine accepts untrusted patterns AND untrusted text, so that is a
' denial-of-service vector, not just a slow case.
'
' A budget cannot be threaded through the immutable continuation state: a failed
' branch's work would be forgotten on backtrack, which is precisely the work that
' needs counting. It has to be module-level and monotonic, reset once per search.
MUT __regex_steps AS Integer = 0"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_steps", BODY));
}
