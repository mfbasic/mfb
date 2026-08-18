//! `crypto::generateP256Raw` — descriptor entry + authored docs.
//!
//! A NATIVE member: an internal raw NIST P-256 key generator. Its `Body::native`
//! OS-seam slots point at [`super::native::lower_crypto_ec`], the shared elliptic-curve
//! lowering. This member is internal glue for `crypto::generateP256`; there is no
//! dedicated man page, so the docs below are authored here.

use super::{bytes, Body, Implementation, RegistryFunction};

const INTRO: &str =
    r#"Generate a raw NIST P-256 private key. Internal glue for `crypto::generateP256`."#;
const DESC: &str = r#"`crypto::generateP256Raw` generates a fresh NIST P-256 key and returns its raw
private key in the wire form `0x04 || X || Y || K`: the 65-byte uncompressed
public point (`0x04` prefix plus the two 32-byte field elements `X` and `Y`)
followed by the 32-byte secret scalar `K`.

This is an **internal** raw key generator. It exists as glue for the public
`crypto::generateP256`, which splits this raw buffer into the `publicKey` and
`privateKey` fields of a `crypto::KeyPair`. Prefer `crypto::generateP256` in
application code."#;
const EX: &str = r#"This is an internal helper; use `crypto::generateP256` instead:

```
IMPORT crypto

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateP256()
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "generateP256Raw",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: bytes(),
            errors: vec!["ErrInvalidArgument"],
            body: Body::native(
                Some(super::native::lower_crypto_ec),
                Some(super::native::lower_crypto_ec),
                None,
            ),
        }],
    });
}
