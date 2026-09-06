//! A `CSTRUCT` buffer is zeroed with a tail, and the tail's widths follow its
//! SIZE.
//!
//! `link_thunk.rs::lower_link_thunk` zeroes a struct slot's buffer before the
//! `BIND IN` fields are written into it, widest store first:
//!
//! ```text
//! while z + 8 <= size { store_u64 }   while z + 4 <= size { store_u32 }
//! while z + 2 <= size { store_u16 }   while z     <  size { store_u8  }
//! ```
//!
//! Zeroing is not tidiness. `BIND IN` writes only the fields it names and the C
//! callee reads the whole struct, so every unnamed byte is whatever the arena
//! last left there — `native-struct-scalar-rt`'s own comment says as much
//! ("`seconds` stays 0 because the whole buffer is zeroed first"). A tail the
//! zeroing missed is a field the callee reads as garbage, intermittently.
//!
//! **The size is not the sum of the field widths.** `compute_c_layout` rounds
//! up to the struct's alignment, which is its widest member's:
//! `CInt32 + CInt16 + CUInt8` occupies 7 bytes and is a struct of **8**, so it
//! is zeroed by the `u64` loop alone and reaches no tail at all. Reaching a
//! given tail store therefore means choosing an alignment as well as a size,
//! and the only way to a size of 3 or 5 is all-`CUInt8` fields.
//!
//! Which is what the first version of this suite got wrong; see the counting
//! note on [`thunk_narrow_stores`].

use std::collections::BTreeMap;

use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{try_code_for_linking_src, CodeTarget};

