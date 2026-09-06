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

use crate::target::shared::nir::NirModule;
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
  LET d AS Shape = Dot[counter]
  MUT n AS Integer = 0
  n = n + 1
  counter = counter + n
  io::print(label & describe(d) & toString(n))
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
