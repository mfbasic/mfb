//! bug-642: a call rule failing in a function body monomorph instantiated on a
//! user's behalf is reported once per instantiating call, at that call's file
//! and line, with the detail naming the member — never at the body's own line.

use super::*;
use crate::hir::{HirItem, InstantiationSite};

fn site(file: &str, line: usize) -> InstantiationSite {
    InstantiationSite {
        file: file.to_string(),
        line,
        callee: "pkg::twice".to_string(),
    }
}

#[test]
fn a_call_rule_in_an_instantiated_body_is_reported_at_each_site() {
    let mut hir = hir_from(
        "FUNC one(x AS Integer) AS Integer\n  RETURN x\nEND FUNC\n\
         FUNC twice() AS Integer\n  RETURN one(1, 2)\nEND FUNC\n\
         FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    );
    let mut found = false;
    for file in &mut hir.files {
        for item in &mut file.items {
            if let HirItem::Function(function) = item {
                if function.name == "twice" {
                    function.instantiated_at = vec![site("a.mfb", 3), site("b.mfb", 9)];
                    found = true;
                }
            }
        }
    }
    assert!(found, "the fixture declares `twice`");
    let reported: Vec<(String, std::path::PathBuf, usize, String)> =
        collect_diagnostics(Path::new("/proj"), &hir, &[], &[], &HashMap::new(), &[])
            .into_iter()
            .map(|d| (d.rule, d.path, d.line, d.detail))
            .collect();
    assert_eq!(reported.len(), 2, "{reported:#?}");
    for ((_, path, line, detail), (file, want_line)) in
        reported.iter().zip([("a.mfb", 3), ("b.mfb", 9)])
    {
        assert_eq!(path, &Path::new("/proj").join(file));
        assert_eq!(*line, want_line);
        assert!(
            detail.starts_with("In the body of `pkg::twice`, instantiated by this call: "),
            "{detail}"
        );
    }
}
