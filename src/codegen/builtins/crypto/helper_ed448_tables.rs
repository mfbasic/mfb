//! `__crypto_ed448OrderTable` / `__crypto_ed448FoldTable` / `__crypto_ed448BaseTable`
//! — shared private helpers for the `crypto` package (registered as one chunk).
//!
//! The Ed448 constants (RFC 8032 §5.2), decoded from hex once at program start
//! into the module globals `__CRYPTO_ED448_L` (the group order `L = 2^446 − c` as
//! 57 little-endian byte limbs), `__CRYPTO_ED448_C` (`c = 2^446 mod L`, 28 byte
//! limbs — the fold constant of `__crypto_ed448ModL`), and `__CRYPTO_ED448_B`
//! (the base point as a projective `(X:Y:Z)` triple of 16-limb field elements).
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Ed448 group order L (RFC 8032 §5.2.1) as 57 little-endian byte limbs.
FUNC __crypto_ed448OrderTable() AS List OF Integer
  RETURN __crypto_bytesToLimbs(encoding::hexDecode("f34458ab92c27823558fc58d72c26c219036d6ae49db4ec4e923ca7cffffffffffffffffffffffffffffffffffffffffffffffffffffff3f00"))
END FUNC
' c = 2^446 mod L = 2^446 - L, the 224-bit fold constant, as 28 byte limbs.
FUNC __crypto_ed448FoldTable() AS List OF Integer
  RETURN __crypto_bytesToLimbs(encoding::hexDecode("0dbba7546d3d87dcaa703a728d3d93de6fc92951b624b13b16dc3583"))
END FUNC
' The edwards448 base point B = (X : Y : 1) (RFC 8032 §5.2.1), field elements from
' their 56-byte little-endian encodings.
FUNC __crypto_ed448BaseTable() AS List OF Integer
  LET bx AS List OF Integer = __crypto_gf448Unpack(encoding::hexDecode("5ec00cc72ba826268e93008be1803b431165b62af71aae1264a4d3a324e36dea67170f477065149eda36bf22a6151d22ed0ded6bc670194f"))
  LET by AS List OF Integer = __crypto_gf448Unpack(encoding::hexDecode("14fa30f25b790898adc8d74e2c13bdfdc4397ce61cffd33ad7c2a0051e9c78874098a36c7373ea4b62c7c9563720768824bcb66e71463f69"))
  RETURN __crypto_ed448Point3(bx, by, __crypto_gf448One())
END FUNC
LET __CRYPTO_ED448_L AS List OF Integer = __crypto_ed448OrderTable()
LET __CRYPTO_ED448_C AS List OF Integer = __crypto_ed448FoldTable()
LET __CRYPTO_ED448_B AS List OF Integer = __crypto_ed448BaseTable()"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("crypto_ed448Tables", BODY));
}
