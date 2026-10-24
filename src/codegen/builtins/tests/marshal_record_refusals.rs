//! The helper-tier record builder (`memory/marshal/record.rs`,
//! `emit_build_inlined_record`) takes one value slot per declared field, in field
//! order. A call that hands it a different number of slots is refused with the
//! counts, rather than building the record with fields shifted into the wrong slots.

use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::util::Vregs;
use crate::codegen::memory::marshal::{emit_build_inlined_record, RecordBuildScratch};
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT io

TYPE Pair
  name AS String
  n AS Integer
END TYPE

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

fn build(slots: &[usize]) -> Result<(), String> {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let model = TypeModel::from_module(&module).expect("the model builds");
    let scratch = RecordBuildScratch {
        size: 64,
        result: 72,
        cursor: 80,
        block_size: 88,
    };
    let mut vregs = Vregs::new();
    let result = vregs.next();
    emit_build_inlined_record(
        "_mfb_probe",
        "probe",
        &ParameterType::declared("Pair"),
        &model,
        slots,
        &scratch,
        result,
        "probe_alloc_fail",
        &mut vregs,
        &mut Vec::new(),
        &mut Vec::new(),
    )
}

#[test]
fn a_record_built_from_the_wrong_number_of_slots_is_refused() {
    let Err(err) = build(&[0]) else {
        panic!("Pair has two fields; one slot cannot build it");
    };
    assert!(
        err.contains("native record 'Pair' construction expected 2 fields, got 1 slots"),
        "{err}"
    );
    build(&[0, 8]).expect("one slot per field builds the record");
}
