//! Statement temporaries (`engine/value/builder_values/pending_temps.rs`) for two
//! producers the program suites only reach as a binding's root, driven on a builder.
//!
//! * A trapped call (`CallResult`) lowered as a NESTED operand goes through the same
//!   resource-temporary classification as a plain call (bug-842): a result that
//!   reaches no resource registers no resource temporary.
//! * A `CallResult` payload is not registered for the statement's end when the
//!   value is a borrowed `get` result (plan-86 E): the container owns that element,
//!   so freeing it at the statement's end would free it under its owner.

use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::harness;
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirModule, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;
use std::collections::HashMap;

const TRAPPED: &str = "\
IMPORT io

FUNC risky(n AS Integer) AS Integer
  IF n > 3 THEN
    FAIL error(404, \"too big\")
  END IF
  RETURN n
END FUNC

FUNC main() AS Integer
  LET v AS Integer = risky(2) TRAP(e)
    RETURN 1
  END TRAP
  io::print(toString(v))
  RETURN 0
END FUNC
";

fn module() -> NirModule {
    nir_for_src(TRAPPED, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|err| panic!("the probe lowers to NIR: {err}"))
}

fn trapped_call(module: &NirModule) -> NirValue {
    struct Find(Option<NirValue>);
    impl NirVisitor for Find {
        fn visit_value(&mut self, value: &NirValue) {
            if self.0.is_none() && matches!(value, NirValue::CallResult { .. }) {
                self.0 = Some(value.clone());
                return;
            }
            walk_value(self, value);
        }
    }
    let main = module
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main");
    let mut find = Find(None);
    find.visit_ops(&main.body);
    find.0.expect("main traps a call")
}

#[test]
fn a_nested_trapped_call_registers_no_resource_temporary() {
    let module = module();
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let mut harness = harness(&model);
    harness.functions = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect::<HashMap<String, &NirFunction>>();
    harness.function_symbols = module
        .functions
        .iter()
        .map(|function| {
            (
                function.name.clone(),
                crate::target::shared::nir::function_symbol(&function.name),
            )
        })
        .collect();
    harness
        .string_symbols
        .extend(crate::codegen::memory::data::string_symbols(
            &module, &model,
        ));
    let platform = TestPlatform;
    let mut builder = harness.builder("main", &platform);
    let result = match builder.lower_value(&trapped_call(&module)) {
        Ok(result) => result,
        Err(error) => panic!("a trapped call lowers as an operand: {error}"),
    };
    assert_eq!(
        result.type_,
        ParameterType::ResultOf(Box::new(ParameterType::Integer))
    );
    assert!(
        builder.statement_temps.drops.is_empty(),
        "an Integer result reaches no resource, so nothing is listed for the statement"
    );
}

#[test]
fn a_borrowed_get_result_registers_no_payload_temporary() {
    let model = TypeModel::from_module(&module()).expect("the probe's type model builds");
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("borrow", &platform);
    let slot = builder.allocate_stack_object("payload", 8);
    let produced = NirValue::Call {
        target: "mk".to_string(),
        args: Vec::new(),
        loc: Default::default(),
    };
    let list = ParameterType::list_of(ParameterType::Integer);
    // A fresh list payload is the statement's to free...
    let before = builder.pending_temp_frees.len();
    builder.register_call_result_payload_temp(&produced, &list, slot);
    assert_eq!(builder.pending_temp_frees.len(), before + 1);
    // ...unless the lowering is producing a borrowed element.
    builder.borrow_get_result = true;
    let before = builder.pending_temp_frees.len();
    builder.register_call_result_payload_temp(&produced, &list, slot);
    assert_eq!(
        builder.pending_temp_frees.len(),
        before,
        "a borrowed element is the container's"
    );
}
