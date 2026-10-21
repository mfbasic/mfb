//! A `Float` a native `LINK` function returns is arithmetic like any other.
//!
//! Float `+ - * / ...` can raise `ErrFloatOverflow`, and its message string is
//! registered as a data object only when the module-level pre-pass
//! (`module_analysis::float_errors`) can see that some expression is Float
//! arithmetic. The pre-pass types a call from the module's own functions'
//! declared returns; a `LINK` wrapper (and any imported function) is not one of
//! them, so `libc::retDouble() + 1.0` typed as unknown, the pre-pass reported no
//! Float arithmetic, and lowering the `+` failed with
//!
//! > native code string literal 'Floating-point arithmetic overflowed to
//! > infinity.' has no data object
//!
//! while the same sum over a local bound from the call (`LET c = libc::retDouble()`)
//! built, because a local carries its declared type.

use crate::testutil::{try_code_for_linking_src, CodeTarget};

const FLOAT_FROM_LINK: &str = "\
IMPORT io

LINK \"c\" AS libc
  FUNC pid() AS Float
    SYMBOL \"getpid\"
    ABI () AS status CDouble
    RETURN status
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::pid() + 1.0))
  RETURN 0
END FUNC
";

/// Float arithmetic directly over a `LINK` call's result lowers on every backend.
#[test]
fn arithmetic_over_a_link_functions_float_result_lowers() {
    for target in CodeTarget::ALL {
        if let Err(err) = try_code_for_linking_src(FLOAT_FROM_LINK, target, &["c"]) {
            panic!(
                "a `Float` returned by a LINK function must be typed Float for the \
                 error-message pre-pass; on {} lowering said: {err}",
                target.name()
            );
        }
    }
}
