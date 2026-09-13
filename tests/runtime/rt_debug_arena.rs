//! plan-130-C: the `arena` section of the `--debug` report registers every arena.
//!
//! A `--debug` build keeps a process-global registry of arena states — the main
//! thread's, each `thread::start` worker's, and the canvas graphics thread's — and the
//! report lists them in registration order: `arena.count <n>`,
//! `arena.registry_overflow <n>`, and `arena.<i>.kind <main|worker|graphics>`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;

/// Build `source` with `--debug` and return the executable (the host libc's on Linux).
fn build_debug(name: &str, source: &str) -> PathBuf {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// The `arena.` lines of the (single, last) report block on `stderr`.
fn arena_lines(case: &str, stderr: &str) -> Vec<String> {
    let at = stderr
        .rfind("mfb.debug.begin ")
        .unwrap_or_else(|| panic!("{case}: no report block on stderr:\n{stderr}"));
    stderr[at..]
        .lines()
        .take_while(|line| *line != "mfb.debug.end 1")
        .filter(|line| line.starts_with("arena."))
        .map(str::to_string)
        .collect()
}

/// Three `thread::start` workers plus the main thread: four arenas, registered in
/// start order, none dropped.
#[test]
fn three_workers_register_four_arenas() {
    let source = concat!(
        "IMPORT io\n",
        "IMPORT thread\n",
        "\n",
        "ISOLATED FUNC work(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n",
        "  RETURN len(seed)\n",
        "END FUNC\n",
        "\n",
        "FUNC main AS Integer\n",
        "  LET a AS Thread OF String TO Integer = thread::start(work, \"a\")\n",
        "  LET b AS Thread OF String TO Integer = thread::start(work, \"bb\")\n",
        "  LET c AS Thread OF String TO Integer = thread::start(work, \"ccc\")\n",
        "  LET total AS Integer = thread::waitFor(a) + thread::waitFor(b) + thread::waitFor(c)\n",
        "  io::print(toString(total))\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let exe = build_debug("dbg_arena_workers", source);
    let output = Command::new(&exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "program failed:\n{stdout}\n{stderr}"
    );
    assert_eq!(stdout, "6\n", "the workers ran");
    assert_eq!(
        arena_lines("workers", &stderr),
        vec![
            "arena.count 4",
            "arena.registry_overflow 0",
            "arena.0.kind main",
            "arena.1.kind worker",
            "arena.2.kind worker",
            "arena.3.kind worker",
        ],
        "arena section:\n{stderr}"
    );
}

/// A canvas app that presents once starts the graphics thread, which registers its own
/// arena: the report lists the program's arena as `main` and the render thread's as
/// `graphics`. Headless, so it opens no window.
#[cfg(target_os = "macos")]
#[test]
fn the_canvas_graphics_thread_registers_a_graphics_arena() {
    let name = "dbg_arena_graphics";
    let source = concat!(
        "IMPORT app\n",
        "IMPORT canvas\n",
        "IMPORT color\n",
        "IMPORT io\n",
        "\n",
        "SUB main()\n",
        "  app::setMode(app::Mode.Canvas)\n",
        "  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(255, 0, 0))]\n",
        "  canvas::present([box])\n",
        "  io::print(\"done\")\n",
        "END SUB\n",
    );
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--app")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --app --debug");
    assert!(
        output.status.success(),
        "{name} failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(common::app_binary(&project, name))
        .env("MFB_MACAPP_HEADLESS", "1")
        .output()
        .expect("run the headless app");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "program failed:\n{stdout}\n{stderr}");
    assert!(stdout.contains("done"), "the program ran:\n{stdout}");
    let arenas = arena_lines("graphics", &stderr);
    assert_eq!(
        arenas,
        vec![
            "arena.count 2",
            "arena.registry_overflow 0",
            "arena.0.kind main",
            "arena.1.kind graphics",
        ],
        "arena section:\n{stderr}"
    );
}
