//! bug-1075: a store through a by-reference capture lists every resource the stored
//! value reaches on the capturing binding's owned-list for that resource's kind
//! (`cleanup/owned/builder_owned_cleanup/mod.rs`, `emit_captured_store_float`),
//! driven directly on a builder over a real program's type model.
//!
//! The binding's cell carries one owned-list head per resource kind its type reaches.
//! A stored value reaching a resource of a kind the binding's type does not list has
//! nowhere to be listed, and is refused by name rather than linked onto another
//! kind's list (whose drain would close it through the wrong op). Also pinned: the
//! signature walk that names the type variables a `collections` shape mentions.

use crate::codegen::cleanup::owned::builder_owned_cleanup::type_variables;
use crate::codegen::engine::builder::{LocalValue, TypeModel};
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::os::linux::flavor::LinuxFlavor;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT fs
IMPORT udp
IMPORT io

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

#[test]
fn a_captured_store_of_an_unlisted_resource_kind_is_refused() {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let platform = crate::target::linux_common::code::Platform::for_test(
        crate::target::linux_x86_64::code::X86_64,
        LinuxFlavor::Glibc,
    );
    let mut harness = BuilderHarness {
        type_model: TypeModel::from_module(&module).expect("the model builds"),
        ..BuilderHarness::default()
    };
    // The node allocation's failure arm raises `ErrOutOfMemory`.
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_str_oom".to_string());
    let mut builder = harness.builder("_mfb_captured_store_probe", &platform);
    // `files` is a by-reference capture of a `List OF fs::File`: its cell lists Files.
    let cell = builder.allocate_stack_object("files", 8);
    builder.locals.insert(
        "files".to_string(),
        LocalValue {
            type_: ParameterType::list_of(ParameterType::declared("fs.File")),
            stack_offset: cell,
            constant: None,
            by_ref: true,
        },
    );
    let value = builder.allocate_stack_object("value", 8);
    let Err(err) =
        builder.emit_captured_store_float("files", value, &ParameterType::declared("udp.Socket"))
    else {
        panic!("a udp::Socket has no list in a List OF fs::File binding's cell");
    };
    assert!(
        err.contains(
            "a resource of type 'udp.Socket' is stored through a captured reference to a \
             'List OF fs.File', which lists no such resource (bug-1075)"
        ),
        "{err}"
    );
    builder
        .emit_captured_store_float("files", value, &ParameterType::declared("fs.File"))
        .expect("a File is listed on the binding's File list");
}

/// The type variables of a `collections` signature are found inside every wrapper,
/// a `Result` included.
#[test]
fn type_variables_are_found_inside_a_result() {
    let mut out = Vec::new();
    type_variables(
        &ParameterType::result_of(ParameterType::list_of(ParameterType::var("T"))),
        &mut out,
    );
    assert_eq!(out, vec![ParameterType::var("T")]);
}
