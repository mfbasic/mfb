//! `__http_startExchange` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Connect per `url.scheme`, write the whole request, and return the transport as a
' `Stream` union carrying a default `http::PendingState`. The bind widens the concrete
' variant into the union and default-inits the STATE (plan-74); `RETURN s` carries
' the STATE out (plan-74 stateful-union return, unblocked by plan-80). Shared by
' `startRead` (no body) and the blocking `read`/`write` wrappers.
FUNC __http_startExchange(url AS net::Url, body AS String, hasBody AS Boolean, headers AS Map OF String TO String, method AS String) AS RES Stream STATE PendingState
  LET verb AS String = __http_normalizeMethod(method)
  LET request AS String = __http_buildRequest(verb, url, body, hasBody, headers)
  IF url.scheme = "https" THEN
    RES s AS Stream STATE PendingState = tls::connect(url.host, url.port, __HTTP_CONNECT_TIMEOUT_MS, url.host)
    MATCH s
      CASE tls::Socket(t)
        tls::write(t, request)
      CASE tcp::Socket(p)
        tcp::write(p, request)
    END MATCH
    s.state.sentAll = TRUE
    RETURN s
  END IF
  RES s AS Stream STATE PendingState = tcp::connect(url.host, url.port, __HTTP_CONNECT_TIMEOUT_MS)
  MATCH s
    CASE tcp::Socket(p)
      tcp::write(p, request)
    CASE tls::Socket(t)
      tls::write(t, request)
  END MATCH
  s.state.sentAll = TRUE
  RETURN s
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_startExchange", BODY));
}
