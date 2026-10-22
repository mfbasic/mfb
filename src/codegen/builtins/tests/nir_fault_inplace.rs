//! The in-place self-update arms (`collection/assign/**` — append/prepend/insert/
//! removeAt/set/mid/repeat, replace, sort, distinct, filter, transform, take/drop, the
//! `math` array members, the set algebra and add/remove, `merge`, `mapValues`,
//! removeKey and map `set`, the `String` self-updates, and the record-field forms of
//! several), handed a self-update statement whose NIR disagrees with what the front
//! end promised, one value at a time: a constant's or a function value's declared
//! type, a name, a call's last argument, or the member itself swapped for one over
//! another collection kind (which each arm's G9 gate must decline).
//!
//! `nir_fault.rs` sweeps every value of two broad programs; neither carries most of
//! these statements, and a corruption only reaches a builder path some value in the
//! tree took. This sweep corrupts ONLY the values inside the self-update statements
//! of one program — every `x = f(x, ...)` and every `WITH r { f := g(r.f, ...) }` —
//! so it stays a few seconds long while reaching each arm's own argument checks:
//! "native in-place replace old must be Integer, got String", a sort key that is not
//! a function, an argument count the member does not take.
//!
//! The contract is `nir_fault.rs`'s: every corrupted module is refused or lowered,
//! and none takes the process down — a panic is not a diagnostic — and refusal is the
//! rule (a `Const`'s declared type is advisory in some positions, so "every one" would
//! be false).

use super::nir_fault::{restore, snapshot, walk_ops_mut};
use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_nir, nir_for_src, CodeTarget};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT collections
IMPORT io
IMPORT math
IMPORT strings

TYPE Bag
  xs AS List OF Integer
  names AS List OF String
  st AS Set OF Integer
  mp AS Map OF String TO Integer
END TYPE

FUNC keep(n AS Integer) AS Boolean
  RETURN n > 1
END FUNC

FUNC twice(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC bump(n AS Integer) AS Integer
  RETURN n + 1
END FUNC

FUNC main() AS Integer
  MUT xs AS List OF Integer = [3, 1, 2, 3]
  xs = collections::replace(xs, 3, 7)
  xs = collections::sort(xs)
  xs = collections::distinct(xs)
  xs = collections::filter(xs, keep)
  xs = collections::transform(xs, twice)
  xs = collections::take(xs, 3)
  xs = collections::drop(xs, 1)
  xs = collections::append(xs, 9)
  xs = collections::prepend(xs, 0)
  xs = collections::insert(xs, 1, 5)
  xs = collections::removeAt(xs, 0)
  xs = collections::append(xs, [7, 8])
  xs = collections::mid(xs, 0, 3)
  xs = collections::repeat(xs, 4, 2, 1)
  xs = collections::set(xs, 0, 6)
  MUT names AS List OF String = ["b", "a", "b"]
  names = collections::replace(names, "b", "zz")
  names = collections::sort(names)
  names = collections::distinct(names)
  MUT fs AS List OF Float = [1.0, -2.0, 4.0]
  fs = math::abs(fs)
  fs = math::sqrt(fs)
  MUT st AS Set OF Integer = collections::toSet([1, 2, 3])
  st = collections::union(st, collections::toSet([4]))
  st = collections::intersection(st, collections::toSet([1, 4]))
  st = collections::difference(st, collections::toSet([1]))
  st = collections::symmetricDifference(st, collections::toSet([9]))
  st = collections::add(st, 5)
  st = collections::remove(st, 5)
  MUT mp AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  mp = collections::merge(mp, Map OF String TO Integer { "b" := 2 }, TRUE)
  mp = collections::mapValues(mp, bump)
  mp = collections::removeKey(mp, "a")
  mp = collections::set(mp, "z", 3)
  MUT s AS String = "hello"
  s = strings::replace(s, "l", "L")
  s = strings::upper(s)
  s = strings::mid(s, 1, 3)
  MUT bag AS Bag = Bag[[5, 4, 5], ["q", "p"], collections::toSet([1]), Map OF String TO Integer { "k" := 1 }]
  bag = WITH bag { xs := collections::replace(bag.xs, 5, 6) }
  bag = WITH bag { xs := collections::sort(bag.xs) }
  bag = WITH bag { names := collections::sort(bag.names) }
  bag = WITH bag { st := collections::union(bag.st, collections::toSet([2])) }
  bag = WITH bag { mp := collections::merge(bag.mp, Map OF String TO Integer { "j" := 2 }, FALSE) }
  io::print(toString(len(xs)) & toString(len(names)) & toString(len(fs)) & toString(len(st)) & toString(len(mp)) & s & toString(len(bag.xs)))
  RETURN 0
END FUNC
"#;

/// The ways one value is broken: its declared type, the name it reads, the field it
/// reads, and (for a call) its last argument.
#[derive(Clone, Copy, Debug)]
enum Fault {
    Type,
    Name,
    Arity,
    /// A `collections` member swapped for one of the same arity over ANOTHER
    /// collection kind (`append` -> `add`): each arm's G9 gate — "`add` mutates a
    /// Set" — then sees a binding of the wrong kind.
    Retarget,
}

/// The member a [`Fault::Retarget`] swaps `member` for.
fn other_kind_member(member: &str) -> Option<&'static str> {
    Some(match member {
        "append" | "prepend" => "add",
        "add" => "append",
        "removeAt" | "removeKey" => "remove",
        "remove" => "removeAt",
        "insert" => "mid",
        "mid" => "insert",
        _ => return None,
    })
}

