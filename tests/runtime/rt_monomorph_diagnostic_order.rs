//! Monomorphization lowered the program's functions by iterating a `HashMap`,
//! whose order changes from one process to the next. It reports diagnostics as
//! it lowers, so a program with a monomorph-stage error in several functions
//! printed them in a different order on every build — measured on the parent
//! with four `TYPE_OVERLOAD_AMBIGUOUS` calls, eight builds gave seven distinct
//! orders. Found while fixing bug-822. Functions are now lowered in source
//! order, so the diagnostics follow it and are identical build to build.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::process::Command;

const SOURCE: &str =
    "FUNC encode(value AS String) AS List OF Byte\n  RETURN [toByte(65)]\nEND FUNC\n\
FUNC encode(value AS String) AS List OF Integer\n  RETURN [1]\nEND FUNC\n\
FUNC alpha() AS Integer\n  LET x = encode(\"a\")\n  RETURN 0\nEND FUNC\n\
FUNC beta() AS Integer\n  LET x = encode(\"b\")\n  RETURN 0\nEND FUNC\n\
FUNC gamma() AS Integer\n  LET x = encode(\"c\")\n  RETURN 0\nEND FUNC\n\
FUNC main AS Integer\n  LET x = encode(\"d\")\n  RETURN 0\nEND FUNC\n";

#[test]
fn monomorph_diagnostics_follow_source_order_on_every_build() {
    let root = std::env::temp_dir().join(format!("mfb_monomorph_order_{}", common::unique_nonce()));
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"order\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\"}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src").join("main.mfb"), SOURCE).expect("write source");
    for attempt in 0..8 {
        let out = Command::new(common::mfb_exe())
            .arg("build")
            .arg(&root)
            .output()
            .expect("run mfb build");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !out.status.success(),
            "build unexpectedly succeeded:\n{text}"
        );
        let lines = text
            .lines()
            .filter(|line| line.contains("error[2-203-0101 TYPE_OVERLOAD_AMBIGUOUS]"))
            .filter_map(|line| line.split("main.mfb:").nth(1))
            .filter_map(|rest| rest.split(' ').next())
            .collect::<Vec<_>>();
        assert_eq!(lines, ["8", "12", "16", "20"], "attempt {attempt}:\n{text}");
    }
}
