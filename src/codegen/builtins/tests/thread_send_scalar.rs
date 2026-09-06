//! Sending a SCALAR across a thread, which nothing did.
//!
//! `emit_thread_copy_real` dispatches the cross-arena deep copy on the message
//! type, and its first arm is the scalar one — `Nothing`, `Boolean`, `Byte`,
//! `Integer`, `Float`, `Fixed`, `Money`, `Scalar`. A scalar has no block to
//! copy: the arm is a register move, and the value goes over as itself.
//!
//! Every thread fixture in the tree sends a `String`, a collection, a record or
//! a resource — all of which are blocks — so the one arm that copies nothing was
//! the one nothing reached. It is three lines, and it is the arm every other
//! arm's correctness is measured against: if a scalar were routed to
//! `copy_flat_block` it would be allocated a block and the receiver would read a
//! pointer as an integer.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

/// One `thread::send` per scalar type the arm names.
///
/// A single program rather than one each: the arm is selected per MESSAGE type,
/// so one lowering exercises several branches of the `matches!`.
///
/// The workers ignore the message and return from their seed. `thread::read`
/// would be the natural thing to write and it does not monomorphize in a
/// single-source project — "native inlined field size not available for type
/// 'Msg'", the generic parameter reaching lowering unsubstituted. It is not
/// needed: the scalar copy is on the SEND, in `main`.
const SCALAR_SENDS: &str = "\
IMPORT io
IMPORT thread

ISOLATED FUNC counter(t AS ThreadWorker OF Integer TO Integer, seed AS Integer) AS Integer
  RETURN seed + 1
END FUNC

ISOLATED FUNC adder(t AS ThreadWorker OF Float TO Integer, seed AS Integer) AS Integer
  RETURN seed + 2
END FUNC

ISOLATED FUNC flagger(t AS ThreadWorker OF Boolean TO Integer, seed AS Integer) AS Integer
  RETURN seed + 3
END FUNC

ISOLATED FUNC byter(t AS ThreadWorker OF Byte TO Integer, seed AS Integer) AS Integer
  RETURN seed + 4
END FUNC

FUNC main() AS Integer
  LET ints AS Thread OF Integer TO Integer = thread::start(counter, 0)
  thread::send(ints, 1)
  thread::send(ints, 2)

  LET floats AS Thread OF Float TO Integer = thread::start(adder, 1)
  thread::send(floats, 2.5)

  LET flags AS Thread OF Boolean TO Integer = thread::start(flagger, 0)
  thread::send(flags, TRUE)

  LET bytes AS Thread OF Byte TO Integer = thread::start(byter, 0)
  thread::send(bytes, toByte(7))

  io::print(toString(thread::waitFor(ints)))
  io::print(toString(thread::waitFor(floats)))
  io::print(toString(thread::waitFor(flags)))
  io::print(toString(thread::waitFor(bytes)))
  RETURN 0
END FUNC
";

/// A scalar message lowers on every backend.
///
/// All five, because the copy is a register move and which register the message
/// arrives in is per-ABI: this is the arm where getting the convention wrong
/// produces a receiver that reads whatever the caller happened to leave there,
/// rather than a crash.
#[test]
fn sending_a_scalar_across_a_thread_lowers_on_every_backend() {
    for target in CodeTarget::ALL {
        try_code_for_src(SCALAR_SENDS, target, Console)
            .unwrap_or_else(|err| panic!("scalar `thread::send` on {}: {err}", target.name()));
    }
}

/// A scalar message is not given a block.
///
/// The point of the scalar arm. A `thread::send` of a `String` allocates a copy
/// in the sender's arena and hands the receiver the pointer; a scalar must not,
/// because there is nothing to copy and the receiver reads the value itself. If
/// the scalar were routed to `copy_flat_block` the receiver would read a pointer
/// as an integer.
#[test]
fn a_scalar_message_is_copied_as_a_value_not_as_a_block() {
    use crate::arch::ops::CodeOp;
    use crate::testutil::code_function;

    let plan = try_code_for_src(SCALAR_SENDS, CodeTarget::LinuxX86_64, Console)
        .expect("the program must lower");
    let main = code_function(&plan, "main");
    let allocs = main
        .instructions
        .iter()
        .filter(|i| i.op == CodeOp::BranchLink)
        .filter_map(|i| i.get("target"))
        .filter(|target| target.contains("arena_alloc"))
        .count();
    // Five sends of three scalar types. The program still allocates for its
    // thread handles and its `toString` results, so the assertion is a BOUND
    // rather than zero -- but a per-send block copy would put five more here.
    assert!(
        allocs < 12,
        "`main` made {allocs} arena_alloc call(s) for five scalar sends; a \
         scalar message needs no block, and one allocation per send means the \
         copy is going through `copy_flat_block` and the receiver is reading a \
         pointer as a value"
    );
}