/// A one-function `LINK` program whose `CSTRUCT` has exactly `fields`, with the
/// first one bound by `BIND IN`.
///
/// The symbol is `getpid`, which takes no arguments and exists everywhere; the
/// struct never reaches it. What is under test is the thunk the compiler builds
/// around the call, not the call.
fn program(fields: &[(&str, &str)]) -> String {
    let record: String = fields
        .iter()
        .map(|(name, _)| format!("  {name} AS Integer\n"))
        .collect();
    let cstruct: String = fields
        .iter()
        .map(|(name, ctype)| format!("    {name} {ctype}\n"))
        .collect();
    let bound = fields[0].0;
    format!(
        "IMPORT io\n\
         \n\
         TYPE Rec\n{record}END TYPE\n\
         \n\
         LINK \"c\" AS libc\n\
         \x20 CSTRUCT S AS Rec\n{cstruct}\x20 END CSTRUCT\n\
         \n\
         \x20 FUNC pack(value AS Integer) AS Integer\n\
         \x20   SYMBOL \"getpid\"\n\
         \x20   ABI (n IN S) AS status CInt32\n\
         \x20   BIND IN n\n\
         \x20     {bound} = value\n\
         \x20   END BIND\n\
         \x20   RETURN status\n\
         \x20 END FUNC\n\
         END LINK\n\
         \n\
         FUNC main() AS Integer\n\
         \x20 io::print(toString(libc::pack(3)))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// Sub-word stores emitted **by `link_thunk.rs`**, counted by width.
///
/// The filter on the emitting file is the whole point. A program this size
/// carries about twenty `store_u8`s from string handling alone, so
/// "the plan contains a `StrU8`" is true of every program ever compiled and
/// says nothing about a struct's tail. The first version of this suite asserted
/// exactly that, and passed against a struct whose tail never ran — the
/// `StrU32` it was reading came from `store_field` marshalling the bound
/// `CInt32`, four hundred lines away in the same file. Recorded as C10.
///
/// `CodeInstruction::source` is the `#[track_caller]` location of the builder
/// call. It is audit-only metadata — never serialized, never affecting emitted
/// bytes — which makes it exactly the right thing to key a test on: it
/// distinguishes two identical instructions by who asked for them, and it
/// cannot drift the way a line number can.
fn thunk_narrow_stores(plan: &NativeCodePlan) -> BTreeMap<u32, usize> {
    use crate::arch::ops::CodeOp;

    let mut counts = BTreeMap::new();
    for instruction in plan.functions.iter().flat_map(|f| f.instructions.iter()) {
        let width = match instruction.op {
            CodeOp::StrU32 => 32,
            CodeOp::StrU16 => 16,
            CodeOp::StrU8 => 8,
            _ => continue,
        };
        if instruction
            .source
            .is_some_and(|location| location.file().ends_with("link_thunk.rs"))
        {
            *counts.entry(width).or_insert(0) += 1;
        }
    }
    counts
}

/// Each row: the struct's fields, its C layout, and the sub-word stores the
/// thunk must emit for it.
///
/// The expectation is **derived**, not recorded. Two things emit a sub-word
/// store in this thunk and both are predictable: the zeroing tail, from the
/// size; and `store_field` marshalling the one bound field, from that field's
/// width. So each row is `zeroing tail + one marshal store`, and the rows that
/// exercise a given tail width also differ in the marshal store, which is why
/// the whole multiset is asserted rather than "width W appears".
const LAYOUTS: &[(&str, &[(&str, &str)], usize, &[(u32, usize)])] = &[
    // size 8, align 8: the u64 loop consumes it. No tail, and the bound CInt64
    // marshals with a u64 store, so the thunk emits no sub-word store at all.
    // The baseline that makes the rows below mean something.
    ("one CInt64", &[("a", "CInt64")], 8, &[]),
    // size 5, align 1: 4 then 1. The only shape that reaches the 32-bit tail
    // store AND the 8-bit one.
    (
        "five CUInt8",
        &[
            ("a", "CUInt8"),
            ("b", "CUInt8"),
            ("c", "CUInt8"),
            ("d", "CUInt8"),
            ("e", "CUInt8"),
        ],
        5,
        // 32: the tail. 8: the tail's last byte, plus marshalling the CUInt8.
        &[(32, 1), (8, 2)],
    ),
    // size 3, align 1: 2 then 1.
    (
        "three CUInt8",
        &[("a", "CUInt8"), ("b", "CUInt8"), ("c", "CUInt8")],
        3,
        &[(16, 1), (8, 2)],
    ),
    // size 12, align 4: a u64 then a u32. The tail after a full word.
    (
        "three CInt32",
        &[("a", "CInt32"), ("b", "CInt32"), ("c", "CInt32")],
        12,
        // one tail u32, one marshalling the CInt32.
        &[(32, 2)],
    ),
    // size 2, align 2: the 16-bit store is the whole zeroing.
    ("one CInt16", &[("a", "CInt16")], 2, &[(16, 2)]),
];

/// The layout the rows above claim is the layout `compute_c_layout` computes.
///
/// Asserted separately because it is the premise of every expectation in the
/// table, and getting it wrong is not hypothetical: the previous version of
/// this suite called `CInt32 + CInt16 + CUInt8` a 7-byte struct.
#[test]
fn the_table_states_each_structs_real_c_layout() {
    use crate::types::ParameterType;

    for (label, fields, size, _) in LAYOUTS {
        let typed: Vec<(String, ParameterType)> = fields
            .iter()
            .map(|(name, ctype)| ((*name).to_string(), ParameterType::declared(ctype)))
            .collect();
        let layout = crate::ir::compute_c_layout(&typed, "linux-x86_64")
            .unwrap_or_else(|err| panic!("{label}: {err}"));
        assert_eq!(
            layout.size, *size,
            "{label}: a C struct's size is rounded up to its alignment \
             ({}), not the sum of its fields",
            layout.align
        );
    }
}

/// The buffer zeroing emits exactly the tail its size calls for.
#[test]
fn a_struct_buffer_is_zeroed_with_the_tail_its_size_calls_for() {
    for (label, fields, size, expected) in LAYOUTS {
        let plan = try_code_for_linking_src(&program(fields), CodeTarget::LinuxX86_64, &["c"])
            .unwrap_or_else(|err| panic!("{label}: {err}"));
        let expected: BTreeMap<u32, usize> = expected.iter().copied().collect();
        assert_eq!(
            thunk_narrow_stores(&plan),
            expected,
            "{label} is a {size}-byte struct: the zeroing takes the widest store \
             that fits what is left, and one more store marshals the bound \
             field. A buffer zeroed only in 8-byte steps writes past its own \
             end into the next one; one zeroed only in 4-byte steps leaves the \
             remainder holding whatever the arena did."
        );
    }
}

/// Every layout lowers on every backend.
///
/// The offsets and the tail stores are decided by shared codegen and emitted
/// per-ISA; a backend without a 16-bit store would have to synthesise one.
#[test]
fn every_struct_layout_lowers_on_every_backend() {
    for (label, fields, _, _) in LAYOUTS {
        for target in CodeTarget::ALL {
            try_code_for_linking_src(&program(fields), target, &["c"])
                .unwrap_or_else(|err| panic!("{label} on {}: {err}", target.name()));
        }
    }
}
