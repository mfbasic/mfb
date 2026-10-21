//! `nir_fault.rs`'s one-value-at-a-time corruption, in the family it does not have:
//! an OPERATOR that does not fit its operands.
//!
//! `nir_fault` renames what a builder reads and flips a constant between `Integer`
//! and `String`. The numeric builders refuse in another way — an operator the
//! operand type has no lowering for ("native code plan does not lower Fixed operator
//! '&'", "... integer operator 'AND'") — and only an operator swapped in place
//! reaches it, because the type checker never lets one through.
//!
//! Every binary and unary operator node of the probe is swapped, one node at a time,
//! for each of the other operators. As in `nir_fault`, the assertion is not that
//! every corruption is refused (`+` for `-` is a perfectly good program) but that
//! none takes a backend down — a panic in codegen is a build that dies with no
//! located error — and that refusal is common enough to mean the builders check
//! their operator.
//!
//! The nodes are split over [`SHARDS`] tests so the runner lowers them in parallel.

use super::nir_fault::{restore, snapshot, walk_ops_mut, SRC};
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::nir::{NirModule, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};

const BINARY_OPS: [BinaryOp; 17] = [
    BinaryOp::Or,
    BinaryOp::Xor,
    BinaryOp::And,
    BinaryOp::Equal,
    BinaryOp::NotEqual,
    BinaryOp::Less,
    BinaryOp::LessEqual,
    BinaryOp::Greater,
    BinaryOp::GreaterEqual,
    BinaryOp::Concat,
    BinaryOp::Add,
    BinaryOp::Subtract,
    BinaryOp::Multiply,
    BinaryOp::Divide,
    BinaryOp::Mod,
    BinaryOp::IntDiv,
    BinaryOp::Power,
];

const UNARY_OPS: [UnaryOp; 3] = [UnaryOp::Not, UnaryOp::Negate, UnaryOp::SizeOf];

const SHARDS: usize = 8;

/// The operators `nir_fault::SRC` has no operand for: `Fixed`, `Float`, `Byte` and
/// `Money` arithmetic, record equality, boolean and comparison chains over mixed
/// numeric types.
const NUMERIC_SRC: &str = r#"IMPORT io

TYPE Pt
  x AS Integer
  y AS Integer
END TYPE

FUNC main() AS Integer
  LET a AS Pt = Pt[1, 2]
  LET b AS Pt = Pt[1, 3]
  LET same AS Boolean = a = b
  LET fx AS Fixed = 2.50F
  LET fy AS Fixed = 3.25F
  LET fz AS Fixed = fx * fy + fx / fy - fx MOD fy
  LET fl AS Float = 1.5
  LET fm AS Float = fl * 2.0 + fl / 3.0 - fl MOD 2.0 + fl ^ 2.0
  LET by AS Byte = toByte(7)
  LET bz AS Byte = by + by * by - by
  LET mo AS Money = 1.25m
  LET mp AS Money = mo * 3 + mo - mo / 2
  LET s AS String = "a" & "b"
  LET c1 AS Boolean = fx < fy AND fl > 1.0 OR NOT same
  LET c2 AS Boolean = by >= bz XOR mo <= mp
  LET i AS Float = 7 MOD 3 + 7 DIV 2 - 1
  LET ip AS Integer = 2 ^ 3
  io::print(toString(fz) & toString(fm) & toString(bz) & toString(mp) & s & toString(c1) & toString(c2) & toString(i) & toString(ip))
  RETURN 0
END FUNC
"#;

