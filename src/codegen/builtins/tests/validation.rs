//! `NativeCodePlan::validate` refuses each malformed plan it is written to catch.
//!
//! Validation is the last thing between a codegen bug and an object file. It
//! runs twice per backend — once at the tail of `lower_module_for_platform` and
//! once on the returned plan — and every rule in it exists because something
//! reached the encoder, or the linker, or the running program instead.
//!
//! Nothing in process ever reached one. A plan produced by a *correct* lowering
//! is well-formed by construction, so the corpus exercises the walk and none of
//! the twenty-one refusals: they are the branch not taken on every one of the
//! 424 fixtures, five backends each.
//!
//! The shape here is one valid plan, mutated one field at a time. That is what
//! makes each row a test of its own rule rather than of validation in general —
//! a mutation that broke two invariants at once would be caught by whichever
//! check ran first, and the rule under test could be deleted without the row
//! noticing.
//!
//! The base plan is a real lowering, not a hand-built stub. A stub would be
//! valid only in the ways the author remembered, and the rows that "caught" it
//! would be catching the stub.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::{CodeDataObject, CodeImport, NativeCodePlan, RelocIntent};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

/// A program that reaches libc, so its plan has imports, data objects, external
/// relocations and internal calls to mutate.
const SRC: &str = "\
IMPORT io
IMPORT strings

FUNC greet(who AS String) AS String
  RETURN \"hello, \" & strings::upper(who)
END FUNC

