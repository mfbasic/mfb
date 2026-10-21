//! The `LINK` thunk emitter refuses a corrupted declaration by name.
//!
//! `ir::verify` rejects a malformed `LINK` function before codegen, but a decoded
//! `.mfp` package does not pass through it, and the thunk emitter is the last line
//! before a native call is built from the declaration. Every field it indexes by
//! name — a `BIND IN` field into its `CSTRUCT`, a parameter into the wrapper's, a
//! `CBuffer` slot into its `BUFFER SIZE` clause — is looked up and, if absent,
//! reported with the function and the name. The alternative to the report is a
//! thunk that stores through an offset it never computed.
//!
//! Each case lowers a well-formed program to NIR, corrupts exactly one thing in the
//! module, and asserts the refusal names it. The well-formed program is lowered
//! unchanged first, so a case that "refuses" because the program was never valid
//! cannot pass.

use crate::target::shared::nir::NirModule;
use crate::testutil::{code_for_nir, fixture_src, nir_for_linking_src, CodeTarget};
use crate::types::ParameterType;

/// A `CSTRUCT` of one `CInt32` field, read back through an `OUT` slot.
const OUT_STRUCT: &str = "\
IMPORT io

TYPE Rec
  field AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT S AS Rec
    field CInt32
  END CSTRUCT

  FUNC get() AS Rec
    SYMBOL \"getpid\"
    ABI (out OUT S) AS status CInt32
    RETURN out
  END FUNC
END LINK

FUNC main() AS Integer
  LET r = libc::get()
  io::print(toString(r.field))
  RETURN 0
END FUNC
";

/// The same struct bound IN from a wrapper parameter.
const IN_STRUCT: &str = "\
IMPORT io

TYPE Rec
  field AS Integer
END TYPE

LINK \"c\" AS libc
  CSTRUCT S AS Rec
    field CInt32
  END CSTRUCT

  FUNC put(v AS Integer) AS Integer
    SYMBOL \"getpid\"
    ABI (s IN S) AS status CInt32
    BIND IN s
      field = v
    END BIND
    RETURN status
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::put(1)))
  RETURN 0
END FUNC
";

/// A `CString` field bound IN from a `String` parameter.
const IN_CSTRING: &str = "\
IMPORT io

TYPE Rec
  field AS String
END TYPE

LINK \"c\" AS libc
  CSTRUCT S AS Rec
    field CString
  END CSTRUCT

  FUNC put(v AS String) AS Integer
    SYMBOL \"getpid\"
    ABI (s IN S) AS status CInt32
    BIND IN s
      field = v
    END BIND
    RETURN status
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::put(\"x\")))
  RETURN 0
END FUNC
";

/// A direct `CInt32`-style return, for the return-marshalling refusal.
const DIRECT_RETURN: &str = "\
IMPORT io

LINK \"c\" AS libc
  FUNC pid() AS Integer
    SYMBOL \"getpid\"
    ABI () AS status CInt64
    RETURN status
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::pid()))
  RETURN 0
END FUNC
";

/// A pinned `CInt32` argument.
const CONST_PIN: &str = "\
IMPORT io

LINK \"c\" AS libc
  FUNC pin() AS Integer
    SYMBOL \"getpid\"
    ABI (x CInt32) AS status CInt32
    CONST x = 1
    RETURN status
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::pin()))
  RETURN 0
END FUNC
";

fn module(source: &str, target: CodeTarget) -> NirModule {
    nir_for_linking_src(source, target, &["c"]).unwrap_or_else(|err| panic!("NIR: {err}"))
}

/// Lower `source` after `corrupt`, expecting a refusal that contains `wanted`.
fn refused(source: &str, corrupt: impl Fn(&mut NirModule), wanted: &str) {
    for target in [CodeTarget::MacosAarch64, CodeTarget::LinuxX86_64] {
        let pristine = module(source, target);
        code_for_nir(&pristine, target)
            .unwrap_or_else(|err| panic!("the uncorrupted program must lower: {err}"));
        let mut broken = module(source, target);
        corrupt(&mut broken);
        let err = match code_for_nir(&broken, target) {
            Ok(_) => panic!("{}: a corrupted declaration must be refused", target.name()),
            Err(err) => err,
        };
        assert!(
            err.contains(wanted),
            "{}: the refusal must say {wanted:?}; it said {err:?}",
            target.name()
        );
    }
}

