//! `__REGEX_PARSE_DEPTH_LIMIT` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-423: the recursive-descent parser had no depth cap, so a deeply-nested-group
' pattern (`((((...`) recursed once per group and overflowed the native stack
' during *compile*, killing the process with an uncatchable SIGSEGV before any
' matching happened -- bug-315 had guarded only the matcher. This is the parser's
' own ceiling on group-nesting depth. It is LOWER than __REGEX_DEPTH_LIMIT on
' purpose: each nesting level costs three native frames (parseAlt -> parseConcat
' -> parseParen -> parseAlt), so the same ~600-frame stack budget the matcher
' proved safe corresponds to ~200 nesting levels here. Measured: the produced
' executable's stack is exhausted around 350 nested `(` (N<=300 fails cleanly,
' N>=400 SIGSEGVs), so 200 rejects the pathological cases with margin while
' leaving any realistic pattern -- which nests only a handful deep -- untouched.
LET __REGEX_PARSE_DEPTH_LIMIT AS Integer = 200"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseDepthLimit", BODY));
}
