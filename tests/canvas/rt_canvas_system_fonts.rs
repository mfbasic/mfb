//! `canvas::listSystemFonts` and `canvas::loadSystemFont` (plan-147).
//!
//! These run a real headless `--app` program against the host's installed fonts. That
//! is the point, not a convenience: what has to be true is a statement about the
//! operating system's answer — every name the list offers must load, and the filters
//! that keep CFF faces and variable-font named instances out must hold on a real font
//! collection — and no fixture can stand in for the machine's own fonts. The numbers
//! are therefore *properties* (sorted, unique, every name loads), never a count copied
//! from a run.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build a `--app` program, run it headless, and return its stdout lines.
fn run(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_app(&project, name);
    let out = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        out.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&out.status),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    lines
}

/// `ErrNotFound` — `mfb spec diagnostics error-codes` row `7-705-0004`.
const ERR_NOT_FOUND: &str = "77050004";

/// Prints every listed name, then loads each one and reports the tally.
const EVERY_FONT: &str = r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC tryLoad(n AS String) AS Boolean
  RES f AS canvas::Font = canvas::loadSystemFont(n) TRAP(e)
    io::print("failed: " & n & ": " & e.message)
    RETURN FALSE
  END TRAP
  RETURN TRUE
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  LET names AS List OF String = canvas::listSystemFonts()
  MUT loaded AS Integer = 0
  FOR EACH n IN names
    io::print("name: " & n)
    IF tryLoad(n) THEN
      loaded = loaded + 1
    END IF
  NEXT
  io::print("system fonts: " & toString(len(names)) & " loaded: " & toString(loaded))
END SUB
"#;

/// Every name the list offers loads; the list is sorted and holds no name twice.
///
/// Loading *every* face is what catches a filter that leaks: a CFF face, or a
/// variable font's named instance under a PostScript name no face in the file carries,
/// would list fine and then fail here.
#[cfg(target_os = "macos")]
#[test]
fn every_listed_system_font_loads() {
    let lines = run("canvas_system_fonts_every", EVERY_FONT);
    let names: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("name: "))
        .collect();
    let failures: Vec<&String> = lines.iter().filter(|l| l.starts_with("failed: ")).collect();
    let tally = lines
        .iter()
        .find(|l| l.starts_with("system fonts: "))
        .unwrap_or_else(|| panic!("no tally line in {lines:?}"));
    eprintln!("{tally}");

    assert!(!names.is_empty(), "the host lists no fonts at all");
    assert!(
        failures.is_empty(),
        "listed fonts that do not load: {failures:#?}"
    );
    assert_eq!(
        tally,
        &format!("system fonts: {n} loaded: {n}", n = names.len()),
    );
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted, "the list is not sorted and duplicate-free");
    assert!(
        names.contains(&"Helvetica") && names.contains(&"Helvetica Bold"),
        "Helvetica ships with every macOS and lives in a collection; it must be listed",
    );
    assert!(
        !names.iter().any(|n| n.starts_with('.')),
        "a private system face (a name starting with `.`) was listed",
    );
}

/// A listed name draws real text, and a name nothing carries is `ErrNotFound`.
#[cfg(target_os = "macos")]
#[test]
fn a_system_font_measures_text_and_an_unknown_name_is_not_found() {
    let lines = run(
        "canvas_system_fonts_load",
        r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES regular AS canvas::Font = canvas::loadSystemFont("Helvetica")
  RES bold AS canvas::Font = canvas::loadSystemFont("Helvetica Bold")
  LET r AS Float = canvas::measureText(regular, 100.0, "Hamburgefonts").width
  LET b AS Float = canvas::measureText(bold, 100.0, "Hamburgefonts").width
  IF r > 0.0 AND b > r THEN
    io::print("bold is wider")
  ELSE
    io::print("regular " & toString(r) & " bold " & toString(b))
  END IF
  RES none AS canvas::Font = canvas::loadSystemFont("No Such Font") TRAP(e)
    io::print("unknown: " & toString(e.code))
    EXIT SUB
  END TRAP
  io::print("unknown: loaded")
END SUB
"#,
    );
    assert_eq!(
        lines,
        vec![
            "bold is wider".to_string(),
            format!("unknown: {ERR_NOT_FOUND}"),
        ],
    );
}

/// `mfb build -app -target <target> -nplan` of a program that lists the system fonts,
/// returning the plan text.
fn cross_plan(name: &str, target: &str) -> String {
    let project = common::temp_project(
        name,
        r#"IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  io::print(toString(len(canvas::listSystemFonts())))
END SUB
"#,
    );
    let out = Command::new(common::mfb_exe())
        .args(["build", "-app", "-target", target, "-nplan"])
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        out.status.success(),
        "mfb build -app -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let plan = std::fs::read_to_string(project.join(format!("{name}.nplan"))).expect("read nplan");
    let _ = std::fs::remove_dir_all(&project);
    plan
}

/// On Linux, fontconfig is loaded at run time and never linked (plan-147-C): a canvas
/// program must start on a machine without it, where the font list is simply empty.
/// So the plan imports `dlopen`/`dlsym` and names no fontconfig library — a
/// `DT_NEEDED` on `libfontconfig.so.1` would make every canvas app fail to exec there.
#[test]
fn linux_reaches_fontconfig_through_dlopen_not_a_link() {
    for target in ["linux-x86_64", "linux-aarch64"] {
        let plan = cross_plan(
            &format!("canvas_sysfont_plan_{}", target.replace('-', "_")),
            target,
        );
        assert!(
            !plan.contains("fontconfig"),
            "{target}: the plan names fontconfig as a library to link",
        );
        for symbol in ["\"dlopen\"", "\"dlsym\""] {
            assert!(
                plan.contains(symbol),
                "{target}: the plan does not import {symbol}"
            );
        }
    }
}

/// On Windows the fonts come from DirectWrite (plan-147-D): the plan imports
/// `dwrite.dll`'s one flat export, `DWriteCreateFactory`, and nothing else from it —
/// every other DirectWrite call is a vtable call.
#[test]
fn windows_reaches_directwrite_through_its_factory() {
    let plan = cross_plan("canvas_sysfont_plan_windows", "windows-x86_64");
    assert!(
        plan.contains("dwrite.dll"),
        "the plan does not import dwrite.dll"
    );
    assert!(
        plan.contains("\"DWriteCreateFactory\""),
        "the plan does not import DWriteCreateFactory",
    );
}
