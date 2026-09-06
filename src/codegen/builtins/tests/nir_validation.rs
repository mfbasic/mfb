//! `validate_nir` refuses each malformed module it is written to catch.
//!
//! It is the gate one stage above `NativeCodePlan::validate`: every backend runs
//! it on the NIR before laying out a plan, so it is where a lowering bug is
//! supposed to be caught while the module is still readable. And, exactly like
//! the code-plan validator, every one of its refusals was unreached — a module
//! from a correct lowering is well-formed by construction, so the corpus walks
//! the whole validator 424 fixtures x 5 backends and takes none of its `Err`
//! arms.
//!
//! `target/shared/validate/names.rs` was at 59.81%, and the shape of the file is
//! the shape of the gap: it is almost entirely `return Err`.
//!
//! One real module, mutated one field at a time. A mutation that broke two
//! invariants at once would be caught by whichever check ran first, and the rule
//! under test could be deleted without the row noticing.

use crate::target::shared::nir::{NirModule, NirOp};
use crate::target::shared::validate::validate_nir;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};

/// A program with two globals, a record, a union, an enum and two functions —
/// and, inside `main`, two binds, an assignment and a store to a global.
///
/// Two of each of the things that must be unique, so a collision can be made by
/// re-pointing the second at the first rather than by duplicating a row —
/// `NirGlobal` and `NirFunction` are not `Clone`, and deriving it on a
/// production type so a test can duplicate a row would be the test changing the
/// product to suit itself.
const SRC: &str = "\
IMPORT io

ENUM Color
  Red, Blue
END ENUM

TYPE Dot
  x AS Integer
END TYPE

TYPE Tag
  name AS String
END TYPE

UNION Shape
  Dot
  Tag
END UNION

MUT counter AS Integer = 7
LET label AS String = \"tag\"

FUNC describe(s AS Shape) AS String
  MATCH s
    CASE Dot(d)
      RETURN toString(d.x)
    CASE Tag(t)
      RETURN t.name
  END MATCH
END FUNC

FUNC main() AS Integer
  LET describer AS FUNC(Shape) AS String = describe
  LET d AS Shape = Dot[counter]
  MUT n AS Integer = 0
  n = n + 1
  ' An IF and a MATCH in main, so a mutation can be placed inside a nested
  ' body -- which is what reaches the `?` that carries a failure back out of
  ' one, rather than the arm that opens it.
  IF n > 0 THEN
    counter = counter + n
  END IF
  MATCH n
    CASE 1
      n = n + 0
    CASE ELSE
      n = n + 0
  END MATCH
  io::print(label & describer(d) & toString(n))
  RETURN 0
END FUNC
";

/// A freshly lowered, valid module. One per mutation: sharing one would let an
/// earlier row's mutation leak into a later one's premise.
fn module() -> NirModule {
    nir_for_src(SRC, CodeTarget::LinuxX86_64, Console).expect("the program must lower to NIR")
}

/// The module a correct lowering produces passes.
///
/// Every row below asserts a mutated module is REFUSED, and all of them would
/// pass if `validate_nir` refused everything. This is the row that says it does
/// not.
#[test]
fn a_module_from_a_real_lowering_validates() {
    validate_nir(&module()).expect("a module from a correct lowering must validate");
}

/// Apply `mutate` to a fresh module and return the refusal it must produce.
fn refusal(what: &str, mutate: impl FnOnce(&mut NirModule)) -> String {
    let mut module = module();
    mutate(&mut module);
    match validate_nir(&module) {
        Err(message) => message,
        Ok(()) => panic!(
            "validate_nir accepted a module with {what}. Every backend runs this \
             before laying out a plan, so accepting it moves the failure to the \
             plan validator at best and to the encoder at worst"
        ),
    }
}

