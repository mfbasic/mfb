//! The in-place map arms (`collection/assign/builder_inplace_setmap/map_ops.rs`:
//! `m = collections::merge(m, n, preferB)` and `m = collections::mapValues(m, f)`)
//! handed a module the front end would have rejected (`nir_retype.rs`), plus the
//! fixed-width merge a program reaches.
//!
//! Both arms mutate a `Map`; a binding of another collection kind is declined and the
//! copying lowering refuses it by name. `mapValues` writes `f`'s results over the
//! map's own values, so `f` must be a function; anything else is refused by name.

use super::nir_retype::{refusal, retyped};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
IMPORT io

FUNC mg(m AS Map OF String TO Integer, n AS Map OF String TO Integer) AS Integer
  MUT ys AS Map OF String TO Integer = m
  ys = collections::merge(ys, n, TRUE)
  RETURN len(ys)
END FUNC

FUNC mgi(m AS Map OF Integer TO Integer, n AS Map OF Integer TO Integer) AS Integer
  MUT ys AS Map OF Integer TO Integer = m
  ys = collections::merge(ys, n, FALSE)
  RETURN len(ys)
END FUNC

FUNC mv(m AS Map OF String TO Integer, f AS FUNC(Integer) AS Integer) AS Integer
  MUT ys AS Map OF String TO Integer = m
  ys = collections::mapValues(ys, f)
  RETURN len(ys)
END FUNC

FUNC bump(n AS Integer) AS Integer
  RETURN n + 1
END FUNC

FUNC main() AS Integer
  LET a AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  LET b AS Map OF Integer TO Integer = Map OF Integer TO Integer { 1 := 2 }
  io::print(toString(mg(a, a) + mgi(b, b) + mv(a, bump)))
  RETURN 0
END FUNC
"#;

fn ty(name: &str) -> ParameterType {
    ParameterType::parse(name)
}

/// `merge` mutates a `Map`: over a `List` it is declined and refused by the
/// copying lowering.
#[test]
fn merge_declines_a_list() {
    let list = ty("List OF Integer");
    let err = refusal(
        retyped(
            SRC,
            "mg",
            &[("m", list.clone()), ("n", list.clone()), ("ys", list)],
        ),
        "merge over a List",
    );
    assert!(err.contains("List OF Integer"), "merge over a List: {err}");
}

/// `mapValues` writes `f`'s results over the map's values: a callback that is not
/// a function is refused by name.
#[test]
fn map_values_refuses_a_non_function() {
    let err = refusal(
        retyped(SRC, "mv", &[("f", ParameterType::Integer)]),
        "mapValues with an Integer callback",
    );
    assert!(
        err.contains("native in-place mapValues action must be a function, got Integer"),
        "{err}"
    );
}

/// A merge of two fixed-width maps borrows no key or value bytes, so it reserves no
/// scratch areas; a `String`-keyed merge does.
#[test]
fn a_fixed_width_merge_needs_no_scratch_areas() {
    let plan = code_for_src_cached(SRC, CodeTarget::LinuxX86_64, Console);
    let areas = |name: &str| {
        code_function(plan, name)
            .stack_slots
            .iter()
            .filter(|slot| slot.type_ == "inplace_areas_need")
            .count()
    };
    assert_eq!(areas("mgi"), 0, "Integer keys and values need no scratch");
    assert!(
        areas("mg") > 0,
        "String keys are copied out through scratch"
    );
}

/// A key or value type no declaration defines cannot be read out of an entry: the
/// merge refuses it at the other map's key or value, `mapValues` at the value it
/// hands `f`.
#[test]
fn an_undeclared_key_or_value_type_is_refused_by_name() {
    for map in ["Map OF Nope TO Integer", "Map OF String TO Nope"] {
        let m = ty(map);
        let err = refusal(
            retyped(SRC, "mg", &[("m", m.clone()), ("n", m.clone()), ("ys", m)]),
            &format!("merge over {map}"),
        );
        assert!(
            err.contains("native collection packed payload does not support type 'Nope'"),
            "merge over {map}: {err}"
        );
    }
    let m = ty("Map OF String TO Nope");
    let f = ParameterType::func(vec![ty("Nope")], ty("Nope"));
    let err = refusal(
        retyped(SRC, "mv", &[("m", m.clone()), ("ys", m), ("f", f)]),
        "mapValues over Map OF String TO Nope",
    );
    assert!(
        err.contains("native collection packed payload does not support type 'Nope'"),
        "{err}"
    );
}