#[test]
fn a_bind_in_field_the_struct_does_not_have_is_refused() {
    refused(
        IN_STRUCT,
        |module| module.link_functions[0].bind_in[0].fields[0].name = "ghost".to_string(),
        "BIND IN sets unknown field 'ghost' of CSTRUCT 'S'",
    );
}

#[test]
fn a_bind_in_field_bound_to_nothing_is_refused() {
    refused(
        IN_STRUCT,
        |module| module.link_functions[0].bind_in[0].fields[0].param = None,
        "binds neither a parameter nor a literal",
    );
}

#[test]
fn a_bind_in_field_bound_to_an_unknown_parameter_is_refused() {
    refused(
        IN_STRUCT,
        |module| {
            module.link_functions[0].bind_in[0].fields[0].param = Some("ghost".to_string());
        },
        "binds field 'field' to unknown parameter 'ghost'",
    );
}

#[test]
fn a_cstring_field_bound_to_an_unknown_parameter_is_refused() {
    refused(
        IN_CSTRING,
        |module| {
            module.link_functions[0].bind_in[0].fields[0].param = Some("ghost".to_string());
        },
        "binds field 'field' to unknown parameter 'ghost'",
    );
}

#[test]
fn a_struct_field_of_an_unknown_c_type_is_refused() {
    refused(
        IN_STRUCT,
        |module| module.link_cstructs[0].fields[0].ctype = ParameterType::declared("CWat"),
        "CWat",
    );
}

#[test]
fn an_out_struct_whose_record_has_no_layout_is_refused() {
    refused(
        OUT_STRUCT,
        |module| module.link_cstructs[0].maps_to = ParameterType::declared("Ghost"),
        "whose record 'Ghost' has no field layout",
    );
}

#[test]
fn an_out_struct_record_field_with_no_struct_field_is_refused() {
    refused(
        OUT_STRUCT,
        |module| module.link_cstructs[0].fields[0].name = "other".to_string(),
        "field 'field' has no CSTRUCT field",
    );
}

#[test]
fn a_return_of_an_unknown_c_type_is_refused() {
    refused(
        DIRECT_RETURN,
        |module| module.link_functions[0].abi_return_ctype = ParameterType::declared("CWat"),
        "has unknown ABI return ctype 'CWat'",
    );
}

/// A `CBuffer` slot with no `BUFFER SIZE` clause is refused rather than allocated
/// with no length.
#[test]
fn a_cbuffer_slot_with_no_size_clause_is_refused() {
    let source = fixture_src("native-cbuffer-read-rt");
    refused(
        &source,
        |module| {
            for function in &mut module.link_functions {
                function.buffers.clear();
            }
        },
        "has no BUFFER SIZE clause",
    );
}

/// A `CONST` pin must fit its `CInt32` slot: a value that does not is refused at
/// compile time rather than truncated to its low 32 bits.
#[test]
fn a_const_pin_outside_the_cint32_range_is_refused() {
    refused(
        CONST_PIN,
        |module| module.link_functions[0].consts[0].1 = i64::from(i32::MAX) + 1,
        "does not fit the signed 32-bit range of its CInt32 ABI slot",
    );
}

/// An ABI slot fed by no parameter, no pin and no `OUT` direction has nothing to
/// pass: refused, not staged from whatever the frame held.
#[test]
fn an_abi_slot_with_no_source_is_refused() {
    refused(
        CONST_PIN,
        |module| module.link_functions[0].consts.clear(),
        "ABI slot 'x' has no source (param, const, or OUT)",
    );
}

/// A direct return of a type that has no return-side meaning (`CString`) is refused.
#[test]
fn a_direct_cstring_return_is_refused() {
    refused(
        DIRECT_RETURN,
        |module| module.link_functions[0].abi_return_ctype = ParameterType::declared("CString"),
        "has unknown ABI return ctype 'CString'",
    );
}
