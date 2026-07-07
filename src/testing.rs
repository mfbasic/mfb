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
    AstProject, Function, FunctionKind, Import, Item, TestCase, TestGroup, TestGroupMember,
    Visibility,
};
use crate::coverage::CovSlot;
use std::path::Path;

mod desugar;

pub(crate) use desugar::{expand_expect, validate_expect_placement};

/// The outcome of lowering the `TESTING` blocks: the synthesized entry-point name
/// (test mode only) and the coverage slot map (`--coverage` only).
pub(crate) struct TestLowering {
    /// The driver entry-point name, overriding the manifest entry in test mode.
    pub(crate) entry: Option<String>,
    /// The `slot -> (file, line)` coverage map, empty unless `--coverage`.
    pub(crate) cov_slots: Vec<CovSlot>,
}

/// The `coverage.*` sidecar file names written into the project directory during
/// a `--coverage` run (plan-18-C D4).
pub(crate) const COVMAP_FILE: &str = "coverage.covmap.json";
pub(crate) const COVDATA_FILE: &str = "coverage.covdata";
pub(crate) const COVFAIL_FILE: &str = "coverage.covfail";
pub(crate) const COVERAGE_HTML: &str = "coverage.html";

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

/// Resolve every `TESTING` block in the project according to `mode`. In build
/// mode the blocks are dropped. In test mode they are desugared into a runnable
/// driver (entry point returned); with `--coverage`, user statements are also
/// instrumented and the slot map is returned. `project_dir` is the absolute
/// project directory, used to bake the coverage sidecar paths into the driver.
pub(crate) fn lower_testing_blocks(
    ast: &mut AstProject,
    mode: CompileMode,
    project_dir: &Path,
) -> TestLowering {
    match mode {
        CompileMode::Build => {
            for file in &mut ast.files {
                file.items.retain(|item| !matches!(item, Item::Testing(_)));
            }
            TestLowering {
                entry: None,
                cov_slots: Vec::new(),
            }
        }
        CompileMode::Test { coverage } => {
            let cov_slots = desugar_project(ast, coverage, project_dir);
            TestLowering {
                entry: Some(DRIVER_NAME.to_string()),
                cov_slots,
            }
        }
    }
}

/// Test-mode lowering: replace every file's `TESTING` blocks with the generated
/// case `SUB`s *in that same file*, then append the driver `FUNC` to the first
/// file. Keeping each case SUB in its originating file means its body inherits
/// that file's import scope — a `bits::` call stays in the file that
/// `IMPORT bits`, and any file-local `PRIVATE` references (already rewritten by
/// `scope_privates`) still resolve against that file's declarations.
/// With `coverage`, additionally instrument the user statements and emit the
/// coverage runtime helpers; returns the coverage slot map (empty otherwise).
fn desugar_project(ast: &mut AstProject, coverage: bool, project_dir: &Path) -> Vec<CovSlot> {
    // Enumerate the report steps (group headers and cases) in declaration order
    // across all files; each case's index (globally unique across the project)
    // names its generated SUB. The step order the driver iterates is exactly this
    // order.
    let mut steps: Vec<desugar::DriverStep> = Vec::new();
    let mut case_index = 0usize;

    for file in &mut ast.files {
        let mut replacement: Vec<Item> = Vec::new();
        let mut generated: Vec<Function> = Vec::new();
        for item in std::mem::take(&mut file.items) {
            match item {
                Item::Testing(block) => {
                    for group in block.groups {
                        lower_group(group, 0, &mut steps, &mut generated, &mut case_index);
                    }
                }
                other => replacement.push(other),
            }
        }
        // The generated case SUBs stay in the file they were declared in, after
        // that file's ordinary items.
        replacement.extend(generated.into_iter().map(Item::Function));
        file.items = replacement;
    }

    // The driver (entry point) goes into the first file — there is always at least
    // one source file in a project. It streams the report through `io::print`, so
    // ensure that file imports `io`; the case SUBs it calls are Public, so the
    // cross-file calls resolve regardless of which file each case lives in.
    let driver = desugar::build_driver(&steps, coverage);
    let sink = ast
        .files
        .first_mut()
        .expect("a project has at least one source file");
    ensure_import(sink, "io");
    sink.items.push(Item::Function(driver));

    // Coverage instrumentation runs last: it walks the now-complete item list
    // (skipping the generated driver/helpers by name), injects hit counters, and
    // appends the coverage runtime helpers + global counter array.
    if coverage {
        desugar::instrument_coverage(ast, project_dir)
    } else {
        Vec::new()
    }
}

fn ensure_import(file: &mut crate::ast::AstFile, module: &str) {
    if !file.imports.iter().any(|import| import.module == module) {
        file.imports.push(Import {
            module: module.to_string(),
            alias: None,
            line: 0,
        });
    }
}

/// Lower one `TGROUP` at nesting `depth` (top-level groups are depth 0) into an
/// ordered run of driver steps: a header for the group followed by, in source
/// order, one step per direct `TCASE` and a recursive run per nested `TGROUP`.
/// A group with no case anywhere in its subtree emits nothing — an empty header
/// would be noise — matching the pre-nesting behaviour for a case-less group.
fn lower_group(
    group: TestGroup,
    depth: usize,
    steps: &mut Vec<desugar::DriverStep>,
    generated: &mut Vec<Function>,
    case_index: &mut usize,
) {
    if !group_has_cases(&group) {
        return;
    }
    steps.push(desugar::DriverStep::Group {
        indent: depth * 2,
        description: group.description,
    });
    for member in group.members {
        match member {
            TestGroupMember::Case(case) => {
                // One generated SUB per case; its index is the running case count
                // across the whole project, so the name is globally unique even
                // though the SUB stays in its own file.
                let index = *case_index;
                *case_index += 1;
                let sub_name = format!("__mfb_test_case_{index}");
                let TestCase {
                    description, body, ..
                } = case;
                let desugared_body = desugar::desugar_case_body(body);
                generated.push(Function {
                    kind: FunctionKind::Sub,
                    // Public so the driver (in the first file) can call it across
                    // file boundaries; the unique generated name avoids collision.
                    visibility: Visibility::Public,
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
                steps.push(desugar::DriverStep::Case {
                    sub_name,
                    description,
                    indent: (depth + 1) * 2,
                });
            }
            TestGroupMember::Group(nested) => {
                lower_group(nested, depth + 1, steps, generated, case_index);
            }
        }
    }
}

/// Whether a group has at least one `TCASE` anywhere in its subtree.
fn group_has_cases(group: &TestGroup) -> bool {
    group.members.iter().any(|member| match member {
        TestGroupMember::Case(_) => true,
        TestGroupMember::Group(nested) => group_has_cases(nested),
    })
}
