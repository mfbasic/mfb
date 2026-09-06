//! One real module, corrupted one value at a time, lowered by every backend.
//!
//! The gap this is for is the largest single shape left in `src/codegen/**`:
//! the `return Err(...)` a builder makes when the NIR it was handed does not
//! say what the stage before it promised — "native collection set list index
//! must be Integer, got String", "native code assignment unknown local 'x'",
//! "native code record 'Dot' has no field 'q'". Every one of them is
//! unreachable from a program, because the type checker and `ir::shape` run
//! first and a module that reached codegen is well-formed by construction. They
//! are also the messages that decide whether a lowering bug surfaces as a
//! sentence or as a wrong instruction, so deleting them is not the answer.
//!
//! The corpus proves the ACCEPTING half at 630 programs. This proves the other
//! half the only way it can be proved in process: take a module the corpus
//! already lowers, break exactly one value in it, and hand it back to the same
//! five backends.
//!
//! **One value at a time**, for the reason `nir_validation.rs` states about the
//! validator: a module broken in two places is refused by whichever check runs
//! first, and the other rule could be deleted without a row noticing.
//!
//! The assertion is a FLOOR on refusals rather than "every corruption is
//! refused", because that is not a property this compiler has and pretending
//! otherwise would make the row a lie. A `Const`'s declared type is advisory in
//! plenty of positions; a `Local` renamed to something nothing binds is not. The
//! honest statement is that the great majority are caught, and that none of them
//! takes the process down — a panic is not a diagnostic.

use crate::target::shared::nir::{NirMatchPattern, NirModule, NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};
use crate::types::ParameterType;

/// A program broad enough that its values reach most of the builder.
///
/// Every construct here exists to put a different SHAPE of value in the tree,
/// because a corruption only reaches a builder path some value in the tree took:
/// a record field write and a `WITH` rebuild, a list/map/set literal and an
/// in-place mutation of each, a nested list, a union match, an enum comparison,
/// every loop form, a global read and write, a `SUB` call, a lambda, string
/// slicing, `Float`/`Fixed`/`Money` arithmetic, a comparison chain, and a
/// `TRAP`.
const SRC: &str = r#"IMPORT collections
IMPORT io
IMPORT strings

ENUM Color
  Red, Blue
END ENUM

TYPE Dot
  x AS Integer
  y AS Integer
  tag AS String
END TYPE

TYPE Bag
  items AS List OF Integer
  name AS String
END TYPE

TYPE Circle
  radius AS Integer
END TYPE

TYPE Square
  side AS Integer
END TYPE

UNION Shape
  Circle
  Square
END UNION

MUT counter AS Integer = 0

FUNC area(s AS Shape) AS Integer
  MATCH s
    CASE Circle(c)
      RETURN c.radius * c.radius * 3
    CASE Square(q)
      RETURN q.side * q.side
  END MATCH
END FUNC

FUNC label(d AS Dot) AS String
  RETURN d.tag & ":" & toString(d.x + d.y)
END FUNC

SUB bump(by AS Integer)
  counter = counter + by
END SUB

FUNC main() AS Integer
  MUT dot AS Dot = Dot[1, 2, "origin"]
  dot = WITH dot { x := 10 }
  io::print(label(dot))

  MUT xs AS List OF Integer = [1, 2, 3]
  xs = collections::append(xs, 4)
  xs = collections::set(xs, 0, 9)
  xs = collections::prepend(xs, 0)
  xs = collections::removeAt(xs, 1)
  MUT total AS Integer = 0
  FOR i = 0 TO 3
    total = total + collections::get(xs, i)
  NEXT
  FOR EACH v IN xs
    total = total + v
  NEXT
  MUT n AS Integer = 0
  WHILE n < 3
    n = n + 1
  END WHILE
  DO
    n = n - 1
  LOOP UNTIL n <= 0
  io::print("total=" & toString(total) & " n=" & toString(n))

  LET grid AS List OF List OF Integer = [[1, 2], [3]]
  io::print("grid=" & toString(len(collections::flatten(grid))))

  MUT bag AS Bag = Bag[[1, 2], "b"]
  bag = WITH bag { items := collections::append(bag.items, 3) }
  io::print("bag=" & bag.name & toString(len(bag.items)))

  MUT names AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  names = collections::set(names, "b", 2)
  names = collections::removeKey(names, "a")
  MUT seen AS Set OF Integer = Set OF Integer { 1, 2, 3 }
  seen = collections::add(seen, 4)
  seen = collections::remove(seen, 1)
  io::print("sizes=" & toString(len(names)) & "," & toString(len(seen)))

  LET shape AS Shape = Circle[4]
  io::print("area=" & toString(area(shape)))

  LET c AS Color = Color.Blue
  IF c = Color.Blue THEN
    io::print("blue")
  ELSE
    io::print("red")
  END IF

  bump(5)
  io::print("counter=" & toString(counter))

  LET f AS Float = 1.5 * 2.0 + 0.25
  LET x AS Fixed = 2.50F * 3.0F
  LET m AS Money = 19.99m + 0.01m
  io::print("nums=" & toString(f) & "," & toString(x) & "," & toString(m))

  LET word AS String = strings::mid("abcdef", 1, 3)
  io::print("word=" & word & " upper=" & strings::upper(word))

  LET doubled AS List OF Integer = collections::transform(xs, LAMBDA(v AS Integer) -> v * 2)
  io::print("doubled=" & toString(len(doubled)))

  LET safe AS Integer = collections::get(xs, 99) TRAP(e)
    RECOVER 0
  END TRAP
  io::print("safe=" & toString(safe))

  RETURN 0
