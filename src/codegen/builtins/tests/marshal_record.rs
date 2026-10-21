//! The helper-tier record marshaller sizes every inlined field kind from its own
//! block.
//!
//! `marshal::emit_build_inlined_record` is what a runtime helper uses to build a
//! record (`udp::Datagram`, `net::PingResult`, `io::KeyEvent`, ...) when the
//! caller holds no sizes: for each inlined field it must work out how many bytes
//! to copy into the record's data region. The size depends on what the field IS:
//!
//!   * a `String` is `byteLength + 9` (length word, trailing NUL);
//!   * a collection is `header + capacity * stride + dataCapacity`, plus a
//!     `capacity << 4` bucket region when it is a `Map` or `Set` - a wrong size
//!     frees or copies past the block and corrupts the arena;
//!   * a data union is self-describing: its `size` word, at +8;
//!   * a `RES <resource union>` field is a fixed two-word box.
//!
//! No builtin helper builds a record with a collection, union or resource-union
//! field, so no program reaches these arms; they are driven directly on the type
//! model of a real program. The twins below differ in exactly one field, so each
//! assertion is about the instructions that one field adds.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::util::Vregs;
use crate::codegen::memory::marshal::{emit_build_inlined_record, RecordBuildScratch};
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT fs
IMPORT io
IMPORT tcp

TYPE Dot
  x AS Integer
END TYPE

TYPE Tag
  name AS String
END TYPE

UNION Shape
  Dot
  Tag
END UNION

UNION Stream
  fs::File
  tcp::Socket
END UNION

TYPE WithMap
  m AS Map OF String TO Integer
  n AS Integer
END TYPE

TYPE WithSet
  s AS Set OF Integer
  n AS Integer
END TYPE

TYPE WithList
  l AS List OF Integer
  n AS Integer
END TYPE

TYPE WithUnion
  u AS Shape
  n AS Integer
END TYPE

TYPE WithBox
  stream AS RES Stream
  n AS Integer
END TYPE

TYPE WithString
  s AS String
  n AS Integer
END TYPE

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

/// The instructions that build `record` from one slot per field.
fn build(record: &str) -> Vec<CodeInstruction> {
    let model = model();
    let record_type = ParameterType::declared(record);
    let fields = model.record_fields[&record_type].len();
    let slots: Vec<usize> = (0..fields).map(|index| 8 * index).collect();
    let scratch = RecordBuildScratch {
        size: 8 * fields,
        result: 8 * fields + 8,
        cursor: 8 * fields + 16,
        block_size: 8 * fields + 24,
    };
    let mut vregs = Vregs::new();
    let result = vregs.next();
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    emit_build_inlined_record(
        "_mfb_probe",
        "probe",
        &record_type,
        &model,
        &slots,
        &scratch,
        result,
        "probe_alloc_fail",
        &mut vregs,
        &mut instructions,
        &mut relocations,
    )
    .unwrap_or_else(|err| panic!("{record}: {err}"));
    instructions
}

fn count(instructions: &[CodeInstruction], matches: impl Fn(&CodeInstruction) -> bool) -> usize {
    instructions.iter().filter(|i| matches(i)).count()
}

/// Shifts by four: the `capacity << 4` bucket region of a `Map`/`Set`.
fn bucket_shifts(instructions: &[CodeInstruction]) -> usize {
    count(instructions, |i| {
        i.op == CodeOp::LslImm && i.get("shift").as_deref() == Some("4")
    })
}

#[test]
fn a_map_or_set_field_is_sized_with_its_bucket_region_and_a_list_is_not() {
    let list = bucket_shifts(&build("WithList"));
    let map = bucket_shifts(&build("WithMap"));
    let set = bucket_shifts(&build("WithSet"));
    assert_eq!(list, 0, "a `List` has no hash buckets");
    assert!(
        map > 0,
        "a `Map` field's block carries `capacity << 4` bytes of hash buckets after its \
         entries and data; sizing it without them copies a short block"
    );
    assert_eq!(set, map, "a `Set` is a map of keys: the same bucket region");
}

#[test]
fn a_resource_union_field_is_a_fixed_two_word_box() {
    let sixteen = |instructions: &[CodeInstruction]| {
        count(instructions, |i| {
            i.op == CodeOp::MovImm && i.get("value").as_deref() == Some("16")
        })
    };
    assert!(
        sixteen(&build("WithBox")) > sixteen(&build("WithString")),
        "a `RES <resource union>` field is `{{tag, record-ptr}}`: its size is the \
         constant 16, where a `String` field's is walked from its length word"
    );
}

#[test]
fn a_data_union_field_is_sized_from_its_own_size_word() {
    // The union's size word sits at +8 of its block: one more load at that offset
    // than the String twin, which reads its length at +0.
    let size_word = |instructions: &[CodeInstruction]| {
        count(instructions, |i| {
            i.op == CodeOp::LdrU64 && i.get("offset").as_deref() == Some("8")
        })
    };
    assert!(
        size_word(&build("WithUnion")) > size_word(&build("WithString")),
        "a data union is self-describing: its size is the word at +8"
    );
}