/// Swap the operator of the `index`-th operator node (binary and unary, in tree
/// order) for its `variant`-th alternative; `false` when there is no such node or
/// alternative.
fn swap_nth(module: &mut NirModule, index: usize, variant: usize) -> bool {
    let mut seen = 0usize;
    let mut applied = false;
    for function in &mut module.functions {
        walk_ops_mut(&mut function.body, &mut |_| {}, &mut |value| {
            if applied {
                return;
            }
            match value {
                NirValue::Binary { op, .. } => {
                    if seen == index {
                        let others: Vec<BinaryOp> =
                            BINARY_OPS.iter().copied().filter(|o| o != op).collect();
                        if let Some(next) = others.get(variant) {
                            *op = *next;
                            applied = true;
                        }
                    }
                    seen += 1;
                }
                NirValue::Unary { op, .. } => {
                    if seen == index {
                        let others: Vec<UnaryOp> =
                            UNARY_OPS.iter().copied().filter(|o| o != op).collect();
                        if let Some(next) = others.get(variant) {
                            *op = *next;
                            applied = true;
                        }
                    }
                    seen += 1;
                }
                _ => {}
            }
        });
        if applied {
            return true;
        }
    }
    false
}

/// `(swapped, refused)` for the nodes of shard `shard`; panics collected into
/// `panicked`.
fn sweep(shard: usize, panicked: &mut Vec<String>) -> (usize, usize) {
    let (mut swept, mut refused) = (0usize, 0usize);
    for source in [SRC, NUMERIC_SRC] {
        let (more_swept, more_refused) = sweep_program(source, shard, panicked);
        swept += more_swept;
        refused += more_refused;
    }
    (swept, refused)
}

fn sweep_program(source: &str, shard: usize, panicked: &mut Vec<String>) -> (usize, usize) {
    let target = CodeTarget::LinuxX86_64;
    let mut module = nir_for_src(source, target, Console).expect("the probe lowers");
    let pristine = snapshot(&module);
    let (mut swept, mut refused) = (0usize, 0usize);
    for index in (shard..).step_by(SHARDS) {
        restore(&mut module, &pristine);
        if !swap_nth(&mut module, index, 0) {
            break;
        }
        for variant in 0.. {
            restore(&mut module, &pristine);
            if !swap_nth(&mut module, index, variant) {
                break;
            }
            swept += 1;
            match code_for_nir(&module, target) {
                Ok(_) => {}
                Err(message) if message.starts_with("panicked: ") => {
                    panicked.push(format!("#{index}.{variant}: {message}"));
                }
                Err(_) => refused += 1,
            }
        }
    }
    (swept, refused)
}

fn shard_test(shard: usize) {
    // The code stage recurses over the value tree; a libtest thread's 2 MiB is not
    // what the compiler runs on.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut panicked = Vec::new();
            let (swept, refused) = sweep(shard, &mut panicked);
            assert!(
                panicked.is_empty(),
                "{} swapped-operator module(s) took the backend down instead of being refused:\n  {}",
                panicked.len(),
                panicked.join("\n  ")
            );
            // Each shard owns a share of ~100 operator nodes x 16 alternatives.
            assert!(
                swept > 50,
                "shard {shard} swapped only {swept} operators; a walker that stopped \
                 finding operator nodes would show up here rather than as a green run \
                 over nothing"
            );
            assert!(
                refused * 100 > swept * 10,
                "only {refused} of {swept} swapped-operator modules were refused"
            );
        })
        .expect("spawn the sweep thread")
        .join()
        .unwrap_or_else(|_| panic!("shard {shard} failed"));
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_0() {
    shard_test(0);
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_1() {
    shard_test(1);
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_2() {
    shard_test(2);
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_3() {
    shard_test(3);
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_4() {
    shard_test(4);
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_5() {
    shard_test(5);
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_6() {
    shard_test(6);
}

#[test]
fn swapped_operators_are_refused_not_a_panic_shard_7() {
    shard_test(7);
}

/// The numeric probe is itself a valid program: every backend lowers it, so a
/// refusal in the sweep above is the swapped operator's doing.
#[test]
fn the_numeric_probe_lowers_on_every_backend() {
    for target in CodeTarget::ALL {
        let module = nir_for_src(NUMERIC_SRC, target, Console).expect("the probe lowers to NIR");
        code_for_nir(&module, target).unwrap_or_else(|error| panic!("{}: {error}", target.name()));
    }
}
