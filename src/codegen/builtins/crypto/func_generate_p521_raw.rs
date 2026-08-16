//! `crypto::generateP521Raw` — descriptor entry + authored docs.
//!
//! A NATIVE member: an internal raw NIST P-521 key generator. Its `Body::native`
//! OS-seam slots point at [`super::native::lower_crypto_ec`], the shared elliptic-curve
//! lowering. This member is internal glue for `crypto::generateP521`; there is no
//! dedicated man page, so the docs below are authored here.

use super::{bytes, Body, Implementation, RegistryFunction};

const INTRO: &str =
    r#"Generate a raw NIST P-521 private key. Internal glue for `crypto::generateP521`."#;
const DESC: &str = r#"`crypto::generateP521Raw` generates a fresh NIST P-521 key and returns its raw
private key in the wire form `0x04 || X || Y || K`: the 133-byte uncompressed
public point (`0x04` prefix plus the two 66-byte field elements `X` and `Y`)
followed by the 66-byte secret scalar `K`.

This is an **internal** raw key generator. It exists as glue for the public
`crypto::generateP521`, which splits this raw buffer into the `publicKey` and
`privateKey` fields of a `crypto::KeyPair`. Prefer `crypto::generateP521` in
application code."#;
const EX: &str = r#"This is an internal helper; use `crypto::generateP521` instead:

```
IMPORT crypto

SUB main()
  LET kp AS crypto::KeyPair = crypto::generateP521()
END SUB
```"#;

pub(super) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "generateP521Raw",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
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
