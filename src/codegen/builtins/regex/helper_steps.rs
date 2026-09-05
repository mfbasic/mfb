//! `__regex_steps` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-315: a global backtracking budget. The matcher is a pure backtracker with
' no memoization, so a nested/ambiguous quantifier such as `^(a+)+$` explores
' exponentially many input partitions -- `aaaa...X` at 24 scalars already ran for
' minutes. The engine accepts untrusted patterns AND untrusted text, so that is a
' denial-of-service vector, not just a slow case.
'
' A budget cannot be threaded through the immutable continuation state: a failed
' branch's work would be forgotten on backtrack, which is precisely the work that
' needs counting. It has to be module-level and monotonic, reset once per search.
MUT __regex_steps AS Integer = 0
' bug-510 (DEC-02): the same count for the whole public call. `__regex_steps` is
' reset per search, and `findAll`/`replace` run one search per match, so a pattern
' that stayed just under the per-search budget on every match cost matches x budget
' -- 368 bytes of subject bought 17 s. `__regex_makeCtx` arms the call-wide budget
' from the subject length; `__regex_run` charges every node visit to both.
MUT __regex_callSteps AS Integer = 0
MUT __regex_callBudget AS Integer = 0"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_steps", BODY));
}
