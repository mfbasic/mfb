//! Value-construction forms of `engine/value/builder_values/` that only a module
//! rewritten after the front end can present.
//!
//! * bug-781: a capture-less closure value is the static descriptor a function
//!   reference loads (`env = 0`), so it lowers exactly as that reference does
//!   (`lower_inner.rs`). The IR never builds a `Closure` with no captures
//!   (`ir/lower/expression.rs` makes a capture-less lambda a `FunctionRef`), but a
//!   decoded package's value codec can carry one. Checked by rewriting the
//!   program's `FunctionRef` to the equivalent empty `Closure` and comparing
//!   `main`'s instruction streams, op by op.
//! * A union wrap of a member that is neither a resource nor a record is refused
//!   by name (`lower_construct.rs`).

use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};

const SOURCE: &str = "\
IMPORT io

FUNC main() AS Integer
  LET f AS FUNC(Integer) AS Integer = LAMBDA(v AS Integer) -> v * 2
  io::print(toString(f(21)))
  RETURN 0
END FUNC
";

fn main_ops(module: &NirModule, target: CodeTarget) -> Vec<String> {
    let plan = code_for_nir(module, target).unwrap_or_else(|error| panic!("{error}"));
    plan.functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main")
        .instructions
        .iter()
        .map(|instruction| instruction.op.mnemonic().to_string())
        .collect()
}

#[test]
fn a_capture_less_closure_lowers_as_its_function_reference() {
    for target in CodeTarget::ALL {
        let reference = nir_for_src(SOURCE, target, Console).expect("lowers to NIR");
        let mut closure = nir_for_src(SOURCE, target, Console).expect("lowers to NIR");
        let main = closure
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .expect("main");
        let mut rewritten = 0;
        for op in &mut main.body {
            if let NirOp::Bind {
                value: Some(value), ..
            } = op
            {
                if let NirValue::FunctionRef { name, type_ } = value {
                    *value = NirValue::Closure {
                        name: name.clone(),
                        type_: type_.clone(),
                        captures: Vec::new(),
                    };
                    rewritten += 1;
                }
            }
        }
        assert_eq!(
            rewritten,
            1,
            "{}: the probe binds one lambda",
            target.name()
        );
        assert_eq!(
            main_ops(&closure, target),
            main_ops(&reference, target),
            "{}",
            target.name()
        );
    }
}

/// A union wrap whose member is neither a resource nor a record names no layout
/// to build: refused by name rather than wrapped by a guess. The front end only
/// ever wraps a declared variant, so the member is renamed in the module.
#[test]
fn a_union_wrap_of_a_member_that_is_no_record_is_refused() {
    let source = "\
IMPORT io

TYPE Circle
  r AS Integer
END TYPE

TYPE Square
  side AS Integer
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC area(s AS Shape) AS Integer
  MATCH s
    CASE Circle(c)
      RETURN c.r
    CASE Square(q)
      RETURN q.side
  END MATCH
END FUNC

FUNC main() AS Integer
  LET s AS Shape = Circle[r := 2]
  io::print(toString(area(s)))
  RETURN 0
END FUNC
";
    for target in CodeTarget::ALL {
        let mut module = nir_for_src(source, target, Console).expect("lowers to NIR");
        let main = module
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .expect("main");
        let mut renamed = 0;
        for op in &mut main.body {
            if let NirOp::Bind {
                value: Some(NirValue::UnionWrap { member_type, .. }),
                ..
            } = op
            {
                *member_type = crate::types::ParameterType::declared("Nope");
                renamed += 1;
            }
        }
        assert_eq!(renamed, 1, "{}: the probe wraps one variant", target.name());
        let Err(error) = code_for_nir(&module, target) else {
            panic!("{}: a wrap of 'Nope' lowered", target.name());
        };
        assert!(
            error.contains("union wrap member 'Nope' is not a record"),
            "{}: {error}",
            target.name()
        );
    }
}