/// `target` with its `collections` member swapped (both post-lowering spellings,
/// `collections.X` and the monomorph `#collections_X$T…`).
fn retarget(target: &str) -> Option<String> {
    let (prefix, rest) = if let Some(rest) = target.strip_prefix("#collections_") {
        ("#collections_", rest)
    } else {
        ("collections.", target.strip_prefix("collections.")?)
    };
    let (member, suffix) = rest.split_at(rest.find('$').unwrap_or(rest.len()));
    Some(format!("{prefix}{}{suffix}", other_kind_member(member)?))
}

/// Apply `fault` to `value` if it applies; whether it did.
fn break_value(value: &mut NirValue, fault: Fault) -> bool {
    match (fault, value) {
        (Fault::Type, NirValue::Const { type_, .. }) => {
            *type_ = if *type_ == ParameterType::Integer {
                ParameterType::String
            } else {
                ParameterType::Integer
            };
            true
        }
        // A function value's declared return type: the predicate, key or mapping
        // function an arm calls per element.
        (Fault::Type, NirValue::FunctionRef { type_, .. }) => match type_ {
            ParameterType::Func(_, returns, _) => {
                **returns = if **returns == ParameterType::Integer {
                    ParameterType::String
                } else {
                    ParameterType::Integer
                };
                true
            }
            _ => false,
        },
        (Fault::Name, NirValue::Local(name)) => {
            *name = "__no_such_local__".to_string();
            true
        }
        (Fault::Name, NirValue::MemberAccess { member, .. }) => {
            *member = "__no_such_field__".to_string();
            true
        }
        (Fault::Name, NirValue::FunctionRef { name, .. }) => {
            *name = "__no_such_function__".to_string();
            true
        }
        (
            Fault::Arity,
            NirValue::Call { args, .. }
            | NirValue::CallResult { args, .. }
            | NirValue::RuntimeCall { args, .. },
        ) if !args.is_empty() => {
            args.pop();
            true
        }
        (Fault::Retarget, NirValue::Call { target, .. }) => match retarget(target) {
            Some(swapped) => {
                *target = swapped;
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// Whether `op` is one of the self-update statements: an assignment whose value is a
/// call or a `WITH` rebuild of a record local.
fn is_self_update(op: &NirOp) -> bool {
    match op {
        NirOp::Assign { value, .. } => matches!(
            value,
            NirValue::Call { .. }
                | NirValue::RuntimeCall { .. }
                | NirValue::CallResult { .. }
                | NirValue::WithUpdate { .. }
        ),
        _ => false,
    }
}

/// What a value is, for a report: a call's target, else the variant.
fn describe(value: &NirValue) -> String {
    match value {
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => format!("call `{target}`"),
        NirValue::Const { value, .. } => format!("constant `{value}`"),
        NirValue::Local(name) => format!("local `{name}`"),
        NirValue::MemberAccess { member, .. } => format!("field `{member}`"),
        NirValue::FunctionRef { name, .. } => format!("function `{name}`"),
        _ => "a value".to_string(),
    }
}

/// Break the `index`-th breakable value inside a self-update statement of `main`;
/// what was broken, or `None` past the end.
fn break_nth(module: &mut NirModule, fault: Fault, index: usize) -> Option<String> {
    let mut seen = 0usize;
    let mut applied = None;
    for function in module.functions.iter_mut().filter(|f| f.name == "main") {
        // The walk visits an op, then its values; the op callback opens and closes
        // the window in which values count.
        let inside = std::cell::Cell::new(false);
        walk_ops_mut(
            &mut function.body,
            &mut |op| inside.set(is_self_update(op)),
            &mut |value| {
                if applied.is_some() || !inside.get() {
                    return;
                }
                let mut probe = value.clone();
                if !break_value(&mut probe, fault) {
                    return;
                }
                if seen == index {
                    applied = Some(describe(value));
                    *value = probe;
                }
                seen += 1;
            },
        );
    }
    applied
}

#[test]
fn a_corrupted_self_update_is_refused_rather_than_lowered() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(sweep)
        .expect("spawn the sweep thread")
        .join()
        .expect("the sweep must not take the process down");
}

fn sweep() {
    let target = CodeTarget::LinuxX86_64;
    let mut module = nir_for_src(SRC, target, Console).expect("the probe program lowers");
    let pristine = snapshot(&module);
    code_for_nir(&module, target).expect("the unmutated probe lowers");
    let mut swept = 0usize;
    let mut refused = 0usize;
    let mut panicked = Vec::new();
    for fault in [Fault::Type, Fault::Name, Fault::Arity, Fault::Retarget] {
        for index in 0.. {
            restore(&mut module, &pristine);
            let Some(broken) = break_nth(&mut module, fault, index) else {
                break;
            };
            swept += 1;
            match code_for_nir(&module, target) {
                Ok(_) => {}
                Err(message) if message.starts_with("panicked: ") => {
                    panicked.push(format!("{fault:?} of {broken}: {message}"));
                }
                Err(_) => refused += 1,
            }
        }
    }
    assert!(
        panicked.is_empty(),
        "{} corrupted self-update(s) took the backend down instead of being refused:\n  {}",
        panicked.len(),
        panicked.join("\n  ")
    );
    eprintln!("nir_fault_inplace: {refused} of {swept} corrupted self-updates refused");
    // Measured: 149 corrupted, 111 refused. The rest lowered: a `Const`'s declared type
    // is advisory where the arm re-derives it, and a dropped trailing argument of a
    // member with an optional parameter is still a valid call.
    assert!(swept >= 140, "the sweep corrupted only {swept} values");
    assert!(
        refused * 100 >= swept * 70,
        "only {refused} of {swept} corrupted self-updates were refused"
    );
}
