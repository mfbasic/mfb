//! `__http_dispatch` — shared private helper for the `http` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __http_dispatch(req AS Request, routes AS List OF Route) AS Response
  LET n AS Integer = len(routes)
  MUT idx AS Integer = 0
  WHILE idx < n
    LET r AS Route = collections::get(routes, idx)
    LET m AS __http_RouteMatch = __http_matchPath(r.pattern, req.path)
    IF m.matched = TRUE THEN
      LET boundReq AS Request = WITH req { params := m.params }
      RETURN __http_invokeHandler(r, boundReq)
    END IF
    idx = idx + 1
  END WHILE
  RETURN __http_status(404, "Not Found")
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("http_dispatch", BODY));
}
