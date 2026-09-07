//! `__crypto_blake2bSigmaTable` — shared private helper for the `crypto` package.
//!
//! The 12 BLAKE2b message-schedule permutations (RFC 7693 §2.7 SIGMA), flattened
//! to 192 single-byte entries and carried as hex the way the SHA-2 tables are.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT encoding

' The 12 BLAKE2b message-schedule permutations (RFC 7693 SIGMA), 16 entries per
' round, flattened to 192 single-byte entries.
FUNC __crypto_blake2bSigmaTable() AS List OF Byte
  LET s1 AS String = "000102030405060708090a0b0c0d0e0f0e0a0408090f0d06010c00020b0705030b080c0005020f0d0a0e030607010904"
  LET s2 AS String = "070903010d0c0b0e0206050a04000f080900050702040a0f0e010b0c0608030d020c060a000b0803040d07050f0e0109"
  LET s3 AS String = "0c05010f0e0d040a000706030902080b0d0b070e0c01030905000f040806020a060f0e090b0300080c020d0701040a05"
  LET s4 AS String = "0a020804070601050f0b090e030c0d00000102030405060708090a0b0c0d0e0f0e0a0408090f0d06010c00020b070503"
  RETURN encoding::hexDecode(s1 & s2 & s3 & s4)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_blake2bSigmaTable",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
