//! The resource-cleanup bookkeeping of `resource/cleanup/builder_resource_cleanup.rs`,
//! driven directly on a builder over a real program's type model.
//!
//! A function's scope drops are a list (`active_cleanups`) that the lowering edits as
//! it goes: a floated bind gives its close away, a consumed argument's owner stops
//! closing, a union binding that only holds a box frees the box and closes nothing.
//! Programs exercise the common edits; these rows pin the edges each one owes and no
//! program isolates — the edit asked about a name that has no cleanup, a type that is
//! not a resource union, an alias chain that loops — because getting any of them
//! wrong is a close that runs twice or never.

use crate::codegen::engine::builder::{
    ActiveCleanup, CodeBuilder, LocalValue, ResourceCleanup, TypeModel,
};
use crate::codegen::engine::tests::test_support::BuilderHarness;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::nir::NirValue;
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
    TypeModel::from_module(&module).expect("the model builds")
}

fn stream() -> ParameterType {
    ParameterType::declared("Stream")
}

fn file() -> ParameterType {
    ParameterType::declared("fs.File")
}

/// Run `body` on a fresh builder over the program's type model.
fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let platform = platform();
    let harness = BuilderHarness {
        type_model: model(),
        ..BuilderHarness::default()
    };
    let mut builder = harness.builder("_mfb_cleanup_probe", &platform);
    body(&mut builder)
}

fn local(builder: &mut CodeBuilder<'_>, name: &str, type_: ParameterType) {
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
}

fn resource_cleanup(name: &str, frees_record: bool) -> ActiveCleanup {
    ActiveCleanup::Resource(ResourceCleanup {
        name: name.to_string(),
        symbol: "_probe_close".to_string(),
        state_type: None,
        has_io_buffers: false,
        frees_record,
        moved_record_only: false,
        owner_flag_slot: None,
        reclaim_slot: None,
        default_record_slot: None,
        scope_depth: 1,
    })
}

fn cleanup_names(builder: &CodeBuilder<'_>) -> Vec<String> {
    builder
        .active_cleanups
        .iter()
        .filter_map(|cleanup| match cleanup {
            ActiveCleanup::Resource(r) => Some(format!("resource {}", r.name)),
            ActiveCleanup::ResourceUnion(u) => Some(format!("union {}", u.name)),
            _ => None,
        })
        .collect()
}

/// The per-variant tag tables are a resource UNION's; any other type has none.
#[test]
fn only_a_resource_union_has_variant_tag_tables() {
    with_builder(|builder| {
        for not_union in [file(), ParameterType::Integer] {
            assert!(
                builder
                    .resource_union_record_free_tags(&not_union)
                    .is_empty(),
                "`{not_union}` has no record-free tags"
            );
            assert!(
                builder.resource_union_io_buffer_tags(&not_union).is_empty(),
                "`{not_union}` has no I/O-buffer tags"
            );
            assert!(builder.resource_union_cleanup(&not_union).is_none());
        }
        let file_tag = builder.type_model.union_variant_tags[&file()];
        assert_eq!(
            builder.resource_union_io_buffer_tags(&stream()),
            vec![file_tag],
            "the File variant carries the two I/O buffer words, the Socket does not"
        );
        let closes = builder
            .resource_union_cleanup(&stream())
            .expect("Stream is a resource union");
        assert_eq!(closes.len(), 2, "one close per variant: {closes:?}");
    });
}

/// An alias chain is followed to its root, and a chain that loops is cut off
/// rather than followed forever: the root query stops after 64 hops, and a
/// returned local whose alias sources loop is not an owned box.
#[test]
fn alias_chains_resolve_to_their_root_and_loops_terminate() {
    with_builder(|builder| {
        builder
            .live_resource_aliases
            .insert("c".to_string(), "b".to_string());
        builder
            .live_resource_aliases
            .insert("b".to_string(), "a".to_string());
        assert_eq!(builder.resource_alias_root("c"), "a");
        assert_eq!(builder.resource_alias_root("a"), "a", "a root is its own");

        builder
            .live_resource_aliases
            .insert("a".to_string(), "c".to_string());
        let root = builder.resource_alias_root("c");
        assert!(
            ["a", "b", "c"].contains(&root.as_str()),
            "a looping chain ends on one of its members: {root}"
        );

        builder
            .resource_alias_sources
            .insert("u".to_string(), "v".to_string());
        builder
            .resource_alias_sources
            .insert("v".to_string(), "u".to_string());
        local(builder, "u", stream());
        assert!(
            !builder.returned_resource_union_owns_box(&NirValue::Local("u".to_string())),
            "a returned local whose alias sources loop owns no box"
        );
    });
}

/// A box-only union drop is registered for a resource union, and for nothing else.
#[test]
fn a_box_cleanup_is_registered_only_for_a_resource_union() {
    with_builder(|builder| {
        builder.register_union_box_cleanup("f", &file());
        builder.register_union_box_cleanup("n", &ParameterType::Integer);
        assert!(
            builder.active_cleanups.is_empty(),
            "a concrete resource or a non-resource gets no union box drop"
        );
        builder.register_union_box_cleanup("u", &stream());
        match builder.active_cleanups.as_slice() {
            [ActiveCleanup::ResourceUnion(union)] => {
                assert_eq!(union.name, "u");
                assert!(!union.closes_variant, "the box drop closes nothing");
            }
            _ => panic!("one union box drop: {:?}", cleanup_names(builder)),
        }
    });
}

