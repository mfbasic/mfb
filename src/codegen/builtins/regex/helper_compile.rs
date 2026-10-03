//! `__regex_compile` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-773: the last few compiled programs, keyed by pattern text, so a program that
' calls regex::* with the same pattern compiles it once. Only a SUCCESSFUL compile is
' kept -- an invalid pattern re-parses and fails on every call. At most
' __REGEX_CACHE_SLOTS entries, replaced round-robin. Module-level, so per thread: a
' worker's globals live in its own arena (mfb spec threading, isolation).
MUT __regex_cacheKeys AS List OF String = []
MUT __regex_cacheProgs AS List OF __regex_Program = []
MUT __regex_cacheNext AS Integer = 0
LET __REGEX_CACHE_SLOTS AS Integer = 8

FUNC __regex_compile(pattern AS String) AS __regex_Program
  FOR slot = 0 TO len(__regex_cacheKeys) - 1
    IF collections::get(__regex_cacheKeys, slot) = pattern THEN
      RETURN collections::get(__regex_cacheProgs, slot)
    END IF
  NEXT
  LET prog AS __regex_Program = __regex_compileUncached(pattern)
  IF len(__regex_cacheKeys) < __REGEX_CACHE_SLOTS THEN
    __regex_cacheKeys = collections::append(__regex_cacheKeys, pattern)
    __regex_cacheProgs = collections::append(__regex_cacheProgs, prog)
  ELSE
    __regex_cacheKeys = collections::set(__regex_cacheKeys, __regex_cacheNext, pattern)
    __regex_cacheProgs = collections::set(__regex_cacheProgs, __regex_cacheNext, prog)
    __regex_cacheNext = (__regex_cacheNext + 1) MOD __REGEX_CACHE_SLOTS
  END IF
  RETURN prog
END FUNC

FUNC __regex_compileUncached(pattern AS String) AS __regex_Program
  LET pat AS List OF String = __regex_toScalars(pattern)
  LET flags0 AS __regex_Flags = __regex_Flags[FALSE, FALSE, FALSE, FALSE, FALSE]
  LET names0 AS Map OF String TO Integer = Map OF String TO Integer {}
  LET parsed AS __regex_Parse = __regex_parseAlt(pat, len(pat), 0, flags0, 0, names0, 0)
  IF parsed.nxt <> len(pat) THEN
    FAIL error(77050003, "invalid regex")
  END IF
  ' bug-773: `__regex_flatten` attaches the first-scalar set (`__regex_firstSetKids`).
  RETURN __regex_flatten(parsed.node, parsed.groups, parsed.names)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_compile", BODY));
}
