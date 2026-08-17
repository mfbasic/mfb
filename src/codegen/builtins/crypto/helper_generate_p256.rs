//! `__crypto_generateP256` — shared private helper for the `crypto` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' --- NIST EC key generation --------------------------------------------------
' The platform key API (SecKey/EVP_PKEY) is reached through the native raw
' keygen helper, which returns the private bytes as `0x04||X||Y||K`. The public
' key is the leading SEC1 uncompressed point `0x04||X||Y` (1 + 2*fieldLen bytes:
' 65 for P-256, 97 for P-384, 133 for P-521). Both encodings are wire-compatible
' across macOS and Linux. bug-339 B3: the SEC1 prefix copy uses __crypto_truncate
' (native range copy); the former __crypto_bytePrefix was its byte-identical twin.
FUNC __crypto_generateP256() AS KeyPair
  LET priv AS List OF Byte = crypto::generateP256Raw()
  LET pub AS List OF Byte = __crypto_truncate(priv, 65)
  RETURN KeyPair[priv, pub]
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_generateP256", BODY));
}
