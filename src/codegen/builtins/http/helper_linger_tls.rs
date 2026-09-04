//! `__http_lingerTls` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-507: a bounded lingering close after an EARLY rejection (408/413/431 —
' the request was refused before the client finished sending it). Closing with
' unread input queued makes the kernel answer the client with RST, and most
' clients then report "connection reset" instead of ever reading the 4xx we
' just wrote. Draining what the client already sent — at most 4 MiB, each
' read waiting no more than 500 ms — lets the close complete cleanly so the reply is
' delivered, while a client that keeps streaming cannot hold the loop. Never
' called after a complete request (there is nothing to drain and waiting for
' the client's EOF would add latency to every exchange).
SUB __http_lingerTls(RES sock AS tls::Socket)
  tls::setReadTimeout(sock, 500)
  MUT remaining AS Integer = 4194304
  WHILE remaining > 0
    MUT chunk AS List OF Byte = []
    chunk = tls::read(sock, 65536) TRAP(e)
      RECOVER []
    END TRAP
    IF len(chunk) = 0 THEN
      remaining = 0
    ELSE
      remaining = remaining - len(chunk)
    END IF
  END WHILE
END SUB"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_lingerTls", BODY));
}