END FUNC
"#;

/// Corrupt `value` if it is a shape this knows how to corrupt.
///
/// Deliberately narrow. A corruption has to be one the stage before codegen
/// would have caught — a type that disagrees, a name nothing binds — rather than
/// a structural impossibility, because a structurally impossible module tests
/// `validate_nir` (which `nir_validation.rs` already covers) instead of the
/// builder.
fn corrupt(value: &mut NirValue) -> bool {
    match value {
        NirValue::Const { type_, .. } => {
            *type_ = if *type_ == ParameterType::Integer {
                ParameterType::String
            } else {
                ParameterType::Integer
            };
            true
        }
        NirValue::Local(name) => {
            *name = "__no_such_local__".to_string();
            true
        }
        NirValue::Global { name, .. } => {
            *name = "__no_such_global__".to_string();
            true
        }
        NirValue::MemberAccess { member, .. } => {
            *member = "__no_such_field__".to_string();
            true
        }
        NirValue::Call { target, .. } => {
            *target = "__no_such_function__".to_string();
            true
        }
        _ => false,
    }
}

/// Whether [`corrupt`] would do anything to this value.
fn corruptible(value: &NirValue) -> bool {
    matches!(
        value,
        NirValue::Const { .. }
            | NirValue::Local(_)
            | NirValue::Global { .. }
            | NirValue::MemberAccess { .. }
            | NirValue::Call { .. }
    )
}

/// Apply `f` to every value in `ops`, depth first, parents before children.
///
/// The mutable twin of `nir::visit::walk_ops`, which takes `&NirOp` and so
/// cannot be used to change anything. It lives here rather than beside its
/// immutable sibling because nothing in the product needs it — and, like that
/// one, it is exhaustive with no `_` arm, so a new `NirOp` or `NirValue` variant
/// is a compile error rather than a silent hole in this sweep.
fn walk_ops_mut(ops: &mut [NirOp], f: &mut dyn FnMut(&mut NirValue)) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    walk_value_mut(value, f);
                }
            }
            NirOp::Assign { value, .. } => walk_value_mut(value, f),
            NirOp::StateAssign { value, .. } => walk_value_mut(value, f),
            NirOp::Return { value } => {
                if let Some(value) = value {
                    walk_value_mut(value, f);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => walk_value_mut(code, f),
            NirOp::Fail { error } => walk_value_mut(error, f),
            NirOp::Eval { value } => walk_value_mut(value, f),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                walk_value_mut(condition, f);
                walk_ops_mut(then_body, f);
                walk_ops_mut(else_body, f);
            }
            NirOp::Match { value, cases } => {
                walk_value_mut(value, f);
                for case in cases {
                    match &mut case.pattern {
                        NirMatchPattern::Else => {}
                        NirMatchPattern::Value(value) => walk_value_mut(value, f),
                        NirMatchPattern::OneOf(values) => {
                            for value in values {
                                walk_value_mut(value, f);
                            }
                        }
                    }
                    if let Some(guard) = &mut case.guard {
                        walk_value_mut(guard, f);
                    }
                    walk_ops_mut(&mut case.body, f);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                walk_value_mut(condition, f);
                walk_ops_mut(body, f);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                walk_value_mut(start, f);
                walk_value_mut(end, f);
                walk_value_mut(step, f);
                walk_ops_mut(body, f);
            }
            NirOp::DoUntil { body, condition } => {
                walk_ops_mut(body, f);
                walk_value_mut(condition, f);
            }
            NirOp::ForEach { iterable, body, .. } => {
                walk_value_mut(iterable, f);
                walk_ops_mut(body, f);
            }
            NirOp::Trap { body, .. } => walk_ops_mut(body, f),
        }
    }
}

