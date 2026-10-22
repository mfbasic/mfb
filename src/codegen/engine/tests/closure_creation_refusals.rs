//! Closure creation (`engine/value/builder_values/lower_construct.rs`) refuses an
//! environment word its creator cannot supply, and a union wrap of a record that is
//! no variant.
//!
//! A non-escaping callback that self-updates a by-reference `String` capture works on
//! its OWNER's resources, handed over through one extra environment word each: the
//! creator's self-update scratch (plan-142-G), the owner's capacity shadow and its
//! `os::appResourcePath` base cache (plan-146-G). The creator's prescan reserves
//! each, from the same function table the callback analysis reads, so in a lowered
//! module they always agree. The lowering still refuses, naming the closure and what
//! is missing, when the creator does not have one: the alternative is an environment
//! word holding the address of a slot nothing reserved. Each row drives
//! `lower_value` on the program's real `Closure`, with the creator's slot absent and
//! then present.

use crate::codegen::engine::builder::{CodeBuilder, LocalValue, TypeModel};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record, union};
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirModule, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;
use std::collections::HashMap;

fn module(source: &str) -> NirModule {
    nir_for_src(source, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|err| panic!("the probe lowers to NIR: {err}"))
}

/// The first capturing `Closure` value in `main`.
fn capturing_closure(module: &NirModule) -> NirValue {
    struct Find(Option<NirValue>);
    impl NirVisitor for Find {
        fn visit_value(&mut self, value: &NirValue) {
            if self.0.is_none() {
                if let NirValue::Closure { captures, .. } = value {
                    if !captures.is_empty() {
                        self.0 = Some(value.clone());
                        return;
                    }
                }
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
    find.0.expect("main creates a capturing closure")
}

/// A creator-side resource an environment may need.
#[derive(Clone, Copy, PartialEq)]
enum Word {
    /// The creator's own self-update scratch (plan-142-G).
    Scratch,
    /// The owner's capacity shadow of the captured `String` (plan-146-G).
    Shadow,
    /// The owner's `os::appResourcePath` base cache (plan-146-G).
    Base,
}

/// Lower `module`'s capturing closure on a builder that has the module's functions
/// and type model, binds `captured` as a `String` local, and reserves every
/// creator-side word except `missing`.
fn lower_closure(module: &NirModule, captured: &str, missing: Option<Word>) -> Result<(), String> {
    let model = TypeModel::from_module(module).expect("the probe's type model builds");
    let mut harness = harness(&model);
    harness.functions = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect::<HashMap<String, &NirFunction>>();
    let platform = TestPlatform;
    let mut builder = harness.builder("main", &platform);
    let stack_offset = builder.allocate_stack_object(captured, 8);
    builder.locals.insert(
        captured.to_string(),
        LocalValue {
            type_: ParameterType::String,
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
    supply(&mut builder, captured, missing);
    builder.lower_value(&capturing_closure(module)).map(|_| ())
}

fn supply(builder: &mut CodeBuilder<'_>, captured: &str, missing: Option<Word>) {
    if missing != Some(Word::Scratch) {
        builder.self_update_scratch = Some(builder.allocate_stack_object("su_scratch", 8));
    }
    if missing != Some(Word::Shadow) {
        let slot = builder.allocate_stack_object("strcap", 8);
        builder
            .string_capacity_slots
            .insert(captured.to_string(), slot);
    }
    if missing != Some(Word::Base) {
        builder.string_app_resource_base = Some(builder.allocate_stack_object("base", 8));
    }
}

fn closure_name(module: &NirModule) -> String {
    match capturing_closure(module) {
        NirValue::Closure { name, .. } => name,
        _ => unreachable!("capturing_closure returns a Closure"),
    }
}

/// The callback builds `strings::upper(s)` in its creator's scratch.
const SCRATCH: &str = "IMPORT collections\nIMPORT io\nIMPORT strings\n\nFUNC main() AS Integer\n  \
     MUT s AS String = \"abc\"\n  collections::forEach([1, 2], LAMBDA(v AS Integer) -> s = \
     strings::upper(s))\n  io::print(s)\n  RETURN 0\nEND FUNC\n";

/// The callback appends to `acc` in place, through its owner's capacity shadow.
const SHADOW: &str =
    "IMPORT collections\nIMPORT io\n\nFUNC main() AS Integer\n  MUT acc AS String = \
     \"\"\n  collections::forEach([\"a\", \"b\"], LAMBDA(x AS String) -> acc = acc & x)\n  \
     io::print(acc)\n  RETURN 0\nEND FUNC\n";

/// The callback resolves `path` through its owner's base cache.
const BASE: &str =
    "IMPORT collections\nIMPORT io\nIMPORT os\n\nFUNC main() AS Integer\n  MUT path AS \
     String = \"icon.png\"\n  collections::forEach([1], LAMBDA(v AS Integer) -> path = \
     os::appResourcePath(path))\n  io::print(path)\n  RETURN 0\nEND FUNC\n";

#[test]
fn a_closure_borrowing_a_scratch_its_creator_does_not_own_is_refused() {
    let module = module(SCRATCH);
    let name = closure_name(&module);
    lower_closure(&module, "s", None).expect("a creator with every word lowers the closure");
    assert_eq!(
        lower_closure(&module, "s", Some(Word::Scratch)).expect_err("no scratch to lend"),
        format!("native closure `{name}` borrows a self-update scratch its creator does not own")
    );
}

/// A frame that only BORROWS a scratch (through its own environment word) cannot
/// lend it on.
#[test]
fn a_borrowed_scratch_is_not_lent_on() {
    let module = module(SCRATCH);
    let name = closure_name(&module);
    let model = TypeModel::from_module(&module).expect("the probe's type model builds");
    let mut harness = harness(&model);
    harness.functions = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect::<HashMap<String, &NirFunction>>();
    let platform = TestPlatform;
    let mut builder = harness.builder("main", &platform);
    let stack_offset = builder.allocate_stack_object("s", 8);
    builder.locals.insert(
        "s".to_string(),
        LocalValue {
            type_: ParameterType::String,
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
    supply(&mut builder, "s", None);
    builder.self_update_scratch_env = Some(builder.allocate_stack_object("su_env", 8));
    match builder.lower_value(&capturing_closure(&module)) {
        Err(error) => assert_eq!(
            error,
            format!(
                "native closure `{name}` borrows a self-update scratch its creator does not own"
            )
        ),
        Ok(_) => panic!("a borrowed scratch must not be lent on"),
    }
}

#[test]
fn a_closure_sharing_a_shadow_its_creator_does_not_have_is_refused() {
    let module = module(SHADOW);
    let name = closure_name(&module);
    lower_closure(&module, "acc", None).expect("a creator with every word lowers the closure");
    assert_eq!(
        lower_closure(&module, "acc", Some(Word::Shadow)).expect_err("no shadow to share"),
        format!(
            "native closure `{name}` shares the capacity shadow of `acc`, which its creator \
             does not have"
        )
    );
}

#[test]
fn a_closure_sharing_a_base_cache_its_creator_does_not_have_is_refused() {
    let module = module(BASE);
    let name = closure_name(&module);
    lower_closure(&module, "path", None).expect("a creator with every word lowers the closure");
    assert_eq!(
        lower_closure(&module, "path", Some(Word::Base)).expect_err("no base cache to share"),
        format!(
            "native closure `{name}` shares an os.appResourcePath base cache its creator does \
             not have"
        )
    );
}

/// A union wrap's member must be one of the union's variants: a record the model
/// knows but that has no variant tag is refused by name, not wrapped with some tag.
#[test]
fn a_union_wrap_of_a_record_that_is_no_variant_is_refused() {
    let model = model(vec![
        record("Circle", vec![field("r", "Integer")]),
        record("Pt", vec![field("x", "Integer")]),
        union("Shape", vec![("Circle", vec![field("r", "Integer")])]),
    ]);
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("wrap", &platform);
    let wrap = NirValue::UnionWrap {
        union_type: ParameterType::declared("Shape"),
        member_type: ParameterType::declared("Pt"),
        value: Box::new(NirValue::Constructor {
            type_: ParameterType::declared("Pt"),
            args: vec![NirValue::Const {
                type_: ParameterType::Integer,
                value: "1".to_string(),
            }],
        }),
    };
    match builder.lower_value(&wrap) {
        Err(error) => assert_eq!(error, "native code union variant 'Pt' does not resolve"),
        Ok(_) => panic!("a record that is no variant of the union must not be wrapped"),
    }
}
