//! The shrinking in-place arms (`collection/assign/builder_inplace_shrink.rs`) handed
//! a module the front end would have rejected (`nir_retype.rs`).
//!
//! `x = collections::filter(x, p)` calls `p` for every element before it compacts,
//! so `p` must be a function returning `Boolean`; a predicate that is not a function
//! is refused by name rather than branched to.

use super::nir_retype::{refusal, retyped};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
IMPORT io

FUNC keep(n AS Integer) AS Boolean
  RETURN n > 1
END FUNC

FUNC fl(xs AS List OF Integer, p AS FUNC(Integer) AS Boolean) AS Integer
  MUT ys AS List OF Integer = xs
  ys = collections::filter(ys, p)
  RETURN len(ys)
END FUNC

FUNC main() AS Integer
  io::print(toString(fl([1, 2], keep)))
  RETURN 0
END FUNC
"#;

#[test]
fn filter_refuses_a_predicate_that_is_not_a_function() {
    let err = refusal(
        retyped(SRC, "fl", &[("p", ParameterType::Integer)]),
        "filter with an Integer predicate",
    );
    assert!(
        err.contains("native collection filter predicate must be a function, got Integer"),
        "{err}"
    );
}
