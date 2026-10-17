//! `strings::lineBreaks` against the Unicode 16.0.0 conformance file (plan-162-E).
//!
//! `third_party/unicode/LineBreakTest-16.0.0.txt` is the UAX #14 oracle: every line is a
//! sequence of scalars with `×` (no break) or `÷` (a break) at each position. One program
//! reads all of its cases from a file — one build, not one per case — and prints, per
//! case, the indices `lineBreaks` reports; this compares them with the `÷` positions.
//! Every case must agree: a line-breaking implementation that is right "almost
//! everywhere" is wrong exactly where LB9, LB25 and the quotation rules get hard.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// One conformance case: its scalars, and the positions (1..=len) the file breaks at.
struct Case {
    line: usize,
    scalars: Vec<u32>,
    breaks: Vec<usize>,
}

fn cases() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/third_party/unicode/LineBreakTest-16.0.0.txt"
    );
    let text = std::fs::read_to_string(path).expect("the vendored conformance file");
    let mut out = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let data = raw.split('#').next().unwrap_or("").trim();
        if data.is_empty() {
            continue;
        }
        let mut scalars = Vec::new();
        let mut breaks = Vec::new();
        for token in data.split_whitespace() {
            match token {
                "×" => {}
                "÷" => {
                    if !scalars.is_empty() {
                        breaks.push(scalars.len());
                    }
                }
                hex => scalars.push(u32::from_str_radix(hex, 16).expect("a hex scalar")),
            }
        }
        out.push(Case {
            line: number + 1,
            scalars,
            breaks,
        });
    }
    out
}

const PROGRAM: &str = r#"IMPORT collections
IMPORT io
IMPORT fs
IMPORT strings
IMPORT encoding

FUNC main() AS Integer
  LET text AS String = fs::readText("cases.txt")
  FOR EACH line IN strings::split(text, "\n")
    IF len(line) > 0 THEN
      MUT cps AS List OF Integer = []
      FOR EACH word IN strings::split(line, " ")
        cps = collections::append(cps, toInt(word))
      NEXT
      MUT out AS String = ""
      FOR EACH b IN strings::lineBreaks(encoding::utf32Decode(cps))
        IF len(out) > 0 THEN
          out = out & " "
        END IF
        out = out & toString(b.index)
      NEXT
      io::print(out)
    END IF
  NEXT
  RETURN 0
END FUNC
"#;

#[test]
fn line_break_conformance() {
    let cases = cases();
    assert_eq!(cases.len(), 16_672, "the 16.0.0 file's case count");
    let project = common::temp_project("rt_strings_line_breaks", PROGRAM);
    let data: Vec<String> = cases
        .iter()
        .map(|c| {
            c.scalars
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    std::fs::write(project.join("cases.txt"), data.join("\n") + "\n").expect("write the cases");
    let binary = common::build_project(&project);
    let run = Command::new(&binary)
        .current_dir(&project)
        .output()
        .expect("run the conformance program");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stderr)
    );
    let got: Vec<&str> = stdout.lines().collect();
    assert_eq!(got.len(), cases.len(), "one result line per case");
    let mut failures = Vec::new();
    for (case, line) in cases.iter().zip(&got) {
        let want = case
            .breaks
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if *line != want {
            failures.push(format!(
                "line {}: {:04X?}\n  want {want}\n  got  {line}",
                case.line, case.scalars
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        failures.is_empty(),
        "{} of {} conformance cases disagree; first 20:\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
