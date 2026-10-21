//! Every C width a `LINK` value crosses the ABI at is converted by its own arm.
//!
//! A `LINK` function's direct return, its parameters and the fields of a `CSTRUCT`
//! are C values of a declared width; MFBASIC reads and writes 8-byte words, a
//! `Boolean` that is exactly 0 or 1 and a `Float` that is finite. The thunk
//! (`link/thunk/link_thunk/{value_marshal,struct_marshal}.rs`) therefore converts
//! on the way across — and each conversion is a WRONG VALUE rather than a failure to
//! build when it is missing: a `CBool` that is `TRUE` for every non-zero byte
//! pattern, a `CByte` carrying the seven bytes the callee never wrote, a
//! `CFloat` whose 32 bits are read as the low half of a double, a `CFloat`
//! parameter handed to C as a double.
//!
//! `link_out_widths.rs` pins the `OUT`-parameter arms; this file pins the others, in
//! the same way: a marker the arm emits and nothing else does, asserted present for
//! its own C type and absent for every other, so the table says WHICH arm ran.

use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{try_code_for_linking_src, CodeTarget};

/// Every operand and mnemonic the link thunk code emitted, as text.
///
/// Filtered by the emitting file for the reason `link_out_widths` gives: a program
/// this size carries hundreds of instructions from string handling and the entry
/// stub, and `CodeInstruction::source` attributes an instruction to whoever asked
/// for it.
fn thunk_text(plan: &NativeCodePlan) -> Vec<String> {
    plan.functions
        .iter()
        .flat_map(|function| function.instructions.iter())
        .filter(|instruction| {
            instruction.source.is_some_and(|location| {
                let file = location.file();
                file.contains("link_thunk/") || file.contains("link_thunk\\")
            })
        })
        .flat_map(|instruction| {
            std::iter::once(instruction.op.mnemonic().to_string()).chain(
                instruction
                    .fields
                    .iter()
                    .map(|(_, operand)| operand.render()),
            )
        })
        .collect()
}

fn lowered(source: &str, target: CodeTarget) -> NativeCodePlan {
    try_code_for_linking_src(source, target, &["c"])
        .unwrap_or_else(|err| panic!("{} must lower on {}: {err}", source, target.name()))
}

fn present(text: &[String], marker: &str) -> bool {
    text.iter().any(|operand| operand.contains(marker))
}

