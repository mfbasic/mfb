//! `__crypto_argon2idProfile` — shared private helper for the `crypto` package.
//!
//! The `crypto::Argon2Profile` spelling of `crypto::argon2id`. It resolves the
//! profile to concrete `memoryKiB`/`iterations`/`parallelism` and calls
//! [`super::helper_argon2id`]'s body — it is not a second implementation and not a
//! second parameter table, so the two spellings have ONE validation site and ONE
//! fill site between them, and the profile constants are the only retunable part.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent -> `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT crypto

' Resolve an `Argon2Profile` to its cost parameters and hand them to the explicit
' `__crypto_argon2id` body. The profile spelling is a WRAPPER, not a second
' implementation: these three constants are the whole of it, so the two spellings
' cannot drift apart and retuning a profile cannot move the explicit member.
' Minimum is the OWASP Password Storage Cheat Sheet's minimum configuration;
' Recommended is the SECOND RECOMMENDED option of RFC 9106 section 4.
FUNC __crypto_argon2idProfile(password AS List OF Byte, salt AS List OF Byte, profile AS Argon2Profile, length AS Integer) AS List OF Byte
  IF profile = Argon2Profile.Minimum THEN
    RETURN __crypto_argon2id(password, salt, 19456, 2, 1, length)
  END IF
  RETURN __crypto_argon2id(password, salt, 65536, 3, 4, length)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2idProfile",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
