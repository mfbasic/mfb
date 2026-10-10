//! `__http_pathEscapesRoot` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-862: TRUE if the slash-separated relative path `rel` (already
' percent-decoded and stripped of its one leading `/`, see
' __http_respondPath) has a `..` segment that would climb above the join
' point it is about to be joined to `root` at — a LEXICAL check, independent
' of what exists on disk, so it catches a traversal attempt before any
' filesystem probe can be used as a file-existence oracle. Depth tracks how
' many real segments have been descended into; a `..` at depth 0 escapes, one
' at depth > 0 just cancels a prior descent. `.` and empty segments (a
' leading/trailing/doubled `/`) are no-ops.
' bug-983: an ABSOLUTE `rel` escapes too. It is what is left of a `//`-prefixed
' capture after the one-slash strip, and fs::pathJoin restarts at an absolute
' component, discarding `root`; checking only `..` let such a request reach the
' filesystem probe and answer 404 for a missing target, the bug-862 oracle.
FUNC __http_pathEscapesRoot(rel AS String) AS Boolean
  IF strings::startsWith(rel, "/") THEN
    RETURN TRUE
  END IF
  LET segs AS List OF String = strings::split(rel, "/")
  MUT depth AS Integer = 0
  FOR EACH seg IN segs
    IF seg = ".." THEN
      IF depth <= 0 THEN
        RETURN TRUE
      END IF
      depth = depth - 1
    ELSEIF seg <> "" AND seg <> "." THEN
      depth = depth + 1
    END IF
  NEXT
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_pathEscapesRoot", BODY));
}
