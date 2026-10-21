//! The frame (unboxed) form of an inline-`TRAP` temporary reads a `Nothing` payload
//! as the constant `NOTHING`.
//!
//! `LET r = f() TRAP(e) … END TRAP` binds `r` to the trapped call's success value
//! through an unboxed temporary (`error/result/unboxed_trap.rs`, bug-778). Its
//! payload slot holds the producer's raw success word — but a `Nothing`-returning
//! producer has no success word to leave there, so the read must be the one
//! `Nothing` value, `0` (bug-744), and not a load of whatever the slot holds.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::CodeInstruction;
use crate::testutil::{code_for_src_on, code_function, CodeTarget};

/// `fs::deleteFile` returns `Nothing` and fails; its success value is bound.
const NOTHING_PAYLOAD: &str = "\
IMPORT fs
IMPORT io

FUNC main() AS Integer
  LET r = fs::deleteFile(\"gone.txt\") TRAP(e)
    io::print(\"trapped\")
    RETURN 1
  END TRAP
  io::print(\"ok\")
  RETURN 0
END FUNC
";

/// The same shape with an `Integer` payload, whose success word IS in the slot.
const INTEGER_PAYLOAD: &str = "\
IMPORT io

FUNC main() AS Integer
  LET r = toInt(\"12x\") TRAP(e)
    io::print(\"trapped\")
    RETURN 1
  END TRAP
  io::print(toString(r))
  RETURN 0
END FUNC
";

fn from_unboxed_trap(instruction: &CodeInstruction) -> bool {
    instruction.source.is_some_and(|at| {
        at.file()
            .replace('\\', "/")
            .ends_with("error/result/unboxed_trap.rs")
    })
}

/// `mov_imm <reg>, Integer 0` emitted by the unboxed form itself.
fn reads_nothing_constant(source: &str, target: CodeTarget) -> bool {
    let plan = code_for_src_on(source, target);
    code_function(&plan, "main")
        .instructions
        .iter()
        .any(|instruction| {
            from_unboxed_trap(instruction)
                && instruction.op == CodeOp::MovImm
                && instruction.get("type").as_deref() == Some("Integer")
                && instruction.get("value").as_deref() == Some("0")
        })
}

#[test]
fn an_unboxed_nothing_payload_reads_as_the_nothing_constant() {
    for target in CodeTarget::ALL {
        assert!(
            reads_nothing_constant(NOTHING_PAYLOAD, target),
            "{}: the bound success value of a trapped Nothing call is NOTHING (0)",
            target.name()
        );
        assert!(
            !reads_nothing_constant(INTEGER_PAYLOAD, target),
            "{}: an Integer payload is read from its slot, not replaced by 0",
            target.name()
        );
    }
}
