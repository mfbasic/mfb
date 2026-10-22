//! The owned-list edits of `cleanup/owned/builder_owned_cleanup/owned_list.rs`, driven
//! directly on a builder over a real program's type model.
//!
//! A resource floated into a container is pushed onto the container's owned-list for
//! its kind (§15.6). The push must find a list that can hold it — a function value
//! needs the container's dynamic list (bug-843), and a concrete resource pushed onto
//! a resource-UNION list must be one of the union's members (bug-1046) — and refuses
//! by name rather than linking a node the drain would close through the wrong op.
//! `RETURN` of an owner container retires exactly the block free of that binding.

use crate::codegen::engine::builder::{
    ActiveCleanup, CodeBuilder, LocalValue, OwnedValueCleanup, TypeModel,
};
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::os::linux::flavor::LinuxFlavor;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

type Linux = crate::target::linux_common::code::Platform<crate::target::linux_x86_64::code::X86_64>;

fn platform() -> Linux {
    crate::target::linux_common::code::Platform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        LinuxFlavor::Glibc,
    )
}

const PROGRAM: &str = "\
IMPORT fs
IMPORT tcp
IMPORT udp
IMPORT io

UNION Stream
  fs::File
  tcp::Socket
END UNION

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

fn model() -> TypeModel {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let mut model = TypeModel::from_module(&module).expect("the model builds");
    // Closures in this program carry resources, so a function value is a leaf.
    model.closure_capture_types = vec![ParameterType::declared("fs.File")];
    model
}

fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let platform = platform();
    let harness = BuilderHarness {
        type_model: model(),
        ..BuilderHarness::default()
    };
    let mut builder = harness.builder("_mfb_owned_list_probe", &platform);
    body(&mut builder)
}

fn local(builder: &mut CodeBuilder<'_>, name: &str, type_: ParameterType) -> usize {
    let stack_offset = builder.allocate_stack_object(name, 8);
    builder.locals.insert(
        name.to_string(),
        LocalValue {
            type_,
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
    stack_offset
}

fn owned_value(type_: ParameterType, stack_offset: usize) -> ActiveCleanup {
    ActiveCleanup::OwnedValue(OwnedValueCleanup {
        type_,
        stack_offset,
        capacity_slot: None,
        loop_alias_slot: None,
        result_wrapper: None,
    })
}

/// A function value floats to a container's DYNAMIC owned-list; a container that
/// has none is refused naming bug-843.
#[test]
fn a_function_value_needs_the_containers_dynamic_list() {
    with_builder(|builder| {
        let slot = builder.allocate_stack_object("closure", 8);
        let closure = ParameterType::func(vec![], ParameterType::Integer);
        let Err(err) = builder.emit_owned_list_push("bag", slot, &closure) else {
            panic!("a closure cannot float to a container with no dynamic list");
        };
        assert!(
            err.contains("a function value floats to 'bag', which has no dynamic owned-list"),
            "{err}"
        );
    });
}

/// A concrete resource pushed onto a resource-union list is listed under its member
/// tag; a resource that is not a member of the union is refused by name.
#[test]
fn a_resource_outside_the_lists_union_is_refused() {
    with_builder(|builder| {
        let head = builder.allocate_stack_object("owned_list_head", 8);
        builder
            .owned_list_heads
            .insert(("hs".to_string(), ParameterType::declared("Stream")), head);
        let slot = builder.allocate_stack_object("socket", 8);
        let Err(err) =
            builder.emit_owned_list_push("hs", slot, &ParameterType::declared("udp.Socket"))
        else {
            panic!("a udp::Socket is not a Stream member");
        };
        assert!(
            err.contains(
                "resource of type 'udp.Socket' floats to 'hs', whose owned-list holds \
                 'Stream', which 'udp.Socket' is not a member of"
            ),
            "{err}"
        );
    });
}

/// `RETURN` of an owner container retires the block free registered at that
/// binding's slot and nothing else; an unknown name is a no-op.
#[test]
fn returning_an_owner_retires_only_its_own_block_free() {
    with_builder(|builder| {
        let list = ParameterType::list_of(ParameterType::declared("fs.File"));
        let xs = local(builder, "xs", list.clone());
        let ys = local(builder, "ys", list.clone());
        builder.active_cleanups.push(owned_value(list.clone(), xs));
        builder.active_cleanups.push(owned_value(list, ys));
        let freed = |builder: &CodeBuilder<'_>| -> Vec<usize> {
            builder
                .active_cleanups
                .iter()
                .filter_map(|cleanup| match cleanup {
                    ActiveCleanup::OwnedValue(o) => Some(o.stack_offset),
                    _ => None,
                })
                .collect()
        };

        builder.retire_returned_owner_block("ghost");
        assert_eq!(
            freed(builder),
            vec![xs, ys],
            "an unknown name retires nothing"
        );

        builder.retire_returned_owner_block("xs");
        assert_eq!(
            freed(builder),
            vec![ys],
            "only `xs`'s block free is retired"
        );
    });
}