fn walk_value_mut(value: &mut NirValue, f: &mut dyn FnMut(&mut NirValue)) {
    f(value);
    match value {
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Closure { captures, .. } => {
            for capture in captures {
                walk_value_mut(capture, f);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                walk_value_mut(arg, f);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. } => walk_value_mut(value, f),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            walk_value_mut(target, f);
            for update in updates {
                walk_value_mut(&mut update.value, f);
            }
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            for value in values {
                walk_value_mut(value, f);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_value_mut(key, f);
                walk_value_mut(value, f);
            }
        }
        NirValue::MemberAccess { target, .. } => walk_value_mut(target, f),
        NirValue::Binary { left, right, .. } => {
            walk_value_mut(left, f);
            walk_value_mut(right, f);
        }
        NirValue::Unary { operand, .. } => walk_value_mut(operand, f),
    }
}

/// The pristine bodies of every function, so a mutated module can be put back.
///
/// `NirFunction` is not `Clone` — and deriving it on a production type so a test
/// could copy one would be the test changing the product to suit itself — but
/// `NirOp` is, because the Opt1 loop rows duplicate statement lists. The bodies
/// alone are enough: nothing here mutates a signature.
fn snapshot(module: &NirModule) -> Vec<Vec<NirOp>> {
    module.functions.iter().map(|f| f.body.clone()).collect()
}

fn restore(module: &mut NirModule, pristine: &[Vec<NirOp>]) {
    for (function, body) in module.functions.iter_mut().zip(pristine) {
        function.body = body.clone();
    }
}

/// Corrupt the `index`-th corruptible value; `false` past the end.
fn corrupt_nth(module: &mut NirModule, index: usize) -> bool {
    let mut seen = 0usize;
    let mut applied = false;
    for function in &mut module.functions {
        walk_ops_mut(&mut function.body, &mut |value| {
            // Count only what `corrupt` acts on, so the index space has no holes
            // and the sweep's own bound means what it says.
            if applied || !corruptible(value) {
                return;
            }
            if seen == index {
                applied = corrupt(value);
            }
            seen += 1;
        });
        if applied {
            return true;
        }
    }
    false
}

/// Every backend refuses a module whose values have been corrupted, one at a
/// time, and none of them dies doing it.
#[test]
fn a_corrupted_value_is_refused_rather_than_lowered() {
    // 64 MiB, matching every other lowering entry in `testutil`: the code stage
    // recurses over the value tree, and a libtest thread's 2 MiB is not what the
    // compiler runs on.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(sweep)
        .expect("spawn the sweep thread")
        .join()
        .expect("the sweep must not take the process down");
}

fn sweep() {
    let mut swept = 0usize;
    let mut refused = 0usize;
    let mut panicked = Vec::new();

    for target in CodeTarget::ALL {
        let mut module = nir_for_src(SRC, target, Console).expect("the probe program must lower");
        let pristine = snapshot(&module);

        // The unmutated module must lower, or every refusal below is a refusal
        // of something else and the sweep measures nothing.
        code_for_nir(&module, target).unwrap_or_else(|err| {
            panic!("the probe program must lower on {}: {err}", target.name())
        });

        for index in 0.. {
            restore(&mut module, &pristine);
            if !corrupt_nth(&mut module, index) {
                break;
            }
            swept += 1;
            match code_for_nir(&module, target) {
                Ok(_) => {}
                Err(message) if message.starts_with("panicked: ") => {
                    panicked.push(format!("{} #{index}: {message}", target.name()));
                }
                Err(_) => refused += 1,
            }
        }
    }

    eprintln!(
        "PROBE swept={swept} refused={refused} panicked={}",
        panicked.len()
    );
    assert!(
        panicked.is_empty(),
        "{} corrupted module(s) took a backend down instead of being refused. A \
         panic is not a diagnostic: the build dies with no located error and the \
         author is told nothing.\n  {}",
        panicked.len(),
        panicked.join("\n  ")
    );

    // Two bounds, and both are load-bearing. The first says the sweep RAN: a
    // walker that stopped finding values would otherwise pass by corrupting
    // nothing, which is exactly how the platform sweep first fooled me
    // (planning/tests.md, C11). The second says refusal is the rule rather than
    // the exception, and is a ratio rather than "all of them" because a `Const`
    // type is advisory in some positions and claiming otherwise would be false.
    assert!(
        swept > 900,
        "the sweep corrupted only {swept} values across five backends; the probe \
         program measured 1,000 (200 corruptible values x 5 backends), and a \
         walker that stopped descending would show up here rather than as a \
         green run over nothing"
    );
    assert!(
        refused * 4 > swept * 3,
        "only {refused} of {swept} corrupted modules were refused; it measured \
         785, and a builder that stopped checking its inputs shows up here as \
         this ratio falling"
    );
}