FUNC main() AS Integer
  io::print(greet(\"world\"))
  RETURN 0
END FUNC
";

/// A freshly lowered, valid plan. One per mutation: `NativeCodePlan` is not
/// `Clone`, and sharing one would let an earlier row's mutation leak into a
/// later one's premise.
fn plan() -> NativeCodePlan {
    try_code_for_src(SRC, CodeTarget::LinuxX86_64, Console).expect("the base program must lower")
}

/// The plan a correct lowering produces passes.
///
/// Every row below asserts a mutated plan is REFUSED, and all of them would
/// pass just as well if `validate` refused everything. This is the row that
/// says it does not.
#[test]
fn a_plan_from_a_real_lowering_validates() {
    plan()
        .validate()
        .expect("a plan from a correct lowering must validate");
}

/// Apply `mutate` to a fresh plan and return the refusal it must produce.
fn refusal(what: &str, mutate: impl FnOnce(&mut NativeCodePlan)) -> String {
    let mut plan = plan();
    mutate(&mut plan);
    match plan.validate() {
        Err(message) => message,
        Ok(()) => panic!(
            "validate accepted a plan with {what}. This is the last check before \
             an object file is written, so accepting it means the failure moves \
             to the encoder, the linker, or the running program"
        ),
    }
}

/// Every whole-plan header field is required.
#[test]
fn the_plan_header_fields_are_all_required() {
    for (what, message, mutate) in [
        (
            "an empty target",
            "target must not be empty",
            Box::new(|p: &mut NativeCodePlan| p.target.clear()) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "an empty arch",
            "arch must not be empty",
            Box::new(|p: &mut NativeCodePlan| p.arch.clear()),
        ),
        (
            "an empty project name",
            "project name must not be empty",
            Box::new(|p: &mut NativeCodePlan| p.project.clear()),
        ),
        (
            "no functions at all",
            "requires at least one function",
            Box::new(|p: &mut NativeCodePlan| p.functions.clear()),
        ),
    ] {
        let refused = refusal(what, mutate);
        assert!(
            refused.contains(message),
            "a plan with {what} must be refused with a message containing \
             {message:?}; it said {refused:?}"
        );
    }
}

/// An entry symbol naming no function is refused.
///
/// The consequence otherwise is a linked executable whose entry address points
/// at nothing — which the linker reports, if it reports it, as an undefined
/// symbol with no hint that the *plan* named it.
#[test]
fn an_entry_symbol_that_resolves_to_no_function_is_refused() {
    let refused = refusal("an unresolvable entry symbol", |p| {
        p.entry_symbol = Some("_no_such_entry".to_string());
    });
    assert!(
        refused.contains("entry symbol '_no_such_entry' does not resolve"),
        "the refusal must name the symbol that did not resolve; it said {refused:?}"
    );
}

/// An import missing either half is refused.
#[test]
fn an_incomplete_import_is_refused() {
    for (what, import) in [
        (
            "an import with no library",
            CodeImport {
                library: String::new(),
                symbol: "malloc".to_string(),
            },
        ),
        (
            "an import with no symbol",
            CodeImport {
                library: "libc.so.6".to_string(),
                symbol: String::new(),
            },
        ),
    ] {
        let refused = refusal(what, |p| p.imports.push(import));
        assert!(
            refused.contains("incomplete import"),
            "{what} must be refused as incomplete; it said {refused:?}"
        );
    }
}

/// A data object missing a field, or sized zero, is refused.
///
/// A zero-size or zero-alignment object is the one that matters: the data
/// layout divides by the alignment, and a run of zero-size objects all land on
/// the same address, so two globals silently alias.
#[test]
fn a_malformed_data_object_is_refused() {
    let well_formed = || CodeDataObject {
        symbol: "_probe_obj".to_string(),
        kind: "bytes".to_string(),
        layout: "const".to_string(),
        align: 8,
        size: 8,
        value: "0000000000000000".to_string(),
    };
    for (what, message, broken) in [
        (
            "a data object with no symbol",
            "incomplete data object",
            CodeDataObject {
                symbol: String::new(),
                ..well_formed()
            },
        ),
        (
            "a data object with no kind",
            "incomplete data object",
            CodeDataObject {
                kind: String::new(),
                ..well_formed()
            },
        ),
        (
            "a data object with no layout",
            "incomplete data object",
            CodeDataObject {
                layout: String::new(),
                ..well_formed()
            },
        ),
        (
            "a data object of zero size",
            "must have nonzero size and alignment",
            CodeDataObject {
                size: 0,
                ..well_formed()
            },
        ),
        (
            "a data object of zero alignment",
            "must have nonzero size and alignment",
            CodeDataObject {
                align: 0,
                ..well_formed()
            },
        ),
    ] {
        let refused = refusal(what, |p| p.data_objects.push(broken));
        assert!(
            refused.contains(message),
            "{what} must be refused with {message:?}; it said {refused:?}"
        );
    }
}

/// A function with no name, no instructions, or no return is refused.
///
/// "No return instruction" is the one with teeth: a body that falls off its end
/// executes whatever the assembler laid down next, which is the following
/// function's prologue. That is the failure mode
/// `abi-function-lowering-needs-its-own-epilogue` names, and it is invisible in
/// every behavioural test that does not happen to call the victim.
#[test]
fn a_malformed_function_is_refused() {
    for (what, message, mutate) in [
        (
            "a function with no name",
            "name and symbol must not be empty",
            Box::new(|p: &mut NativeCodePlan| p.functions[0].name.clear())
                as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "a function with no instructions",
            "has no instructions",
            Box::new(|p: &mut NativeCodePlan| p.functions[0].instructions.clear()),
        ),
        (
            "a function that never returns",
            "has no return instruction",
            Box::new(|p: &mut NativeCodePlan| {
                p.functions[0].instructions.retain(|i| i.op != CodeOp::Ret)
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

/// A branch to a label the function does not define is refused HERE.
///
/// bug-300 E9: `CodeInstruction::validate` only checks a branch HAS a `target`,
/// never that the label exists, so this used to reach the encoder and surface as
/// "branch target label does not resolve" — with no function named. Failing at
/// the layer that owns the invariant is the whole point of the rule.
#[test]
fn a_branch_to_an_undefined_label_is_refused_with_the_function_named() {
    let refused = refusal("a branch to a label it does not define", |p| {
        let function = &mut p.functions[0];
        let at = function
            .instructions
            .iter()
            .position(|i| i.op == CodeOp::Ret)
            .expect("the function returns");
        function.instructions.insert(
            at,
            crate::codegen::engine::types::CodeInstruction::new("b")
                .field("target", Operand::from("_no_such_label")),
        );
    });
    assert!(
        refused.contains("branches to label '_no_such_label'"),
        "the refusal must name the label; it said {refused:?}"
    );
    assert!(
        refused.contains("native code function '"),
        "the refusal must name the FUNCTION -- naming only the label is what the \
         encoder already did, and is why this rule was added; it said {refused:?}"
    );
}

/// Each relocation binding is checked against the symbol table it belongs to.
///
/// The three bindings resolve against three different tables, and a relocation
/// pointed at the wrong one is a call that binds to whatever the loader turns
/// up. The `library` half is the same question from the other side: an internal
/// relocation naming a library would be emitted as an import stub.
#[test]
fn a_relocation_is_refused_when_it_does_not_match_its_binding() {
    for (what, message, mutate) in [
        (
            "an internal relocation to an undefined symbol",
            "internal relocation target '_nowhere' is not defined",
            Box::new(|p: &mut NativeCodePlan| {
                let function = &mut p.functions[0];
                function.relocations.push(reloc(
                    &function.symbol.clone(),
                    "_nowhere",
                    "internal",
                    None,
                ));
            }) as Box<dyn FnOnce(&mut _)>,
        ),
        (
            "an internal relocation that names a library",
            "must not name a library",
            Box::new(|p: &mut NativeCodePlan| {
                let symbol = p.functions[0].symbol.clone();
                p.functions[0].relocations.push(reloc(
                    &symbol,
                    &symbol,
                    "internal",
                    Some("libc.so.6"),
                ));
            }),
        ),
        (
            "an external relocation to a symbol nothing imports",
            "external relocation target '_nowhere' is not imported",
            Box::new(|p: &mut NativeCodePlan| {
                let symbol = p.functions[0].symbol.clone();
                p.functions[0].relocations.push(reloc(
                    &symbol,
                    "_nowhere",
                    "external",
                    Some("libc.so.6"),
                ));
            }),
        ),
        (
            "a data relocation to a symbol that is neither",
            "is not a data object or defined symbol",
            Box::new(|p: &mut NativeCodePlan| {
                let symbol = p.functions[0].symbol.clone();
                p.functions[0]
                    .relocations
                    .push(reloc(&symbol, "_nowhere", "data", None));
            }),
        ),
        (
            "a relocation whose source is another function",
            "does not match function",
            Box::new(|p: &mut NativeCodePlan| {
                let symbol = p.functions[0].symbol.clone();
                p.functions[0]
                    .relocations
                    .push(reloc("_someone_else", &symbol, "internal", None));
            }),
        ),
        (
            "a relocation with a binding that is not one of the three",
            "has invalid binding 'weak'",
            Box::new(|p: &mut NativeCodePlan| {
                let symbol = p.functions[0].symbol.clone();
                p.functions[0]
                    .relocations
                    .push(reloc(&symbol, &symbol, "weak", None));
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

fn reloc(
    from: &str,
    to: &str,
    binding: &str,
    library: Option<&str>,
) -> crate::codegen::engine::types::CodeRelocation {
    crate::codegen::engine::types::CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: RelocIntent::Call,
        binding: binding.to_string(),
        library: library.map(str::to_string),
    }
}