/// A floated source gives up its close. A concrete source's cleanup is removed; a
/// union source keeps a drop that frees its box and closes nothing; a name with no
/// cleanup is a no-op.
#[test]
fn a_floated_source_gives_up_its_close() {
    with_builder(|builder| {
        builder.active_cleanups.push(resource_cleanup("f", true));
        builder.register_union_box_cleanup("u", &stream());
        if let Some(ActiveCleanup::ResourceUnion(union)) = builder.active_cleanups.last_mut() {
            union.closes_variant = true;
            union.record_free_tags = vec![1];
            union.io_buffer_tags = vec![1];
            union.state_type = Some(ParameterType::Integer);
        }

        builder.release_floated_source_cleanup("nobody");
        assert_eq!(cleanup_names(builder), ["resource f", "union u"]);

        builder.release_floated_source_cleanup("u");
        match builder.active_cleanups.last() {
            Some(ActiveCleanup::ResourceUnion(union)) => {
                assert!(!union.closes_variant, "the union source closes nothing");
                assert!(union.record_free_tags.is_empty() && union.io_buffer_tags.is_empty());
                assert!(union.state_type.is_none());
            }
            _ => panic!("the union source keeps its box drop"),
        }

        builder.release_floated_source_cleanup("f");
        assert_eq!(cleanup_names(builder), ["union u"]);
    });
}

/// The union-drop lookup skips every cleanup that is not that binding's union drop.
#[test]
fn the_union_drop_lookup_is_by_binding_and_kind() {
    with_builder(|builder| {
        builder.active_cleanups.push(resource_cleanup("u", true));
        assert!(
            builder.resource_union_binding_cleanup("u").is_none(),
            "a concrete cleanup under the same name is not a union drop"
        );
        builder.register_union_box_cleanup("u", &stream());
        assert!(builder.resource_union_binding_cleanup("u").is_some());
    });
}

/// A drop for a binding with no slot emits nothing (the binding was never lowered
/// in this frame).
#[test]
fn a_union_drop_for_an_unbound_name_emits_nothing() {
    with_builder(|builder| {
        builder.register_union_box_cleanup("ghost", &stream());
        let Some(cleanup) = builder.resource_union_binding_cleanup("ghost") else {
            panic!("the drop is registered");
        };
        let before = builder.instructions.len();
        builder
            .emit_resource_union_cleanup_call(&cleanup)
            .expect("an unbound name is skipped");
        assert_eq!(builder.instructions.len(), before, "nothing emitted");
    });
}

/// A consuming argument stops its owner's close. An argument list too short to
/// hold the consumed parameter, or a consumed name with no local, changes nothing.
#[test]
fn a_consumed_argument_stops_only_its_own_owners_close() {
    with_builder(|builder| {
        let index = crate::codegen::resource::builtin_consuming_parameter_index("canvas.setGroup")
            .expect("canvas::setGroup has a consuming parameter");
        local(builder, "f", file());
        builder.active_cleanups.push(resource_cleanup("f", false));

        builder.deactivate_consumed_cleanups("canvas.setGroup", &[]);
        let mut args = vec![NirValue::Local("other".to_string()); index + 1];
        args[index] = NirValue::Local("ghost".to_string());
        builder.deactivate_consumed_cleanups("canvas.setGroup", &args);
        assert_eq!(
            cleanup_names(builder),
            ["resource f"],
            "nothing named `f` was consumed"
        );

        args[index] = NirValue::Local("f".to_string());
        builder.deactivate_consumed_cleanups("canvas.setGroup", &args);
        assert!(
            cleanup_names(builder).is_empty(),
            "the consumed resource's close moved to the callee"
        );
    });
}

/// An explicit close retires its argument's cleanup; an argument naming no local
/// is skipped.
#[test]
fn a_closed_argument_retires_its_cleanup() {
    with_builder(|builder| {
        local(builder, "f", file());
        builder.active_cleanups.push(resource_cleanup("f", false));
        builder.deactivate_moved_resource_arguments(
            "fs.close",
            &[NirValue::Local("ghost".to_string())],
        );
        assert_eq!(cleanup_names(builder), ["resource f"]);
    });
}

/// An owned-list leaf must be closable: a leaf with no close op is refused by name.
#[test]
fn an_owned_list_leaf_without_a_close_op_is_refused() {
    with_builder(|builder| {
        let Err(err) = builder.resource_leaf_drop(&ParameterType::Integer) else {
            panic!("an Integer leaf has no close op");
        };
        assert!(
            err.contains("owned-list leaf type 'Integer' has no registered close op"),
            "{err}"
        );
        assert!(builder.resource_leaf_drop(&file()).is_ok());
        assert!(builder.resource_leaf_drop(&stream()).is_ok());
    });
}
