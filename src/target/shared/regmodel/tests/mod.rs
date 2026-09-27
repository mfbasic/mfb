//! The three [`RegisterModel`] methods that carry a default body.
//!
//! Every shipping backend overrides all three, so their defaults had never run
//! — and a default nothing executes is a default nobody has read. Each one is a
//! decision about what an ISA gets when it says nothing, and each one is wrong
//! in a way that would not fail to build:
//!
//! * `spill_slot_bytes` is the stride between spill slots. The scalar `8` is
//!   right for an ISA whose widest spill is a word and catastrophic for one
//!   whose FP spill is a 128-bit vector — the second half of every vector would
//!   land in the next slot. Both shipping backends override it to 16 for exactly
//!   that reason, which is why the fallback is untested and why it has to stay
//!   the conservative one.
//! * `math_pool_base` is `None`: an ISA that has not nominated a spare physical
//!   for the arena base gets a vreg, not a guessed register.
//! * `external_int_argument_registers` falls back to the compiler's own neutral
//!   count, which is right for AAPCS64 and riscv64 and wrong for SysV x86-64 —
//!   which is precisely why x86-64 overrides it (bug-296).
//!
//! The model here implements only the seven REQUIRED methods, so the defaults
//! are what a new backend would actually get on its first day.

use super::{RegClass, RegisterModel};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;

/// A backend that has said the minimum the trait demands and nothing more.
struct SilentIsa;

/// An instruction naming the physical registers the allocator handed a
/// spill/reload/move hook. They are written as the fields' operands directly:
/// `abi::`'s constructors take no register text (bug-721), and the typed
/// physical door is confined to the backends.
fn physical(op: &str, fields: Vec<(&'static str, Operand)>) -> CodeInstruction {
    let mut instruction = CodeInstruction::new(op);
    instruction.fields = fields;
    instruction
}

impl RegisterModel for SilentIsa {
    fn allocatable(&self, _class: RegClass) -> &'static [&'static str] {
        &["r0"]
    }

    fn class_of(&self, _reg: &str) -> Option<RegClass> {
        Some(RegClass::Int)
    }

    fn is_callee_saved(&self, _reg: &str) -> bool {
        false
    }

    fn caller_saved(&self, _class: RegClass) -> &'static [&'static str] {
        &["r0"]
    }

    fn emit_spill(&self, _class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        physical(
            "str_u64",
            vec![
                ("src", Operand::from(reg)),
                ("base", abi::stack_pointer().into()),
                ("offset", Operand::from(offset.to_string())),
            ],
        )
    }

    fn emit_reload(&self, _class: RegClass, reg: &str, offset: usize) -> CodeInstruction {
        physical(
            "ldr_u64",
            vec![
                ("dst", Operand::from(reg)),
                ("base", abi::stack_pointer().into()),
                ("offset", Operand::from(offset.to_string())),
            ],
        )
    }

    fn emit_move(&self, dst: &str, src: &str) -> CodeInstruction {
        physical(
            "mov",
            vec![("dst", Operand::from(dst)), ("src", Operand::from(src))],
        )
    }

    fn arena_base(&self) -> &'static str {
        "r1"
    }

    fn closure_env(&self) -> &'static str {
        "r2"
    }

    fn current_thread(&self) -> &'static str {
        "r3"
    }
}

#[test]
fn an_isa_that_overrides_nothing_gets_the_conservative_defaults() {
    let isa = SilentIsa;

    assert_eq!(
        isa.spill_slot_bytes(),
        8,
        "the default spill stride is the SCALAR width: an ISA that has not said \
         its widest spill is a vector must not be given a 16-byte stride it does \
         not fill, and one that spills vectors must override this or its second \
         lane lands in the next slot"
    );
    assert_eq!(
        isa.math_pool_base(),
        None,
        "an ISA that has not nominated a spare physical for the arena base gets \
         a vreg the allocator places, not a register the shared code guessed"
    );
    assert_eq!(
        isa.external_int_argument_registers(),
        abi::REGISTER_ARGUMENT_COUNT,
        "the default external-C argument count is the compiler's own neutral \
         count -- right for AAPCS64 and riscv64, and the reason SysV x86-64 has \
         to override it (bug-296): its sixth argument is the last in a register"
    );
}
