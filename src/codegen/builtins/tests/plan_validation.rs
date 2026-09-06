//! `NativePlan::validate` refuses each malformed plan it is written to catch.
//!
//! The plan validator is the gate between planning/regalloc and the encoder:
//! every backend runs it before laying out bytes, so it is the last place a
//! planning bug is described in words rather than as a wrong relocation. And,
//! exactly like the NIR validator one stage above it, every one of its refusals
//! was unreached — a plan from a correct lowering is well formed by
//! construction, so the corpus walks the whole validator 623 fixtures x 5
//! backends and takes none of its `Err` arms. `target/shared/plan/mod.rs` was at
//! 73.57%, and almost all of the gap is `return Err`.
//!
//! One real plan, mutated one field at a time. A mutation that broke two
//! invariants at once would be caught by whichever check ran first, and the rule
//! under test could be deleted without the row noticing.

use crate::target::shared::plan::{
    CallKind, NativePlan, PlanCall, PlanLabel, PlannedParam, StackSlot, StorageClass, StorageType,
};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{native_plan_for_src, CodeTarget};

/// A program that plans into everything the validator inspects: parameters,
/// stack slots, labels from an `IF` and a `WHILE`, a local call, a runtime call
/// (`io::print`), and an INDIRECT call through a `FUNC`-typed value — the last
/// one because the indirect rule is the only one that refuses the presence of a
/// symbol rather than its absence.
const SRC: &str = "\
IMPORT io

FUNC twice(n AS Integer) AS Integer
  RETURN n * 2
END FUNC

FUNC apply(f AS FUNC(Integer) AS Integer, n AS Integer) AS Integer
  RETURN f(n)
END FUNC

FUNC main() AS Integer
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 3
    IF i > 1 THEN
      total = total + twice(i)
    END IF
    i = i + 1
  END WHILE
  io::print(\"total=\" & toString(apply(twice, total)))
  RETURN 0
END FUNC
";

fn plan() -> NativePlan {
    native_plan_for_src(SRC, CodeTarget::LinuxX86_64, Console).expect("the program must plan")
}

/// The message `validate` gives for a plan `mutate` has damaged, or a panic.
fn refusal(what: &str, mutate: impl FnOnce(&mut NativePlan)) -> String {
    let mut plan = plan();
    mutate(&mut plan);
    match plan.validate() {
        Err(message) => message,
        Ok(()) => panic!(
            "NativePlan::validate accepted a plan with {what}. This is the last \
             gate before the encoder, so accepting it turns a describable \
             planning bug into a wrong relocation or a bad frame"
        ),
    }
}

/// The plan's own header must be complete and self-consistent.
#[test]
fn the_plan_header_must_be_complete() {
    for (what, message, mutate) in [
        (
            "an empty target",
            "native plan target must not be empty",
            Box::new(|p: &mut NativePlan| p.target.clear()) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "an empty project name",
            "native plan project name must not be empty",
            Box::new(|p: &mut NativePlan| p.project.clear()),
        ),
        (
            "no functions",
            "native plan requires at least one function",
            Box::new(|p: &mut NativePlan| p.functions.clear()),
        ),
        (
            "an entry symbol no function defines",
            "does not resolve",
            Box::new(|p: &mut NativePlan| {
                p.entry_symbol = Some("_no_such_entry".to_string());
            }),
        ),
        (
            "an empty required symbol",
            "native plan contains an empty required symbol",
            Box::new(|p: &mut NativePlan| p.runtime_symbols.push(String::new())),
        ),
        (
            "an incomplete platform import",
            "native plan contains an incomplete platform import",
            Box::new(|p: &mut NativePlan| {
                p.platform_imports
                    .push(crate::target::shared::plan::PlatformImport {
                        library: "libc".to_string(),
                        symbol: String::new(),
                        required_by: "main".to_string(),
                    });
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with a message containing {message:?}; it \\
             said {refused:?}"
        );
    }
}

/// Damage the planned function named `name`.
fn wreck(
    plan: &mut NativePlan,
    name: &str,
    damage: impl FnOnce(&mut crate::target::shared::plan::PlannedFunction),
) {
    let function = plan
        .functions
        .iter_mut()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("the program plans a `{name}`"));
    damage(function);
}

/// Damage `main`'s planned function.
fn wreck_main(
    plan: &mut NativePlan,
    damage: impl FnOnce(&mut crate::target::shared::plan::PlannedFunction),
) {
    wreck(plan, "main", damage)
}

fn word_storage() -> StorageType {
    StorageType {
        name: "Integer".to_string(),
        class: StorageClass::Integer,
        size: 8,
        align: 8,
    }
}

/// Everything a planned function names must be named, and its stack slots must
/// be on the stack.
///
/// The offset rule is the one that is not bookkeeping. A stack slot's offset is
/// negative because it is below the frame pointer; a non-negative one addresses
/// the CALLER's frame, so a plan that carried one would emit a function reading
/// and writing its caller's locals. That is not a crash, it is a wrong value in
/// another function.
#[test]
fn a_planned_function_must_name_what_it_uses() {
    for (what, message, mutate) in [
        (
            // `twice`, not `main`: the entry-symbol check runs first and would
            // catch a symbol-less `main` under its own message, leaving this
            // rule untested behind a green row.
            "an empty function symbol",
            "function name and symbol must not be empty",
            Box::new(|p: &mut NativePlan| wreck(p, "twice", |f| f.symbol.clear()))
                as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "an empty parameter name",
            "has an empty parameter name",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.params.push(PlannedParam {
                        name: String::new(),
                        storage: word_storage(),
                    });
                });
            }),
        ),
        (
            "an empty stack slot name",
            "has an empty stack slot name",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.local_slots.push(StackSlot {
                        name: String::new(),
                        storage: word_storage(),
                        offset: -8,
                        mutable: false,
                    });
                });
            }),
        ),
        (
            "a stack slot addressing the caller's frame",
            "has non-stack offset",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.local_slots.push(StackSlot {
                        name: "above_the_frame".to_string(),
                        storage: word_storage(),
                        offset: 16,
                        mutable: false,
                    });
                });
            }),
        ),
        (
            "an empty label name",
            "has an empty label name",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.labels.push(PlanLabel {
                        name: String::new(),
                        kind: crate::target::shared::plan::LabelKind::IfEnd,
                    });
                });
            }),
        ),
        (
            "no planned operations",
            "has no planned operations",
            Box::new(|p: &mut NativePlan| wreck_main(p, |f| f.operations.clear())),
        ),
        (
            "an empty call target",
            "has an empty call target",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.calls.push(PlanCall {
                        target: String::new(),
                        symbol: "_something".to_string(),
                        kind: CallKind::Local,
                        string_literals: Vec::new(),
                    });
                });
            }),
        ),
        (
            "a direct call with no symbol to link against",
            "with an empty symbol",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.calls.push(PlanCall {
                        target: "twice".to_string(),
                        symbol: String::new(),
                        kind: CallKind::Local,
                        string_literals: Vec::new(),
                    });
                });
            }),
        ),
        (
            "an indirect call carrying a linker symbol",
            "carries a linker symbol",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.calls.push(PlanCall {
                        target: "f".to_string(),
                        symbol: "_mfb_twice".to_string(),
                        kind: CallKind::Indirect,
                        string_literals: Vec::new(),
                    });
                });
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with a message containing {message:?}; it \\
             said {refused:?}"
        );
    }
}

