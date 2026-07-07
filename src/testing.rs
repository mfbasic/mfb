//! Built-in test framework lowering (plan-18-testing.md).
//!
//! `TESTING … END TESTING` blocks are parsed into [`ast::Item::Testing`] and then
//! resolved here, immediately after parsing and before every other front-end
//! pass, so the rest of the compiler only ever sees ordinary declarations:
//!
//!   * **`mfb build`** ([`CompileMode::Build`]) — every `TESTING` block is
//!     dropped. The emitted binary is byte-identical to one whose blocks were
//!     physically deleted (the plan-18-A build-exclusion gate).
//!   * **`mfb test`** ([`CompileMode::Test`]) — each `TCASE` desugars to a
//!     generated parameterless `SUB`, and a synthesized driver `FUNC` (the entry
//!     point) runs every case under per-case trap isolation, streams the
//!     pass/fail tree, and exits non-zero iff any case failed.

use crate::ast::{
    AstProject, Function, FunctionKind, Import, Item, Statement, TestCase, TestGroup, Visibility,
};

mod desugar;

pub(crate) use desugar::{expand_expect, validate_expect_placement};

/// Whether a compilation is an ordinary build or a `mfb test` run. Threaded from
/// the CLI into the front end so `TESTING` blocks are dropped or retained
/// accordingly (plan-18-A §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompileMode {
    /// `mfb build`: drop `TESTING` blocks before codegen.
    Build,
    /// `mfb test`: desugar `TESTING` blocks into a runnable driver. `coverage`
    /// enables the `--coverage` instrumentation (plan-18-C).
    Test { coverage: bool },
}

impl CompileMode {
    pub(crate) fn is_test(self) -> bool {
        matches!(self, CompileMode::Test { .. })
    }

    pub(crate) fn coverage(self) -> bool {
        matches!(self, CompileMode::Test { coverage: true })
    }
}

/// The synthesized `mfb test` entry-point function name. The `__mfb_test_`
/// prefix makes a user collision vanishingly unlikely; a plain (non-sigil) name
/// is used so it lowers as an ordinary function and can serve as the NIR entry.
pub(crate) const DRIVER_NAME: &str = "__mfb_test_main";

/// Resolve every `TESTING` block in the project according to `mode`. Returns the
/// synthesized driver entry-point name in test mode (to override the manifest
/// entry), or `None` in build mode (blocks simply dropped).
pub(crate) fn lower_testing_blocks(ast: &mut AstProject, mode: CompileMode) -> Option<String> {
    match mode {
        CompileMode::Build => {
            for file in &mut ast.files {
                file.items.retain(|item| !matches!(item, Item::Testing(_)));
            }
            None
        }
        CompileMode::Test { .. } => Some(desugar_project(ast)),
    }
}

/// Test-mode lowering: collect every case across every file, replace the
/// `TESTING` blocks with the generated case `SUB`s, and append the driver `FUNC`.
fn desugar_project(ast: &mut AstProject) -> String {
    // Enumerate cases in declaration order across all files, assigning each a
    // unique generated SUB name. The registration order the driver iterates is
    // exactly this order.
    let mut registrations: Vec<desugar::Registration> = Vec::new();
    let mut generated: Vec<Function> = Vec::new();

    for file in &mut ast.files {
        let mut replacement: Vec<Item> = Vec::new();
        for item in std::mem::take(&mut file.items) {
            match item {
                Item::Testing(block) => {
                    for group in block.groups {
                        lower_group(group, &mut registrations, &mut generated);
                    }
                }
                other => replacement.push(other),
            }
        }
        file.items = replacement;
    }

    // Emit the generated case SUBs and the driver into the first file (there is
    // always at least one source file in a project).
    let driver = desugar::build_driver(&registrations);
    let sink = ast
        .files
        .first_mut()
        .expect("a project has at least one source file");
    // The driver streams the report through `io::print`; ensure the host file
    // imports `io` so the qualified call resolves.
    if !sink.imports.iter().any(|import| import.module == "io") {
        sink.imports.push(Import {
            module: "io".to_string(),
            alias: None,
            line: 0,
        });
    }
    for func in generated {
        sink.items.push(Item::Function(func));
    }
    sink.items.push(Item::Function(driver));

    DRIVER_NAME.to_string()
}

fn lower_group(
    group: TestGroup,
    registrations: &mut Vec<desugar::Registration>,
    generated: &mut Vec<Function>,
) {
    for case in group.cases {
        let index = registrations.len();
        let sub_name = format!("__mfb_test_case_{index}");
        let TestCase {
            description, body, ..
        } = case;
        let desugared_body = desugar::desugar_case_body(body);
        generated.push(Function {
            kind: FunctionKind::Sub,
            visibility: Visibility::Private,
            isolated: false,
            name: sub_name.clone(),
            template_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            return_resource: false,
            return_state_type: None,
            body: desugared_body,
            trap: None,
            line: 0,
        });
        registrations.push(desugar::Registration {
            group: group.description.clone(),
            case: description,
            sub_name,
        });
    }
}
