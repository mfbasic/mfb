//! The helper-tier record builder (`memory/marshal/record.rs`,
//! `emit_build_inlined_record_sized`) takes one value slot (and one known size) per
//! declared field, in field order. A call that hands it a different number of slots is refused with the
//! counts, rather than building the record with fields shifted into the wrong slots.

use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::util::Vregs;
use crate::codegen::memory::marshal::{
    emit_build_inlined_record_sized, MarshalRegs, RecordBuildScratch,
};
use crate::target::shared::abi;
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
    // As the socket/udp/big helpers call it: fresh marshal registers, the result in
    // `mfb_return(1)` (llvm-cov credits a generic's lines from the instantiation
    // production uses most).
    emit_build_inlined_record_sized(
        "_mfb_probe",
        "probe",
        &ParameterType::declared("Pair"),
        &model,
        slots,
        &vec![None; slots.len()],
        &scratch,
        &MarshalRegs::fresh(&mut vregs),
        abi::mfb_return(1),
        "probe_alloc_fail",
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
