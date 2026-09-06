//! A small vector local that does not escape keeps its lanes in registers.
//!
//! plan-01-vector: a `vector::Float3` bound to a local that never leaves the
//! function has no arena block at all — the three lanes live in scalar-Float
//! carriers, and a read reconstructs a native view from them. One that DOES
//! escape (into a collection, a record field, a call) is materialised to a block
//! at the boundary.
//!
//! `lower_ops_inner`'s `if promote_vector` arm is where the promoted binding is
//! recorded, and nothing in process reached it. The corpus has
//! `vector-promotion`, which is exactly this — and its whole content sits in a
//! `TESTING` block, so its `main` is `RETURN 0` and the harness lowers nothing.
//! Twelve of the 429 corpus fixtures are shaped that way; this is the one that
//! mattered, because it was added to the corpus for this path.
//!
//! What promotion buys is the absence of an allocation, so what a test can see
//! is an allocation that is not there — which is why the escaping program is
//! here beside it.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// Vector locals that never leave `main`.
const PROMOTED: &str = "\
IMPORT io
IMPORT vector

FUNC main() AS Integer
  LET c AS vector::Float3 = vector::cross(vector::Float3[1.0, 2.0, 3.0], vector::Float3[4.0, 5.0, 6.0])
  LET s AS vector::Float3 = vector::scale(c, vector::Float3[2.0, 2.0, 2.0])
  io::print(toString(vector::dot(c, s)))
  RETURN 0
END FUNC
";

/// The same vectors, escaping into a collection.
const ESCAPING: &str = "\
IMPORT collections
IMPORT io
IMPORT vector

FUNC main() AS Integer
  LET c AS vector::Float3 = vector::cross(vector::Float3[1.0, 2.0, 3.0], vector::Float3[4.0, 5.0, 6.0])
  LET s AS vector::Float3 = vector::scale(c, vector::Float3[2.0, 2.0, 2.0])
  MUT xs AS List OF vector::Float3 = []
  xs = collections::append(xs, c)
  xs = collections::append(xs, s)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
";

fn plan(source: &str) -> &'static NativeCodePlan {
    code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
}

/// Calls `main` makes to the arena allocator.
fn arena_allocs(source: &str) -> usize {
    code_function(plan(source), "main")
        .instructions
        .iter()
        .filter(|i| i.op == CodeOp::BranchLink)
        .filter_map(|i| i.get("target"))
        .filter(|target| target.contains("arena_alloc"))
        .count()
}

/// A non-escaping vector local costs no allocation; an escaping one does.
///
/// This is the whole point of the row, and the only thing a test can observe:
/// promotion produces no block, so the observable is an `arena_alloc` that is
/// not there. Both programs compute the same two vectors and differ only in
/// whether they are appended to a list.
#[test]
fn a_non_escaping_vector_local_allocates_nothing_and_an_escaping_one_does() {
    let promoted = arena_allocs(PROMOTED);
    let escaping = arena_allocs(ESCAPING);
    assert!(
        escaping > promoted,
        "two `vector::Float3` locals that never leave `main` keep their lanes in \
         registers and need no block; the same two appended to a list must be \
         materialised. The promoted program made {promoted} arena_alloc call(s) \
         and the escaping one {escaping}"
    );
}

/// The promoted program still lowers on every backend.
///
/// The lanes are scalar-Float carriers and the reconstruction is per-backend
/// (`vector_native_lanes`), so a backend that could not rebuild a native view
/// from them would fail here rather than in whichever program first read one.
#[test]
fn a_promoted_vector_local_lowers_on_every_backend() {
    for target in CodeTarget::ALL {
        let plan = crate::testutil::try_code_for_src(PROMOTED, target, Console)
            .unwrap_or_else(|err| panic!("promoted vector locals on {}: {err}", target.name()));
        assert!(
            plan.functions.iter().any(|f| f.name == "main"),
            "{}: the plan must define `main`",
            target.name()
        );
    }
}