/// A one-function program whose ABI returns `ctype` directly.
fn return_program(ctype: &str, mfb_type: &str) -> String {
    format!(
        "IMPORT io\n\
         \n\
         LINK \"c\" AS libc\n\
         \x20 FUNC fetch() AS {mfb_type}\n\
         \x20   SYMBOL \"getpid\"\n\
         \x20   ABI () AS status {ctype}\n\
         \x20   RETURN status\n\
         \x20 END FUNC\n\
         END LINK\n\
         \n\
         FUNC main() AS Integer\n\
         \x20 io::print(toString(libc::fetch()))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// A one-function program with one `Float` parameter passed as `ctype`.
fn float_param_program(ctype: &str) -> String {
    format!(
        "IMPORT io\n\
         \n\
         LINK \"c\" AS libc\n\
         \x20 FUNC put(x AS Float) AS Integer\n\
         \x20   SYMBOL \"getpid\"\n\
         \x20   ABI (x {ctype}) AS status CInt32\n\
         \x20   RETURN status\n\
         \x20 END FUNC\n\
         END LINK\n\
         \n\
         FUNC main() AS Integer\n\
         \x20 io::print(toString(libc::put(1.5)))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// A one-function program whose `OUT` parameter is read back as `ctype`.
fn out_program(ctype: &str) -> String {
    format!(
        "IMPORT io\n\
         \n\
         LINK \"c\" AS libc\n\
         \x20 FUNC fetch() AS Float\n\
         \x20   SYMBOL \"getpid\"\n\
         \x20   ABI (slot OUT {ctype}) AS status CInt32\n\
         \x20   RETURN slot\n\
         \x20 END FUNC\n\
         END LINK\n\
         \n\
         FUNC main() AS Integer\n\
         \x20 io::print(toString(libc::fetch()))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// `(C return type, MFBASIC type, markers its arm emits, markers it must not)`.
const RETURNS: &[(&str, &str, &[&str], &[&str])] = &[
    // Normalized to 0 or 1 through a branch: any non-zero byte pattern is TRUE.
    (
        "CBool",
        "Boolean",
        &["_bool_true"],
        &["fcvt_d_from_s", "_float_finite"],
    ),
    // Masked to its low byte: the other seven belong to whatever was there before.
    (
        "CByte",
        "Integer",
        &["255"],
        &["_bool_true", "fcvt_d_from_s", "_float_finite"],
    ),
    // Widened from 32 bits and checked finite: an Inf or NaN is refused.
    (
        "CFloat",
        "Float",
        &["fcvt_d_from_s", "_float_finite"],
        &["_bool_true"],
    ),
    (
        "CDouble",
        "Float",
        &["_float_finite"],
        &["fcvt_d_from_s", "_bool_true"],
    ),
    // The integers carry no return-side normalization: the C ABI already delivers
    // them in the low bits and the wrapper type is Integer.
    (
        "CInt8",
        "Integer",
        &[],
        &["_bool_true", "fcvt_d_from_s", "_float_finite", "255"],
    ),
    (
        "CInt16",
        "Integer",
        &[],
        &["_bool_true", "fcvt_d_from_s", "_float_finite", "255"],
    ),
    (
        "CInt64",
        "Integer",
        &[],
        &["_bool_true", "fcvt_d_from_s", "_float_finite", "255"],
    ),
    (
        "CUInt8",
        "Integer",
        &[],
        &["_bool_true", "fcvt_d_from_s", "_float_finite", "255"],
    ),
    (
        "CUInt16",
        "Integer",
        &[],
        &["_bool_true", "fcvt_d_from_s", "_float_finite", "255"],
    ),
    (
        "CUInt32",
        "Integer",
        &[],
        &["_bool_true", "fcvt_d_from_s", "_float_finite", "255"],
    ),
    (
        "CUInt64",
        "Integer",
        &[],
        &["_bool_true", "fcvt_d_from_s", "_float_finite", "255"],
    ),
];

/// A direct return is converted by its own C type's arm, and only that arm.
#[test]
fn a_direct_return_is_converted_by_its_c_type() {
    for (ctype, mfb_type, wanted, unwanted) in RETURNS {
        let plan = lowered(&return_program(ctype, mfb_type), CodeTarget::MacosAarch64);
        let text = thunk_text(&plan);
        for marker in *wanted {
            assert!(
                present(&text, marker),
                "a direct {ctype} return must convert through {marker:?}"
            );
        }
        for marker in *unwanted {
            assert!(
                !present(&text, marker),
                "a direct {ctype} return emitted {marker:?}, which belongs to another width"
            );
        }
    }
}

/// A `CFloat` parameter is narrowed from the double MFBASIC holds, and refused when
/// the narrowing overflows to infinity; a `CDouble` parameter is passed as is.
#[test]
fn a_cfloat_parameter_is_narrowed_and_a_cdouble_is_not() {
    let float = thunk_text(&lowered(
        &float_param_program("CFloat"),
        CodeTarget::MacosAarch64,
    ));
    assert!(
        present(&float, "fcvt_s_from_d"),
        "CFloat must narrow to 32 bits"
    );
    assert!(
        present(&float, "float_overflow_fail"),
        "a Float too large for 32 bits must be refused, not passed as infinity"
    );
    let double = thunk_text(&lowered(
        &float_param_program("CDouble"),
        CodeTarget::MacosAarch64,
    ));
    assert!(
        !present(&double, "fcvt_s_from_d"),
        "a CDouble is not narrowed"
    );
}

/// A `CFloat` `OUT` slot is widened from its 32 bits.
#[test]
fn a_cfloat_out_slot_is_widened() {
    let text = thunk_text(&lowered(&out_program("CFloat"), CodeTarget::MacosAarch64));
    assert!(
        present(&text, "fcvt_d_from_s"),
        "the callee wrote 32 bits; reading them as a double is a wrong value"
    );
}

/// Every width lowers on every backend.
#[test]
fn every_value_width_lowers_on_every_backend() {
    for (ctype, mfb_type, ..) in RETURNS {
        for target in CodeTarget::ALL {
            lowered(&return_program(ctype, mfb_type), target);
        }
    }
    for target in CodeTarget::ALL {
        lowered(&float_param_program("CFloat"), target);
        lowered(&out_program("CFloat"), target);
    }
}

// --- CSTRUCT fields ----------------------------------------------------------

/// A program with a `CSTRUCT` of one `field` of `ctype`, mapped to a record field of
/// `mfb_type`: `get` reads it back through an `OUT` slot, `put` binds it from a
/// parameter and, given a `literal`, `lit` binds it from that.
fn struct_program(ctype: &str, mfb_type: &str, literal: Option<&str>) -> String {
    let lit = literal
        .map(|literal| {
            format!(
                "\n\
                 \x20 FUNC lit() AS Integer\n\
                 \x20   SYMBOL \"getpid\"\n\
                 \x20   ABI (s IN S) AS status CInt32\n\
                 \x20   BIND IN s\n\
                 \x20     field = {literal}\n\
                 \x20   END BIND\n\
                 \x20   RETURN status\n\
                 \x20 END FUNC\n"
            )
        })
        .unwrap_or_default();
    format!(
        "IMPORT io\n\
         \n\
         TYPE Rec\n\x20 field AS {mfb_type}\nEND TYPE\n\
         \n\
         LINK \"c\" AS libc\n\
         \x20 CSTRUCT S AS Rec\n\x20   field {ctype}\n\x20 END CSTRUCT\n\
         \n\
         \x20 FUNC get() AS Rec\n\
         \x20   SYMBOL \"getpid\"\n\
         \x20   ABI (out OUT S) AS status CInt32\n\
         \x20   RETURN out\n\
         \x20 END FUNC\n\
         \n\
         \x20 FUNC put(v AS {mfb_type}) AS Integer\n\
         \x20   SYMBOL \"getpid\"\n\
         \x20   ABI (s IN S) AS status CInt32\n\
         \x20   BIND IN s\n\
         \x20     field = v\n\
         \x20   END BIND\n\
         \x20   RETURN status\n\
         \x20 END FUNC\n\
         {lit}\
         END LINK\n\
         \n\
         FUNC main() AS Integer\n\
         \x20 LET r = libc::get()\n\
         \x20 io::print(toString(r.field))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// The marker the READ arm of one field type emits and nothing else does.
const FIELD_READS: &[(&str, &str, &str, &str)] = &[
    // (C type, MFBASIC type, a bind-in literal, the read arm's marker)
    ("CInt8", "Integer", "3", "ldr_u8"),
    ("CInt16", "Integer", "3", "ldr_u16"),
    ("CBool", "Boolean", "1", "_sf0_true"),
    ("CFloat", "Float", "1", "fcvt_d_from_s"),
];

/// A struct field is read back by its own width, and only that.
#[test]
fn a_struct_field_is_read_back_by_its_c_type() {
    let plans: Vec<(&str, Vec<String>)> = FIELD_READS
        .iter()
        .map(|(ctype, mfb_type, literal, _)| {
            let plan = lowered(
                &struct_program(ctype, mfb_type, Some(literal)),
                CodeTarget::MacosAarch64,
            );
            (*ctype, thunk_text(&plan))
        })
        .collect();
    for (ctype, _, _, marker) in FIELD_READS {
        for (other, text) in &plans {
            if other == ctype {
                assert!(
                    present(text, marker),
                    "a {ctype} field must be read through {marker:?}"
                );
            } else if *marker != "ldr_u8" && *marker != "ldr_u16" {
                // The sub-word loads are also how other widths' tails are read, so
                // only the conversions are exclusive.
                assert!(
                    !present(text, marker),
                    "a {other} field emitted {marker:?}, which belongs to {ctype}"
                );
            }
        }
    }
}

/// A struct field bound from a parameter is narrowed (`CFloat`) or copied as a
/// NUL-terminated string (`CString`); one bound from a literal is stored as the C
/// bits of the literal, with neither.
#[test]
fn a_struct_field_bound_in_is_converted_from_a_parameter_and_stored_from_a_literal() {
    let float = thunk_text(&lowered(
        &struct_program("CFloat", "Float", Some("1")),
        CodeTarget::MacosAarch64,
    ));
    assert!(
        present(&float, "fcvt_s_from_d"),
        "a CFloat field bound from a parameter must be narrowed to 32 bits"
    );
    let string = thunk_text(&lowered(
        &struct_program("CString", "String", None),
        CodeTarget::MacosAarch64,
    ));
    assert!(
        present(&string, "_copy"),
        "a CString field bound from a String parameter is copied to a C string"
    );
}

/// A literal bound into a `CString` field is refused by name: a C string is a copy of a
/// `String` parameter, and a number has no string to copy.
#[test]
fn a_cstring_field_cannot_bind_a_literal() {
    let source = struct_program("CString", "String", Some("1"));
    for target in CodeTarget::ALL {
        let err = match try_code_for_linking_src(&source, target, &["c"]) {
            Ok(_) => panic!(
                "a literal into a CString field must be refused on {}",
                target.name()
            ),
            Err(err) => err,
        };
        assert!(
            err.contains("must bind a String parameter, not a literal"),
            "{}: the refusal must say what the field needs: {err}",
            target.name()
        );
    }
}

/// Every field width lowers on every backend.
#[test]
fn every_struct_field_width_lowers_on_every_backend() {
    for (ctype, mfb_type, literal, _) in FIELD_READS {
        for target in CodeTarget::ALL {
            lowered(&struct_program(ctype, mfb_type, Some(literal)), target);
        }
    }
    for target in CodeTarget::ALL {
        lowered(&struct_program("CString", "String", None), target);
    }
}

// --- expressions -------------------------------------------------------------

/// `SUCCESS_ON` and `RETURN` over every comparison and over subtraction.
const EXPRESSIONS: &str = "\
IMPORT io

LINK \"c\" AS libc
  FUNC pid() AS Integer
    SYMBOL \"getpid\"
    ABI () AS status CInt64
    SUCCESS_ON status > 0 AND status < 99999999 AND status >= 1 AND status <= 99999999
    RETURN status - 1
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::pid()))
  RETURN 0
END FUNC
";

/// Each comparison operator of a `SUCCESS_ON` branches on its own condition, and a
/// `RETURN` expression computes with the operator it names.
#[test]
fn link_expressions_compile_each_operator_to_its_own_instruction() {
    let text = thunk_text(&lowered(EXPRESSIONS, CodeTarget::MacosAarch64));
    for mnemonic in ["b.lt", "b.gt", "b.le", "b.ge", "sub"] {
        assert!(
            present(&text, mnemonic),
            "the expression uses an operator that compiles to `{mnemonic}`"
        );
    }
    for target in CodeTarget::ALL {
        lowered(EXPRESSIONS, target);
    }
}