/// A storage type must describe storage that exists.
///
/// `Void` is size 0 align 1 and everything else is nonzero — a zero-sized
/// `Integer` slot would have every later slot laid out on top of it.
#[test]
fn a_storage_type_must_describe_real_storage() {
    for (what, message, mutate) in [
        (
            "a storage type with no name",
            "storage type name must not be empty",
            Box::new(|p: &mut NativePlan| wreck_main(p, |f| f.returns.name.clear()))
                as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "a void storage type with a size",
            "must be size 0 align 1",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.returns = StorageType {
                        name: "Nothing".to_string(),
                        class: StorageClass::Void,
                        size: 8,
                        align: 8,
                    };
                });
            }),
        ),
        (
            "a sized storage type with no size",
            "must have nonzero size and alignment",
            Box::new(|p: &mut NativePlan| {
                wreck_main(p, |f| {
                    f.returns = StorageType {
                        name: "Integer".to_string(),
                        class: StorageClass::Integer,
                        size: 0,
                        align: 0,
                    };
                });
            }),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "{what} must be refused with a message containing {message:?}; it \\
             said {refused:?}"
        );
    }
}

/// The plan the program really produces passes its own validator, on every
/// backend.
///
/// The row that keeps the rest honest: a validator that refused everything
/// would satisfy every assertion above.
#[test]
fn a_real_plan_validates_on_every_backend() {
    for target in CodeTarget::ALL {
        let plan = native_plan_for_src(SRC, target, Console)
            .unwrap_or_else(|err| panic!("{}: {err}", target.name()));
        plan.validate()
            .unwrap_or_else(|err| panic!("{}: a real plan must validate: {err}", target.name()));
    }
}
