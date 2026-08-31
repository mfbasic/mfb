//! `__http_readTls` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_readTls(RES t AS tls::Socket, n AS Integer) AS __http_PumpRead
  MUT chunk AS List OF Byte = []
  MUT closed AS Boolean = FALSE
  MUT err AS Integer = 0
  chunk = tls::read(t, n) TRAP(e)
    IF e.code = errorCode::ErrConnectionClosed THEN
      closed = TRUE
      RECOVER []
    END IF
    err = e.code
    RECOVER []
  END TRAP
  IF len(chunk) = 0 THEN
    closed = TRUE
  END IF
  RETURN __http_PumpRead[chunk, closed, err]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_readTls", BODY));
}
