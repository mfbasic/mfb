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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::AstFile;

    /// A project source carrying one `TESTING` block: a group with a direct case
    /// and a nested sub-group holding another case, plus a case-less group that
    /// must vanish from the driver.
    const SOURCE: &str = "\
TESTING
  TGROUP \"math\"
    TCASE \"adds\"
      expectEqual(1 + 1, 2)
    END TCASE
    TGROUP \"nested\"
      TCASE \"multiplies\"
        expectInteger(3, 3)
      END TCASE
    END TGROUP
  END TGROUP
  TGROUP \"no cases here\"
  END TGROUP
END TESTING
";

    fn driver<'a>(ast: &'a AstProject) -> Option<&'a Function> {
        ast.files[0].items.iter().find_map(|item| match item {
            Item::Function(function) if function.name == DRIVER_NAME => Some(function),
            _ => None,
        })
    }

    fn function_names(ast: &AstProject) -> Vec<String> {
        ast.files
            .iter()
            .flat_map(|file| file.items.iter())
            .filter_map(|item| match item {
                Item::Function(function) => Some(function.name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn build_mode_drops_testing_blocks_and_synthesizes_nothing() {
        let mut ast = crate::testutil::project_from_src(SOURCE);
        let lowering = lower_testing_blocks(&mut ast, CompileMode::Build, Path::new("/tmp"));
        assert!(lowering.entry.is_none());
        assert!(lowering.cov_slots.is_empty());
        // No `TESTING` item survives, and no driver/case SUB was generated.
        assert!(!ast
            .files
            .iter()
            .any(|file| file.items.iter().any(|item| matches!(item, Item::Testing(_)))));
        assert!(driver(&ast).is_none());
        assert!(!function_names(&ast)
            .iter()
            .any(|name| name.starts_with("__mfb_test_case_")));
    }

    #[test]
    fn test_mode_desugars_cases_and_appends_the_driver() {
        let mut ast = crate::testutil::project_from_src(SOURCE);
        let lowering =
            lower_testing_blocks(&mut ast, CompileMode::Test { coverage: false }, Path::new("/tmp"));
        assert_eq!(lowering.entry.as_deref(), Some(DRIVER_NAME));
        assert!(lowering.cov_slots.is_empty(), "no coverage without --coverage");

        // The two real cases became sequentially-numbered SUBs; the case-less group
        // produced none.
        let names = function_names(&ast);
        assert!(names.iter().any(|name| name == "__mfb_test_case_0"));
        assert!(names.iter().any(|name| name == "__mfb_test_case_1"));
        assert!(!names.iter().any(|name| name == "__mfb_test_case_2"));

        // The driver exists, returns Integer, and the sink file gained an `io`
        // import for its `io.print` calls.
        let driver = driver(&ast).expect("driver present");
        assert_eq!(driver.return_type.as_deref(), Some("Integer"));
        assert!(ast.files[0].imports.iter().any(|import| import.module == "io"));

        // No `TESTING` item survives the lowering.
        assert!(!ast
            .files
            .iter()
            .any(|file| file.items.iter().any(|item| matches!(item, Item::Testing(_)))));
    }

    #[test]
    fn coverage_mode_instruments_statements_and_adds_runtime_helpers() {
        let mut ast = crate::testutil::project_from_src(SOURCE);
        let lowering =
            lower_testing_blocks(&mut ast, CompileMode::Test { coverage: true }, Path::new("/tmp"));
        assert_eq!(lowering.entry.as_deref(), Some(DRIVER_NAME));
        // The instrumented case bodies each contribute at least one slot.
        assert!(
            !lowering.cov_slots.is_empty(),
            "coverage build must emit slots"
        );
        // The coverage runtime helpers and their imports were appended to the sink.
        let names = function_names(&ast);
        assert!(names.iter().any(|name| name == "__mfb_cov_hit"));
        assert!(names.iter().any(|name| name == "__mfb_cov_dump"));
        assert!(ast.files[0]
            .imports
            .iter()
            .any(|import| import.module == "collections"));
        assert!(ast.files[0].imports.iter().any(|import| import.module == "fs"));
    }

    #[test]
    fn group_has_cases_sees_through_nesting_and_rejects_the_empty_subtree() {
        let case = TestGroupMember::Case(TestCase {
            description: "c".to_string(),
            body: Vec::new(),
            line: 0,
        });
        let with_case = TestGroup {
            description: "outer".to_string(),
            members: vec![TestGroupMember::Group(TestGroup {
                description: "inner".to_string(),
                members: vec![case],
                line: 0,
            })],
            line: 0,
        };
        assert!(group_has_cases(&with_case));

        let empty = TestGroup {
            description: "outer".to_string(),
            members: vec![TestGroupMember::Group(TestGroup {
                description: "inner".to_string(),
                members: Vec::new(),
                line: 0,
            })],
            line: 0,
        };
        assert!(!group_has_cases(&empty));
    }

    #[test]
    fn lower_group_skips_a_caseless_group_and_indents_by_depth() {
        // A case-less group emits no steps at all.
        let mut steps = Vec::new();
        let mut generated = Vec::new();
        let mut index = 0usize;
        lower_group(
            TestGroup {
                description: "empty".to_string(),
                members: Vec::new(),
                line: 0,
            },
            0,
            &mut steps,
            &mut generated,
            &mut index,
        );
        assert!(steps.is_empty());
        assert!(generated.is_empty());
        assert_eq!(index, 0);

        // A group with a case emits a group header then a case step, and generates
        // one SUB whose name uses the running index.
        lower_group(
            TestGroup {
                description: "grp".to_string(),
                members: vec![TestGroupMember::Case(TestCase {
                    description: "c".to_string(),
                    body: Vec::new(),
                    line: 0,
                })],
                line: 0,
            },
            1,
            &mut steps,
            &mut generated,
            &mut index,
        );
        assert_eq!(steps.len(), 2);
        assert_eq!(generated.len(), 1);
        assert_eq!(generated[0].name, "__mfb_test_case_0");
        assert_eq!(index, 1);
        // The header is indented by `depth * 2`; the case by `(depth + 1) * 2`.
        match &steps[0] {
            desugar::DriverStep::Group { indent, description } => {
                assert_eq!(*indent, 2);
                assert_eq!(description, "grp");
            }
            _ => panic!("expected a group step"),
        }
        match &steps[1] {
            desugar::DriverStep::Case { indent, .. } => assert_eq!(*indent, 4),
            _ => panic!("expected a case step"),
        }
    }

    #[test]
    fn ensure_import_is_idempotent() {
        let mut file = AstFile {
            path: "main.mfb".to_string(),
            imports: Vec::new(),
            items: Vec::new(),
            internal: false,
        };
        ensure_import(&mut file, "io");
        ensure_import(&mut file, "io");
        assert_eq!(file.imports.len(), 1);
        assert_eq!(file.imports[0].module, "io");
        ensure_import(&mut file, "fs");
        assert_eq!(file.imports.len(), 2);
    }
}
