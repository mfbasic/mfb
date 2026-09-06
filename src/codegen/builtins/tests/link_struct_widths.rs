//! A `CSTRUCT` whose fields are not all 8 bytes wide.
//!
//! `link_thunk.rs` zeroes an `IN`/`OUT` struct buffer before use, and it does
//! that a word at a time with a tail: `store_u32`, `store_u16`, `store_u8` for
//! whatever is left over. Those three stores are only reached by a struct whose
//! SIZE is not a multiple of 8, and every committed `CSTRUCT` fixture describes
//! a struct that is — `struct timespec` is two `CInt64`s.
//!
//! Zeroing the buffer is not tidiness. A `BIND IN` writes only the fields it
//! names, and the C callee reads the whole struct: whatever the arena last left
//! in the unwritten bytes is what it sees. `native-struct-scalar-rt`'s own
//! comment says so — "`seconds` stays 0 because the whole buffer is zeroed
//! first". A tail the zeroing missed is a field the callee reads as garbage,
//! intermittently.
//!
//! Two structs, because one cannot reach all three widths. The zeroing takes
//! the widest store that fits what is left, so a 7-byte struct is a 4 and then
//! three BYTES — the 2-byte store is only reached when exactly 2 remain, which
//! is the 6-byte struct.

use crate::testutil::{try_code_for_linking_src, CodeTarget};

/// A `CSTRUCT` of 4 + 2 + 1 bytes, filled by `BIND IN`.
///
/// The symbol is `getpid`, which takes no arguments and exists everywhere; the
/// struct never reaches it (the ABI list is what matters here, and nothing is
/// executed). What is under test is the thunk the compiler builds around the
/// call, not the call.
const NARROW_STRUCT: &str = "\
IMPORT io

TYPE Narrow
  code AS Integer
  flags AS Integer
  level AS Integer
END TYPE

TYPE Even
  flags AS Integer
END TYPE

LINK \"c\" AS libc
  ' 7 bytes: code@0 (4), flags@4 (2), level@6 (1). Not a multiple of 8, so the
  ' buffer zeroing needs a 4-byte, a 2-byte and a 1-byte store in its tail.
  CSTRUCT NarrowStruct AS Narrow
    code  CInt32
    flags CInt16
    level CUInt8
  END CSTRUCT

  ' 2 bytes. The zeroing takes the widest store that fits what is LEFT, so a
  ' size of exactly 2 is the shortest way to reach the 16-bit one.
  CSTRUCT EvenStruct AS Even
    flags CInt16
  END CSTRUCT

  FUNC pack(value AS Integer) AS Integer
    SYMBOL \"getpid\"
    ABI (n IN NarrowStruct) AS status CInt32
    BIND IN n
      code = value
    END BIND
    RETURN status
  END FUNC

  FUNC packEven(value AS Integer) AS Integer
    SYMBOL \"getpid\"
    ABI (n IN EvenStruct) AS status CInt32
    BIND IN n
      flags = value
    END BIND
    RETURN status
  END FUNC
END LINK

FUNC main() AS Integer
  io::print(toString(libc::pack(3)))
  io::print(toString(libc::packEven(4)))
  RETURN 0
END FUNC
";

/// A struct whose size is not a multiple of 8 zeroes its tail byte by byte.
///
/// The narrow stores ARE the tail. Asserting the 4- and 2-byte ones pins the
/// structs' actual widths: a 7-byte buffer zeroed only in 8-byte steps would
/// write 8 and run one byte past it, and a 6-byte one zeroed only in 4-byte
/// steps would leave two bytes of whatever the arena held.
///
/// The 8-bit store is not asserted. It appears in this program many times over
/// from string handling, so its presence says nothing about the tail — which is
/// the difference between an assertion and a coincidence.
#[test]
fn a_struct_that_is_not_a_multiple_of_eight_zeroes_its_tail() {
    use crate::arch::ops::CodeOp;

    // Through the LINKING entry point: a `LINK "c"` needs a locator for the
    // target, and a source string has no manifest to carry one. That refusal is
    // correct (bug-549's neighbour, NATIVE_LIBRARY_NO_MATCH) and is tested in
    // `fixture_projects.rs`; here it is just in the way.
    let plan = try_code_for_linking_src(NARROW_STRUCT, CodeTarget::LinuxX86_64, &["c"])
        .expect("the LINK program must lower");
    let mut widths = Vec::new();
    for instruction in plan.functions.iter().flat_map(|f| f.instructions.iter()) {
        match instruction.op {
            CodeOp::StrU32 => widths.push(32),
            CodeOp::StrU16 => widths.push(16),
            CodeOp::StrU8 => widths.push(8),
            _ => {}
        }
    }
    for width in [32u32, 16] {
        assert!(
            widths.contains(&width),
            "a 7-byte `CSTRUCT` buffer is zeroed with a {width}-bit store in its \
             tail; without it the C callee reads whatever the arena last left \
             in those bytes, and a `BIND IN` that names one field leaves the \
             rest to that zeroing. Widths emitted: {widths:?}"
        );
    }
}

/// The narrow struct lowers on every backend.
///
/// The field offsets and the tail stores are laid out by shared codegen but
/// emitted per-ISA, and a backend without a 16-bit store would have to
/// synthesise one.
#[test]
fn a_narrow_link_struct_lowers_on_every_backend() {
    for target in CodeTarget::ALL {
        try_code_for_linking_src(NARROW_STRUCT, target, &["c"])
            .unwrap_or_else(|err| panic!("a 7-byte CSTRUCT on {}: {err}", target.name()));
    }
}