/// The module header fields are required.
#[test]
fn the_module_header_fields_are_required() {
    for (what, message, mutate) in [
        (
            "an empty target",
            "NIR target must not be empty",
            Box::new(|m: &mut NirModule| m.target.clear()) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "an empty project name",
            "NIR project name must not be empty",
            Box::new(|m: &mut NirModule| m.project.clear()),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "a module with {what} must be refused with {message:?}; it said {refused:?}"
        );
    }
}

/// A global must be named, visible in a known way, and declared once.
///
/// The duplicate rules are the ones with teeth. Two globals sharing a NAME make
/// the later read resolve to whichever the map kept; two sharing a SYMBOL make
/// the linker fold two distinct variables into one piece of storage, which is a
/// program where writing `a` changes `b`.
#[test]
fn a_malformed_global_is_refused() {
    for (what, message, mutate) in [
        (
            "a global with no name",
            "name, symbol, and type must not be empty",
            Box::new(|m: &mut NirModule| m.globals[0].name.clear()) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "a global with no symbol",
            "name, symbol, and type must not be empty",
            Box::new(|m: &mut NirModule| m.globals[0].symbol.clear()),
        ),
        (
            "a global with a visibility that is not one of the three",
            "has invalid visibility",
            Box::new(|m: &mut NirModule| m.globals[0].visibility = "weak".to_string()),
        ),
        // Two globals collide by re-pointing the SECOND at the first's name or
        // symbol. `NirGlobal` is not `Clone`, and deriving it on a production
        // type so a test can duplicate a row would be the test changing the
        // product to suit itself.
        (
            "two globals with the same name",
            "is declared more than once",
            Box::new(|m: &mut NirModule| {
                let first = m.globals[0].name.clone();
                m.globals[1].name = first;
            }),
        ),
        (
            "two globals with the same symbol",
            "symbol '",
            Box::new(|m: &mut NirModule| {
                let first = m.globals[0].symbol.clone();
                m.globals[1].symbol = first;
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with a message containing {message:?}; it \
             said {refused:?}"
        );
    }
}

/// A declared type must be named, and so must its members and variants.
///
/// An unnamed enum member or union variant is not cosmetic: the tag a `MATCH`
/// compares against is assigned by variant NAME
/// (`recompute_canonical_variant_tags`), so an empty one collides with every
/// other empty one and dispatch goes to whichever arm sorted first.
#[test]
fn a_malformed_type_is_refused() {
    let refused = refusal("a type with no name", |m| {
        m.types[0].name.clear();
    });
    assert!(
        refused.contains("NIR type has empty name"),
        "a type with no name must be refused; it said {refused:?}"
    );

    let refused = refusal("an enum member with no name", |m| {
        let enum_type = m
            .types
            .iter_mut()
            .find(|type_| !type_.members.is_empty())
            .expect("the program declares `ENUM Color`");
        enum_type.members[0].name.clear();
    });
    assert!(
        refused.contains("has empty member name"),
        "an unnamed enum member must be refused; it said {refused:?}"
    );

    let refused = refusal("a union variant with no name", |m| {
        let union_type = m
            .types
            .iter_mut()
            .find(|type_| !type_.variants.is_empty())
            .expect("the program declares `UNION Shape`");
        union_type.variants[0].name.clear();
    });
    assert!(
        refused.contains("has empty variant name"),
        "an unnamed union variant must be refused; it said {refused:?}"
    );
}

/// A function must be named and declared once.
#[test]
fn a_malformed_function_is_refused() {
    for (what, message, mutate) in [
        (
            "a function with no name",
            "NIR function name must not be empty",
            Box::new(|m: &mut NirModule| m.functions[0].name.clear()) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "two functions with the same name",
            "declared more than once",
            Box::new(|m: &mut NirModule| {
                let first = m.functions[0].name.clone();
                m.functions[1].name = first;
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with {message:?}; it said {refused:?}"
        );
    }
}

/// An entry naming no function is refused.
///
/// The module's entry is what the program's `_main` calls. Naming a function
/// that is not there produces a relocation nothing defines, which the plan
/// validator would catch one stage later and the linker one stage after that —
/// each further from the lowering that did it.
#[test]
fn an_entry_that_names_no_function_is_refused() {
    let has_entry = module().entry.is_some();
    assert!(
        has_entry,
        "the harness lowers `main` as the entry, so this program has one to break"
    );
    let refused = refusal("an entry naming no function", |m| {
        if let Some(entry) = &mut m.entry {
            entry.name = "_no_such_entry".to_string();
        }
    });
    assert!(
        refused.contains("_no_such_entry"),
        "the refusal must name the entry that did not resolve; it said {refused:?}"
    );
}

/// The first `Bind` in `main`'s body, which every mutation below edits.
///
/// By position rather than by name: the lowering renames and introduces
/// temporaries, and a test that hunted for `"d"` would go quiet the day one of
/// them was renamed rather than failing.
fn first_bind(module: &mut NirModule) -> &mut crate::target::shared::nir::NirOp {
    let main = module
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("the program declares `main`");
    main.body
        .iter_mut()
        .find(|op| matches!(op, crate::target::shared::nir::NirOp::Bind { .. }))
        .expect("`main` binds at least one local")
}

/// A body's own rules: a bind must be named and typed, a local declared once,
/// an assignment must target something mutable, and a global store must name a
/// global that exists.
///
/// These are `validate_body`'s refusals, one stage inside the name tables the
/// rows above check. Each is a real miscompile if it gets through: a duplicate
/// local makes the second bind silently shadow the first's slot, and an
/// assignment to an immutable local writes storage the optimizer is entitled to
/// have folded away.
#[test]
fn a_malformed_body_op_is_refused() {
    use crate::target::shared::nir::NirOp;

    let refused = refusal("a bind with no name", |m| {
        if let NirOp::Bind { name, .. } = first_bind(m) {
            name.clear();
        }
    });
    assert!(
        refused.contains("empty name or type"),
        "a bind with no name must be refused; it said {refused:?}"
    );

    let refused = refusal("two locals with the same name", |m| {
        // Re-point the SECOND bind at the first's name. Both are still
        // well-typed, so this breaks exactly the uniqueness rule and nothing
        // else.
        let main = m
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .expect("the program declares `main`");
        let mut names = main.body.iter().filter_map(|op| match op {
            NirOp::Bind { name, .. } => Some(name.clone()),
            _ => None,
        });
        let (Some(first), Some(second)) = (names.next(), names.next()) else {
            panic!("`main` must bind at least two locals for this row to mean anything");
        };
        assert_ne!(first, second, "the two binds must start out distinct");
        for op in main.body.iter_mut() {
            if let NirOp::Bind { name, .. } = op {
                if *name == second {
                    *name = first.clone();
                    break;
                }
            }
        }
    });
    assert!(
        refused.contains("is declared more than once"),
        "a duplicate local must be refused; it said {refused:?}"
    );

    let refused = refusal("an assignment to an immutable local", |m| {
        let main = m
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .expect("the program declares `main`");
        // Make every bind immutable; `main` assigns to one of them.
        for op in main.body.iter_mut() {
            if let NirOp::Bind { mutable, .. } = op {
                *mutable = false;
            }
        }
    });
    assert!(
        refused.contains("targets immutable local"),
        "an assignment to an immutable local must be refused; it said {refused:?}"
    );

    let refused = refusal("a global store naming no global", |m| {
        for function in m.functions.iter_mut() {
            for op in function.body.iter_mut() {
                if let NirOp::StoreGlobal { name, .. } = op {
                    *name = "$no_such_global".to_string();
                }
            }
        }
    });
    assert!(
        refused.contains("unknown global"),
        "a store to a global that does not exist must be refused; it said {refused:?}"
    );
}

/// Rename the first `NirValue` in the module for which `pick` returns a
/// mutable reference to its name, and return whether one was found.
///
/// A hand-rolled walk rather than a visitor: `NirValue` has no mutable one, and
/// the alternative — a test that reached in by index — would go quiet the day
/// the lowering emits one more temporary.
fn rename_first_value(
    module: &mut NirModule,
    pick: impl Fn(&mut crate::target::shared::nir::NirValue) -> Option<&mut String> + Copy,
) -> bool {
    use crate::target::shared::nir::{NirOp, NirValue};

    fn walk_value(
        value: &mut NirValue,
        pick: impl Fn(&mut NirValue) -> Option<&mut String> + Copy,
    ) -> bool {
        // The node itself first, so the outermost match wins and one mutation
        // is one broken reference.
        if let Some(name) = pick(value) {
            *name = "$no_such_reference".to_string();
            return true;
        }
        match value {
            NirValue::Call { args, .. }
            | NirValue::CallResult { args, .. }
            | NirValue::Constructor { args, .. }
            | NirValue::RuntimeCall { args, .. } => {
                args.iter_mut().any(|arg| walk_value(arg, pick))
            }
            NirValue::Binary { left, right, .. } => {
                walk_value(left, pick) || walk_value(right, pick)
            }
            NirValue::Unary { operand, .. } => walk_value(operand, pick),
            NirValue::UnionWrap { value, .. }
            | NirValue::UnionExtract { value, .. }
            | NirValue::Checked { value, .. }
            | NirValue::ResultIsOk { value }
            | NirValue::ResultValue { value }
            | NirValue::ResultError { value } => walk_value(value, pick),
            _ => false,
        }
    }

    fn walk_op(op: &mut NirOp, pick: impl Fn(&mut NirValue) -> Option<&mut String> + Copy) -> bool {
        match op {
            NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
                value.as_mut().is_some_and(|v| walk_value(v, pick))
            }
            NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } => {
                walk_value(value, pick)
            }
            NirOp::Return { value } => value.as_mut().is_some_and(|v| walk_value(v, pick)),
            NirOp::Eval { value } => walk_value(value, pick),
            NirOp::ExitProgram { code } => walk_value(code, pick),
            NirOp::Fail { error } => walk_value(error, pick),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                walk_value(condition, pick)
                    || then_body.iter_mut().any(|op| walk_op(op, pick))
                    || else_body.iter_mut().any(|op| walk_op(op, pick))
            }
            NirOp::Match { value, cases } => {
                walk_value(value, pick)
                    || cases
                        .iter_mut()
                        .any(|case| case.body.iter_mut().any(|op| walk_op(op, pick)))
            }
            _ => false,
        }
    }

    module
        .functions
        .iter_mut()
        .any(|function| function.body.iter_mut().any(|op| walk_op(op, pick)))
}

/// Every kind of reference a body can make must resolve.
///
/// `validate_body` checks each against the name table its own pass built: a
/// local, a local REF (the by-reference form), a global, a function, a closure
/// target, a call target. Each is a separate `Err` and each was unreached,
/// because a module from a correct lowering names only things that exist.
///
/// A reference that does not resolve is not a diagnostic the user ever sees — it
/// is a lowering bug, and this is the layer that names WHICH reference rather
/// than leaving it to a relocation error two stages later.
#[test]
fn a_reference_that_does_not_resolve_is_refused() {
    use crate::target::shared::nir::NirValue;

    for (what, message, pick) in [
        (
            "a local reference",
            "local reference",
            (|value: &mut NirValue| match value {
                NirValue::Local(name) => Some(name),
                _ => None,
            }) as fn(&mut NirValue) -> Option<&mut String>,
        ),
        (
            "a global reference",
            "global reference",
            |value: &mut NirValue| match value {
                NirValue::Global { name, .. } => Some(name),
                _ => None,
            },
        ),
        (
            "a function reference",
            "function reference",
            |value: &mut NirValue| match value {
                NirValue::FunctionRef { name, .. } => Some(name),
                _ => None,
            },
        ),
        (
            "a call target",
            "call target",
            |value: &mut NirValue| match value {
                NirValue::Call { target, .. } => Some(target),
                _ => None,
            },
        ),
    ] {
        let mut found = false;
        let refused = refusal(what, |m| {
            found = rename_first_value(m, pick);
        });
        assert!(
            found,
            "the program must contain {what} for this row to break one; it has \
             none, so the row is asserting on a module it did not change"
        );
        assert!(
            refused.contains(message) && refused.contains("$no_such_reference"),
            "{what} that does not resolve must be refused with a message \
             containing {message:?} and naming the reference; it said {refused:?}"
        );
    }
}

/// Apply `wreck` to the first op in `main` it says it handled, depth first.
///
/// `wreck` reports whether it recognised the op, so one walk serves every row
/// below: a row that wants the `IF` returns true only for an `If`, and the walk
/// stops there. Targeted rather than "the first op anywhere", because each row
/// is about ONE arm of `validate_ops` — a walk that stopped at whichever op came
/// first would leave the arm it meant to test untouched while still producing a
/// refusal from somewhere else, which is a green row asserting nothing.
///
/// Returning a `&mut` out of the recursive walk instead would thread a lifetime
/// through every arm for no gain.
fn wreck_first_op(module: &mut NirModule, wreck: &mut dyn FnMut(&mut NirOp) -> bool) -> bool {
    fn walk(ops: &mut [NirOp], wreck: &mut dyn FnMut(&mut NirOp) -> bool) -> bool {
        for op in ops.iter_mut() {
            // The node itself first, so an `If` is a candidate before its body.
            if wreck(op) {
                return true;
            }
            let nested = match op {
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => walk(then_body, wreck) || walk(else_body, wreck),
                NirOp::Match { cases, .. } => {
                    cases.iter_mut().any(|case| walk(&mut case.body, wreck))
                }
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => walk(body, wreck),
                _ => false,
            };
            if nested {
                return true;
            }
        }
        false
    }

    module
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .is_some_and(|function| walk(&mut function.body, wreck))
}

/// A module whose entry point disagrees with the function it names.
///
/// The entry is written by lowering from the same `EntryPoint` the function was
/// built from, so the two agree by construction and neither check had ever
/// fired. Neither is redundant with the type checker: by this stage the source
/// is gone, and an entry whose return type disagrees with its function's is a
/// program the encoder emits a return sequence for against a register the
/// callee never wrote.
#[test]
fn the_entry_point_must_match_the_function_it_names() {
    for (what, message, mutate) in [
        (
            "a return type the function does not have",
            "does not match function return type",
            Box::new(|m: &mut NirModule| {
                let entry = m.entry.as_mut().expect("the program has an entry");
                entry.returns = crate::types::ParameterType::String;
            }) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "parameters on an entry that does not accept args",
            "does not accept args but function has parameters",
            Box::new(|m: &mut NirModule| {
                let entry = m
                    .entry
                    .as_ref()
                    .expect("the program has an entry")
                    .name
                    .clone();
                let function = m
                    .functions
                    .iter_mut()
                    .find(|f| f.name == entry)
                    .expect("the entry names a function");
                function.params.push(crate::target::shared::nir::NirParam {
                    name: "argv".to_string(),
                    type_: crate::types::ParameterType::String,
                    default: None,
                });
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with a message containing {message:?}; it \
             said {refused:?}"
        );
    }
}

/// A parameter needs a name, a type, and a name no other local has.
///
/// A nameless parameter has no slot to bind; a duplicate silently shadows its
/// twin, so every read of the first resolves to the second — a wrong VALUE, not
/// a failure to build.
#[test]
fn a_parameter_needs_a_name_a_type_and_a_unique_one() {
    fn first_with_params(m: &mut NirModule) -> &mut crate::target::shared::nir::NirFunction {
        m.functions
            .iter_mut()
            .find(|f| !f.params.is_empty())
            .expect("the program declares a function with a parameter")
    }

    for (what, message, mutate) in [
        (
            "a parameter with no name",
            "has a parameter with empty name or type",
            Box::new(|m: &mut NirModule| first_with_params(m).params[0].name.clear())
                as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "two parameters with one name",
            "has duplicate local",
            Box::new(|m: &mut NirModule| {
                let function = first_with_params(m);
                let twin = crate::target::shared::nir::NirParam {
                    name: function.params[0].name.clone(),
                    type_: function.params[0].type_.clone(),
                    default: None,
                };
                function.params.push(twin);
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with a message containing {message:?}; it \
             said {refused:?}"
        );
    }
}

/// An op that names something must name something real, and a bind must name
/// something new.
///
/// One row per arm of `validate_ops`, and each names a different kind of
/// damage. An assignment to an unknown local writes a slot nobody reserved. A
/// duplicate bind gives two values one slot. A store to an unknown global
/// relocates against a symbol nothing defines — which the linker does
/// eventually catch, two stages later and without the name of the function it
/// came from.
///
/// The last two rows put the unresolvable reference inside a nested body rather
/// than at the top of one, which is what covers the `?` that carries a failure
/// back out of an `If` or a `MATCH` case.
#[test]
fn an_op_that_names_something_must_name_something_real() {
    use crate::target::shared::nir::NirValue;

    for (what, message, mutate) in [
        (
            "a bind with no name",
            "bind op has empty name or type",
            Box::new(|m: &mut NirModule| {
                assert!(wreck_first_op(m, &mut |op| match op {
                    NirOp::Bind { name, .. } => {
                        name.clear();
                        true
                    }
                    _ => false,
                }));
            }) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "a local bound twice",
            "is declared more than once",
            Box::new(|m: &mut NirModule| {
                // The second bind renamed onto the first. Duplicating a row
                // instead would want `NirOp: Clone` for a test's convenience.
                let mut first: Option<String> = None;
                assert!(wreck_first_op(m, &mut |op| match op {
                    NirOp::Bind { name, .. } => match &first {
                        None => {
                            first = Some(name.clone());
                            false
                        }
                        Some(taken) => {
                            *name = taken.clone();
                            true
                        }
                    },
                    _ => false,
                }));
            }),
        ),
        (
            "an assignment to a local that does not exist",
            "assignment targets unknown local",
            Box::new(|m: &mut NirModule| {
                assert!(wreck_first_op(m, &mut |op| match op {
                    NirOp::Assign { name, .. } => {
                        *name = "$no_such_local".to_string();
                        true
                    }
                    _ => false,
                }));
            }),
        ),
        (
            "a store to a global that does not exist",
            "global store targets unknown global",
            Box::new(|m: &mut NirModule| {
                assert!(wreck_first_op(m, &mut |op| match op {
                    NirOp::StoreGlobal { name, .. } => {
                        *name = "$no_such_global".to_string();
                        true
                    }
                    _ => false,
                }));
            }),
        ),
        (
            "an unresolvable reference in an IF condition",
            "local reference",
            Box::new(|m: &mut NirModule| {
                assert!(wreck_first_op(m, &mut |op| match op {
                    NirOp::If { condition, .. } => {
                        *condition = NirValue::Local("$no_such_local".to_string());
                        true
                    }
                    _ => false,
                }));
            }),
        ),
        (
            "an unresolvable reference in a MATCH scrutinee",
            "local reference",
            Box::new(|m: &mut NirModule| {
                assert!(wreck_first_op(m, &mut |op| match op {
                    NirOp::Match { value, .. } => {
                        *value = NirValue::Local("$no_such_local".to_string());
                        true
                    }
                    _ => false,
                }));
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with a message containing {message:?}; it \
             said {refused:?}"
        );
    }
}
