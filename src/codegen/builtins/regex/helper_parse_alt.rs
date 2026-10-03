//! `__regex_parseAlt` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_parseAlt(pat AS List OF String, n AS Integer, i AS Integer, flags AS __regex_Flags, g AS Integer, names AS Map OF String TO Integer, depth AS Integer) AS __regex_Parse
  ' bug-423: one native frame per group-nesting level with no guard overflowed
  ' the stack on a pathologically deep pattern. Every group descent routes
  ' through here (parseParen -> parseAlt), so capping depth at this single point
  ' turns the crash into an ordinary catchable failure well before the native
  ' stack runs out.
  IF depth > __REGEX_PARSE_DEPTH_LIMIT THEN
    FAIL error(77050003, "regex: pattern nested too deeply")
  END IF
  LET first AS __regex_Parse = __regex_parseConcat(pat, n, i, flags, g, names, depth)
  ' bug-773: with no `|` the one branch IS the result -- returned as it is, not
  ' copied into a one-option list and out again.
  IF first.nxt >= n OR collections::get(pat, first.nxt) <> "|" THEN
    RETURN first
  END IF
  MUT idx AS Integer = first.nxt
  MUT gg AS Integer = first.groups
  MUT nm AS Map OF String TO Integer = first.names
  MUT opts AS List OF __regex_Node = [first.node]
  WHILE idx < n AND collections::get(pat, idx) = "|"
    idx = idx + 1
    LET nextc AS __regex_Parse = __regex_parseConcat(pat, n, idx, flags, gg, nm, depth)
    idx = nextc.nxt
    gg = nextc.groups
    nm = nextc.names
    opts = collections::append(opts, nextc.node)
  END WHILE
  LET node AS __regex_Node = __regex_Alt[opts]
  RETURN __regex_Parse[node, idx, gg, nm]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_parseAlt", BODY));
}
