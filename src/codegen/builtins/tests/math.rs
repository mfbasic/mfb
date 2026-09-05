//! Codegen contracts for the in-tree `fmod` kernel (`gen_fmod.rs`).
//!
//! What the kernel *computes* is already pinned by the acceptance suite
//! (`tests/acceptance/src/arithmetic.mfb:203`, `expectFloat(fa MOD fn, 1.5)`) —
//! that runs a real executable and is the right oracle for a numeric answer.
//! What no end-to-end run can see is asserted here, in process:
//!
//!   * the kernel is **standalone**. The reason musl's `fmod` was ported in-tree
//!     is that a `Float MOD Float` program links no math library and stays a
//!     static ELF (`docs/spec/architecture/18_math-kernels.md`). A regression to
//!     a libm call still computes the right answer, so every behavioural fixture
//!     keeps passing while the guarantee is gone.
//!   * the kernel omits libm's exception prologue **because** a zero divisor is
//!     rejected before it runs. Delete that guard and the reduction loop is
//!     entered with `ey == 0`, which the ported code is documented not to handle.
//!   * two `MOD`s in one function do not collide — which needs a program with
//!     two of them, and is invisible to a fixture that has one.
//!
//! Deliberately NOT asserted: that every branch lands on a defined label.
//! `code::lower_module` already rejects that itself ("native code function
//! 'remainders' branches to label 'fmod_x_subdon', which it does not define"),
//! so a test for it would only restate a product check.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::Stream;
use crate::codegen::engine::types::{CodeFunction, NativeCodePlan};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

/// Two `Float MOD Float`s in one function, so the kernel is emitted twice in a
/// single body.
const FMOD_SRC: &str = "\
FUNC remainders(a AS Float, b AS Float, c AS Float) AS Float
  LET first AS Float = a MOD b
  LET second AS Float = c MOD b
  RETURN first + second
END FUNC

FUNC main() AS Integer
  LET r AS Float = remainders(7.5, 3.0, 11.25)
  IF r > 0.0 THEN
    RETURN 0
  END IF
  RETURN 1
END FUNC
";

/// The label `builder_numeric` emits for "the divisor is not zero", immediately
/// above each kernel.
const DIVISOR_GUARD: &str = "float_mod_divisor_nonzero_";
/// The kernel's own exit label.
const KERNEL_END: &str = "fmod_end_";

fn program() -> &'static NativeCodePlan {
    code_for_src_cached(FMOD_SRC, CodeTarget::LinuxX86_64, Console)
}

fn remainders() -> &'static CodeFunction {
    code_function(program(), "remainders")
}

/// `(start, end)` for each emitted kernel: its divisor guard and its exit.
///
/// Paired positionally rather than by the labels' trailing counters — those are
/// `CodeBuilder::label`'s per-function sequence, and the guard's number and the
/// kernel's are not related.
fn kernels(f: &CodeFunction) -> Vec<(usize, usize)> {
    let stream = Stream::of(f);
    let mut out = Vec::new();
    for (at, name) in stream.labels() {
        if !name.starts_with(DIVISOR_GUARD) {
            continue;
        }
        let end = stream.index_after(at + 1, &format!("`{KERNEL_END}*` exit for `{name}`"), |i| {
            i.op == CodeOp::Label && i.get("name").is_some_and(|n| n.starts_with(KERNEL_END))
        });
        out.push((at, end));
    }
    assert_eq!(
        out.len(),
        2,
        "the fixture has two `Float MOD Float`s, so two kernels must be emitted"
    );
    out
}

/// A `Float MOD Float` program imports no math library, and the kernel calls out
/// to nothing at all.
#[test]
fn a_float_mod_links_no_math_library() {
    let named: Vec<&str> = program()
        .imports
        .iter()
        .map(|import| import.symbol.as_str())
        .collect();
    for banned in ["fmod", "fmodf", "remainder", "remainderf", "drem"] {
        assert!(
            !named.contains(&banned),
            "a `Float MOD Float` program must import no math library; \
             found `{banned}` among {named:?}"
        );
    }
    let f = remainders();
    for (start, end) in kernels(f) {
        let calls = Stream::of(f).calls_between(start, end);
        assert!(
            calls.is_empty(),
            "the fmod kernel is computed entirely over GPRs and must call nothing; \
             the body at [{start}..{end}) calls {calls:?}"
        );
    }
}

/// Every kernel sits behind the zero-divisor rejection.
///
/// The port deliberately drops libm's exception prologue on the stated ground
/// that "the divisor is guaranteed finite and non-zero — `Float MOD Float`
/// raises `ErrFloatDomain` for `b == 0` before calling". If a lowering change
/// ever reaches the kernel without that guard, the mantissa normalization spins
/// on `uy == 0` instead of raising, and no fixture that passes a non-zero
/// divisor would notice.
#[test]
fn every_fmod_kernel_sits_behind_the_zero_divisor_rejection() {
    let f = remainders();
    let stream = Stream::of(f);
    let guards = stream
        .labels()
        .into_iter()
        .filter(|(_, n)| n.starts_with(DIVISOR_GUARD))
        .count();
    let ends = stream
        .labels()
        .into_iter()
        .filter(|(_, n)| n.starts_with(KERNEL_END))
        .count();
    assert_eq!(
        guards, ends,
        "each emitted fmod kernel ({ends}) must be preceded by its own \
         `{DIVISOR_GUARD}*` guard ({guards})"
    );
    // The guard is a real branch, not just a label: something must jump over the
    // kernel when the divisor IS zero.
    for (_, name) in stream.labels() {
        if !name.starts_with(DIVISOR_GUARD) {
            continue;
        }
        assert!(
            stream.branches_to(&name),
            "`{name}` is never branched to — the zero-divisor check does not \
             actually skip the kernel"
        );
    }
}

/// Two `MOD`s in one function emit two disjoint sets of labels.
///
/// `CodeBuilder::label` uniquifies with a per-function counter and the kernel
/// defines twenty labels. Emitting them with fixed names would define each twice
/// in one body — an assembler failure that only a program with two
/// `Float MOD Float`s in the *same* function produces.
#[test]
fn two_float_mods_in_one_function_do_not_collide() {
    let f = remainders();
    let names: Vec<String> = Stream::of(f)
        .labels()
        .into_iter()
        .map(|(_, n)| n)
        .filter(|n| n.starts_with("fmod_"))
        .collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        names.len(),
        unique.len(),
        "`remainders` defines a duplicate fmod label; a program with two \
         `Float MOD Float`s in one function would not assemble. Labels: {names:?}"
    );
    // Exactly two of each — matched as `<prefix><digits>`, since `fmod_modloop_`
    // is also a prefix of `fmod_modloop_end_`.
    for prefix in [
        "fmod_end_",
        "fmod_modloop_",
        "fmod_modloop_end_",
        "fmod_ret_zero_",
        "fmod_scale_sub_",
        "fmod_normloop_",
    ] {
        let n = names
            .iter()
            .filter(|l| {
                l.strip_prefix(prefix).is_some_and(|tail| {
                    !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())
                })
            })
            .count();
        assert_eq!(
            n, 2,
            "two `Float MOD Float`s must emit two `{prefix}<n>` labels, not {n}"
        );
    }
}
