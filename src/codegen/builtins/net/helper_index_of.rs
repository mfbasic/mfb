//! `__net_indexOf` — shared private helper for the `net` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-339 B2: __net_indexOf / __net_slice / __net_defaultPort are byte-for-byte
' duplicated in http_package.mfb. The duplication is LANGUAGE-MANDATED and left in
' place deliberately: an injected built-in package source is one file whose FUNCs
' are file-local (PACKAGE visibility is not valid in an executable, and there are
' zero source-EXPORT public functions in any built-in package — every public
' member is native-declared in the .rs front end). So `http`, even though it
' `IMPORT net`s, cannot reach `net`'s private `__net_*` helpers. The only ways to
' hold ONE copy are (a) promote them to public `strings::` API — a user-facing
' surface addition (new native-declared, source-implemented functions wired across
' the resolver, the three augmentation chains, man pages and the spec), which the
' doc recommends but which is a feature, not this cleanup; or (b) restructure the
' built-in injection model to inject a shared source once — the same risky change
' B1 needs. Note also __net_slice is NOT strings::mid: it clamps a reversed range
' to "" where strings::mid raises ErrIndexOutOfRange, so it is a safe-slice, not a
' redundant alias.
'
' Grapheme index of `needle` in `s` at or after `start`, or -1 when absent.
' `strings::find` fails ErrNotFound on a miss (and, being inline-expanded, cannot
' be wrapped in an inline TRAP), so presence is checked first with `contains`.
FUNC __net_indexOf(s AS String, needle AS String, start AS Integer) AS Integer
  LET tail AS String = __net_slice(s, start, len(s))
  IF strings::contains(tail, needle) = FALSE THEN
    RETURN -1
  END IF
  RETURN strings::find(s, needle, start)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("net_indexOf", BODY));
}
