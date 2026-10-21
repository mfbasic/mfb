//! The statement and value descriptions a failed lowering is attributed to
//! (`nir_op_context`, `nir_value_context`: "... while lowering assign x"), and the
//! name-read test that gates the in-place string self-append
//! (`nir_value_reads_local`).
//!
//! Both are exhaustive matches over `NirOp` / `NirValue`, so they cannot miss a
//! variant; what a test can pin is that each variant gets ITS description (a
//! `NirOp::Fail` described as "return" would send a developer to the wrong
//! statement) and that the read test agrees, over every node of real programs,
//! with an independent walk of the same tree.

use crate::codegen::engine::control::{nir_op_context, nir_value_context, nir_value_reads_local};
use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{fixture_src, nir_for_src, CodeTarget};
use std::collections::BTreeSet;

/// Fixtures that between them contain every statement and value kind: STATE
/// writes, closures and captures, unions, results, operator captures, `WITH`,
/// `FAIL`, `DO .. LOOP UNTIL`, `FOR EACH`, `TRAP`.
const FIXTURES: &[&str] = &[
    "inplace-decline-aliasing-rt",
    "inline-trap-positions-rt",
    "bug752-res-element-callbacks-rt",
    "loop-do-until-mut-integer",
    "flat-union",
    "scope-drop-free-union",
    "bug754-float-overflow-raises-inside-expression-rt",
    "flat-record-collection",
    "record-field",
    "call-function-value-rt",
    "control-flow-behavior",
    "json-behavior",
    "bug118_match_guard_helper",
    "collection-of-function-rt",
    "byref-capture-rt",
];

/// The two statements no fixture above ends up carrying after lowering.
const EXTRA: &str = "\
IMPORT io
IMPORT os

FUNC main() AS Integer
  FOR i = 1 TO 3
    IF i = 2 THEN
      CONTINUE FOR
    END IF
    io::print(toString(i))
  NEXT
  os::exit(0)
  RETURN 0
END FUNC
";

fn modules() -> Vec<NirModule> {
    let sources = FIXTURES
        .iter()
        .map(|name| fixture_src(name))
        .chain(std::iter::once(EXTRA.to_string()));
    sources
        .map(|source| {
            nir_for_src(&source, CodeTarget::LinuxX86_64, Console)
                .unwrap_or_else(|error| panic!("a source lowers to NIR: {error}"))
        })
        .collect()
}

/// Every description produced.
struct Describe {
    ops: BTreeSet<String>,
    values: BTreeSet<String>,
}

impl NirVisitor for Describe {
    fn visit_op(&mut self, op: &NirOp) {
        self.ops.insert(nir_op_context(op));
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        self.values.insert(nir_value_context(value));
        walk_value(self, value);
    }
}

/// `Local(name)` / `LocalRef` anywhere under a node, by an independent
/// traversal (the shared `walk_value` seam rather than `nir_value_reads_local`'s
/// own recursion).
struct Reads<'a> {
    name: &'a str,
    found: bool,
}

impl NirVisitor for Reads<'_> {
    fn visit_value(&mut self, value: &NirValue) {
        match value {
            NirValue::Local(local) | NirValue::LocalRef { name: local, .. }
                if local == self.name =>
            {
                self.found = true;
            }
            _ => walk_value(self, value),
        }
    }
}

/// The names of the locals a function reads.
struct LocalNames(BTreeSet<String>);

impl NirVisitor for LocalNames {
    fn visit_value(&mut self, value: &NirValue) {
        if let NirValue::Local(name) = value {
            self.0.insert(name.clone());
        }
        walk_value(self, value);
    }
}

/// At every value node, `nir_value_reads_local` against the independent walk,
/// for each of the function's local names.
struct Agree<'n> {
    names: &'n BTreeSet<String>,
    checked: usize,
    positives: usize,
}

impl NirVisitor for Agree<'_> {
    fn visit_value(&mut self, value: &NirValue) {
        for name in self.names {
            let mut reads = Reads { name, found: false };
            reads.visit_value(value);
            assert_eq!(
                nir_value_reads_local(value, name),
                reads.found,
                "disagreement on `{name}` in {}",
                nir_value_context(value)
            );
            self.checked += 1;
            self.positives += usize::from(reads.found);
        }
        walk_value(self, value);
    }
}

#[test]
fn every_statement_kind_is_described_as_itself() {
    let mut described = Describe {
        ops: BTreeSet::new(),
        values: BTreeSet::new(),
    };
    for module in modules() {
        for function in &module.functions {
            described.visit_ops(&function.body);
        }
    }
    for expected in [
        "bind ",
        "assign ",
        "store global ",
        "state assign ",
        "return",
        "exit loop",
        "continue loop",
        "fail",
        "exit program",
        "eval ",
        "if",
        "match",
        "while",
        "for ",
        "do until",
        "for each ",
        "trap",
    ] {
        assert!(
            described.ops.iter().any(|op| op.starts_with(expected)),
            "no statement was described as `{expected}` ({} descriptions)",
            described.ops.len()
        );
    }
    for op in &described.ops {
        assert!(
            !op.is_empty() && !op.contains("Nir"),
            "a description leaked an internal name: `{op}`"
        );
    }
    for expected in [
        "call ",
        "construct ",
        "member ",
        "local ",
        "local ref ",
        "function ",
        "closure ",
        "const ",
        "literal ",
        "operator ",
        "wrap ",
        "extract ",
        "result is ok",
        "result value",
        "result error",
        "checked ",
        "with update ",
        "capture ",
    ] {
        assert!(
            described
                .values
                .iter()
                .any(|value| value.starts_with(expected)),
            "no value was described as `{expected}` ({} descriptions)",
            described.values.len()
        );
    }
}

/// `nir_value_reads_local` agrees with an independent walk over every value node
/// of every function, for every local the function reads.
#[test]
fn the_read_test_agrees_with_an_independent_walk() {
    let (mut checked, mut positives) = (0usize, 0usize);
    for module in modules() {
        for function in &module.functions {
            let mut names = LocalNames(BTreeSet::new());
            names.visit_ops(&function.body);
            let mut agree = Agree {
                names: &names.0,
                checked: 0,
                positives: 0,
            };
            agree.visit_ops(&function.body);
            checked += agree.checked;
            positives += agree.positives;
        }
    }
    assert!(
        checked > 1000 && positives > 100,
        "the sweep must be large enough to mean something: {positives}/{checked}"
    );
}
