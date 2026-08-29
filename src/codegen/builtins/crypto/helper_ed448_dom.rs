//! `__crypto_ed448Dom` / `__crypto_ed448Prune` — shared private helpers for the
//! `crypto` package (registered as one chunk).
//!
//! `dom4(0, "")` of RFC 8032 §5.2.2 — the PureEd448 hash prefix `"SigEd448" ‖
//! phflag 0x00 ‖ context length 0x00` (this package signs with the empty
//! context) — and the secret-scalar pruning of §5.2.5: clear the two low bits of
//! byte 0, set bit 7 of byte 55, zero byte 56.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' dom4(0, ""): "SigEd448" || 0x00 (PureEd448) || 0x00 (empty context).
FUNC __crypto_ed448Dom() AS List OF Byte
  MUT dom AS List OF Byte = strings::toBytes("SigEd448")
  dom = collections::append(dom, toByte(0))
  dom = collections::append(dom, toByte(0))
  RETURN dom
END FUNC
' RFC 8032 §5.2.5 pruning of the 57-byte secret scalar.
FUNC __crypto_ed448Prune(h AS List OF Byte) AS List OF Byte
  MUT s AS List OF Byte = h
  s = collections::set(s, 0, toByte(bits::band(toInt(collections::get(s, 0)), 252)))
  s = collections::set(s, 55, toByte(bits::bor(toInt(collections::get(s, 55)), 128)))
  s = collections::set(s, 56, toByte(0))
  RETURN s
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Dom", BODY));
}
