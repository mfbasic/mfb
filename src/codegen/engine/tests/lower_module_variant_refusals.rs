//! `lower_module_for_platform`'s whole-module refusals, from a mutated NIR module.
//!
//! * plan-147-D / bug-784: an owned or destination variant lowers under
//!   `<base>$own<mask>` / `<base>$into<param>`. The naming scheme does not assume a
//!   collision away — an import's symbol is any string — so a variant whose symbol
//!   already names a function of the module is refused rather than emitted as a
//!   second definition of it.
//! * The type model refuses a type kind it does not know.

use super::module_assembly_refusals::{linux_x86_64, module};
use crate::os::linux::flavor::LinuxFlavor::Glibc;
use crate::target::shared::nir::{NirImport, NirModule};
use crate::types::ParameterType;

/// `addOne` is handed its argument (an owned variant, `addOne$own1`) and `stepIn`
/// writes into the element's own field (a destination variant, `stepIn$into0`).
const VARIANTS: &str = "\
IMPORT collections
IMPORT io

TYPE Inner
  a AS Integer
  b AS Integer
  name AS String
END TYPE

TYPE Holder
  id AS Integer
  inner AS Inner
END TYPE

FUNC stepIn(w AS Inner, i AS Integer) AS Inner
  RETURN WITH w { a := w.a + i }
END FUNC

FUNC addOne(xs AS List OF Integer) AS List OF Integer
  RETURN collections::append(xs, 1)
END FUNC

FUNC main() AS Integer
  MUT hs AS List OF Holder = [Holder[id := 1, inner := Inner[a := 1, b := 2, name := \"x\"]]]
  FOR k = 0 TO 0
    LET p = collections::get(hs, k)
    hs = collections::set(hs, k, WITH p { inner := stepIn(p.inner, k) })
  NEXT
  MUT xs AS List OF Integer = [1]
  xs = addOne(xs)
  io::print(toString(len(hs) + len(xs)))
  RETURN 0
END FUNC
";

fn lower(module: &NirModule) -> Result<Vec<String>, String> {
    let plan = crate::target::linux_x86_64::plan::lower_module(module, Glibc)?;
    lower_with(module, &plan)
}

/// Lower `module` against `plan`, which may have been built from the module
/// before it was mutated.
fn lower_with(
    module: &NirModule,
    plan: &crate::target::shared::plan::NativePlan,
) -> Result<Vec<String>, String> {
    crate::codegen::engine::builder::lower_module_for_platform(module, plan, &[], &linux_x86_64())
        .map(|plan| {
            plan.functions
                .into_iter()
                .map(|function| function.symbol)
                .collect()
        })
}

/// An import whose symbol is `symbol`, as a corrupt (or adversarial) package
/// export would carry.
fn import_named(symbol: &str) -> NirImport {
    NirImport {
        package: "probe".to_string(),
        name: "probe::taken".to_string(),
        symbol: symbol.to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: Vec::new(),
        returns: ParameterType::Integer,
    }
}

#[test]
fn a_variant_whose_symbol_is_already_taken_is_refused() {
    let base = module(VARIANTS);
    let symbols = lower(&base).expect("the probe lowers");
    let own = format!(
        "{}$own1",
        crate::target::shared::nir::function_symbol("addOne")
    );
    let into = format!(
        "{}$into0",
        crate::target::shared::nir::function_symbol("stepIn")
    );
    for variant in [&own, &into] {
        assert!(
            symbols.contains(variant),
            "the probe must demand {variant}: {symbols:?}"
        );
    }

    let mut taken = module(VARIANTS);
    taken.imports.push(import_named(&own));
    assert_eq!(
        lower(&taken).expect_err("an owned variant must not reuse a module symbol"),
        format!(
            "plan-147-D: the owned-variant symbol `{own}` already names a function of this module"
        )
    );

    let mut taken = module(VARIANTS);
    taken.imports.push(import_named(&into));
    assert_eq!(
        lower(&taken).expect_err("a destination variant must not reuse a module symbol"),
        format!(
            "bug-784: the destination-variant symbol `{into}` already names a function of \
             this module"
        )
    );
}

/// The native plan refuses the kind first (it has no storage class for it), so the
/// plan is built from the module as lowered and the codegen is handed the mutated
/// one: the type model must refuse it in its own right.
#[test]
fn a_type_kind_the_model_does_not_know_is_refused() {
    let plan = crate::target::linux_x86_64::plan::lower_module(&module(VARIANTS), Glibc)
        .expect("the probe plans");
    let mut module = module(VARIANTS);
    let index = module
        .types
        .iter()
        .position(|type_| type_.name == "Inner")
        .expect("the probe declares Inner");
    module.types[index].kind = "bogus".to_string();
    assert_eq!(
        lower_with(&module, &plan).expect_err("an unknown type kind is refused"),
        "native code plan does not know type kind 'bogus'"
    );
}

/// bug-781: one environment routine serves every site that creates a `LAMBDA`, so
/// two functions creating it must record the same environment layout. Here a
/// second function creates `main`'s lambda over an `Integer` where `main` captures
/// a `String` (an owned word against a plain one): the module is refused when the
/// routines are emitted, naming the environment, instead of copying and dropping
/// one function's environment with the other's layout.
#[test]
fn a_lambda_created_with_two_layouts_across_functions_is_refused() {
    use crate::target::shared::nir::{NirOp, NirValue};
    let source = "IMPORT collections\nIMPORT io\n\nFUNC helper() AS Integer\n  LET one AS Integer = 1\n  \
                  RETURN one\nEND FUNC\n\nFUNC main() AS Integer\n  LET s AS String = io::readLine()\n  \
                  collections::forEach([1], LAMBDA(v AS Integer) -> io::print(s))\n  \
                  RETURN helper()\nEND FUNC\n";
    lower(&module(source)).expect("the program as written lowers");
    let plan = crate::target::linux_x86_64::plan::lower_module(&module(source), Glibc)
        .expect("the probe plans");
    let mut twice = module(source);
    let creates_lambda = twice
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main")
        .body
        .iter()
        .find(|op| matches!(op, NirOp::Eval { .. }))
        .expect("main's forEach creates the lambda")
        .clone();
    let helper = twice
        .functions
        .iter_mut()
        .find(|function| function.name == "helper")
        .expect("helper");
    helper.body.insert(
        0,
        NirOp::Bind {
            mutable: false,
            name: "s".to_string(),
            type_: ParameterType::Integer,
            value: Some(NirValue::Const {
                type_: ParameterType::Integer,
                value: "1".to_string(),
            }),
        },
    );
    helper.body.insert(1, creates_lambda);
    let err = lower_with(&twice, &plan).expect_err("two layouts for one environment");
    assert!(
        err.starts_with("closure environment `") && err.contains("is created with two layouts"),
        "{err}"
    );
}
