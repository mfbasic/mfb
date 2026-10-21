//! Owned-variant demand (`engine/analysis/handover.rs`, plan-147) for three callee
//! shapes the other suites do not lower.
//!
//! * S10: an `ISOLATED` function is an entry point, never called through an owned
//!   variant — the same callee without `ISOLATED` gets one (`addOne$own1`).
//! * P2: a parameter a closure captures is not consumed — the closure may still
//!   read it after the callee would have freed it — so `peek` gets no variant.
//! * plan-147-F: a record whose only inlined sub-block is a fixed-size record
//!   (`Pt`, two `Float`s) is handed over to `RETURN WITH s { a := Pt[…] }`, which
//!   stores the new `Pt` into the parameter's own block (`shift$own1`).

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

fn program(isolated: &str) -> String {
    format!(
        "IMPORT collections
IMPORT io

TYPE Pt
  x AS Float
  y AS Float
END TYPE

TYPE Seg
  a AS Pt
  n AS Integer
  xs AS List OF Integer
END TYPE

{isolated}FUNC addOne(xs AS List OF Integer) AS List OF Integer
  RETURN collections::append(xs, 1)
END FUNC

FUNC peek(xs AS List OF Integer) AS List OF Integer
  collections::forEach([1], LAMBDA(v AS Integer) -> io::print(toString(len(xs) + v)))
  RETURN collections::append(xs, 1)
END FUNC

FUNC shift(s AS Seg, d AS Float) AS Seg
  RETURN WITH s {{ a := Pt[x := s.a.x + d, y := s.a.y] }}
END FUNC

FUNC main() AS Integer
  MUT xs AS List OF Integer = [1]
  xs = addOne(xs)
  xs = peek(xs)
  MUT s AS Seg = Seg[a := Pt[x := 0.0, y := 0.0], n := 0, xs := [1]]
  FOR i = 1 TO 3
    s = shift(s, 1.0)
  NEXT
  io::print(toString(len(xs)) & toString(s.a.x))
  RETURN 0
END FUNC
"
    )
}

fn functions(source: &str) -> Vec<String> {
    code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
        .functions
        .iter()
        .map(|function| function.name.clone())
        .collect()
}

#[test]
fn isolated_and_capturing_callees_get_no_owned_variant_and_a_fixed_record_field_store_does() {
    let isolated = functions(&program("ISOLATED "));
    assert!(isolated.iter().any(|f| f == "shift$own1"), "{isolated:?}");
    assert!(
        !isolated.iter().any(|f| f.starts_with("addOne$own")),
        "{isolated:?}"
    );
    assert!(
        !isolated.iter().any(|f| f.starts_with("peek$own")),
        "{isolated:?}"
    );
    let plain = functions(&program(""));
    assert!(plain.iter().any(|f| f == "addOne$own1"), "{plain:?}");
}
