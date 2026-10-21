//! Statement temporaries (`engine/value/builder_values/pending_temps.rs`).
//!
//! Rule 5 (`mfb spec language resource-management`): a call whose result is an
//! aggregate reaching resources hands the caller's STATEMENT the resources its
//! `RETURN` released, and the statement takes every one at once — one owned list
//! per resource leaf kind. A function value inside the result carries its
//! resources in its environment, so those are taken through the closure's own
//! routine onto a second, dynamic list. Both lists are stack objects of the
//! statement (`resource_temp_owned`, `resource_temp_dynamic`).

use crate::codegen::engine::builder::PendingTemp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};
use crate::types::ParameterType;

fn program(kit_fields: &str, kit_value: &str) -> String {
    format!(
        "IMPORT io
IMPORT fs

TYPE Kit
  f AS RES fs::File
{kit_fields}END TYPE

FUNC put(f AS fs::File, s AS String) AS Integer
  fs::writeAll(f, s)
  RETURN len(s)
END FUNC

FUNC mkKit(seed AS String) AS Kit
  RES f AS fs::File = fs::createTempFile()
  RES h AS fs::File = fs::createTempFile()
  RETURN Kit[f := f{kit_value}]
END FUNC

FUNC useKit(k AS Kit) AS Integer
  RETURN fs::size(k.f)
END FUNC

FUNC main() AS Integer
  io::print(toString(useKit(mkKit(\"y\"))))
  RETURN 0
END FUNC
"
    )
}

fn main_slots(source: &str, target: CodeTarget) -> Vec<String> {
    code_function(code_for_src_cached(source, target, Console), "main")
        .stack_slots
        .iter()
        .map(|slot| slot.name.clone())
        .collect()
}

fn has(slots: &[String], prefix: &str) -> bool {
    slots.iter().any(|name| name.starts_with(prefix))
}

#[test]
fn a_returned_closure_hands_its_resources_to_the_statement_on_a_dynamic_list() {
    let with_closure = program(
        "  g AS FUNC(String) AS Integer\n",
        ", g := LAMBDA(s AS String) -> put(h, s & seed)",
    );
    for target in CodeTarget::ALL {
        let slots = main_slots(&with_closure, target);
        assert!(
            has(&slots, "resource_temp_owned"),
            "{}: the file field is taken onto an owned list: {slots:?}",
            target.name()
        );
        assert!(
            has(&slots, "resource_temp_dynamic"),
            "{}: the closure's environment is taken onto a dynamic list: {slots:?}",
            target.name()
        );
    }
    let plain = program("", "");
    let slots = main_slots(&plain, CodeTarget::LinuxX86_64);
    assert!(has(&slots, "resource_temp_owned"), "{slots:?}");
    assert!(
        !has(&slots, "resource_temp_dynamic"),
        "no function value, no dynamic list: {slots:?}"
    );
}

/// `retarget_pending_temp` moves the identity token of the statement's LAST
/// temporary only when that temporary is the one the identity lowering was
/// handed: re-identifying anything else would let an owner claim a sibling's
/// block (and leave its own to the statement-end free).
#[test]
fn retargeting_moves_only_the_tail_temporary_it_names() {
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("probe", &TestPlatform);
    let temp = |location: &str| PendingTemp {
        type_: ParameterType::String,
        slot: 0,
        location: Operand::from(location.to_string()),
        shallow: false,
        resource: false,
        union_box: false,
    };
    builder.retarget_pending_temp(
        &Operand::from("a".to_string()),
        &Operand::from("b".to_string()),
    );
    assert!(builder.pending_temp_frees.is_empty());
    builder.pending_temp_frees.push(temp("sibling"));
    builder.pending_temp_frees.push(temp("tail"));
    builder.retarget_pending_temp(
        &Operand::from("sibling".to_string()),
        &Operand::from("moved".to_string()),
    );
    let locations = |builder: &crate::codegen::engine::builder::CodeBuilder<'_>| {
        builder
            .pending_temp_frees
            .iter()
            .map(|temp| temp.location.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(locations(&builder), ["sibling", "tail"]);
    builder.retarget_pending_temp(
        &Operand::from("tail".to_string()),
        &Operand::from("moved".to_string()),
    );
    assert_eq!(locations(&builder), ["sibling", "moved"]);
}
