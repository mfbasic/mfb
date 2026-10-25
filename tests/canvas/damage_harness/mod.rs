//! The damage tests' shared harness: a headless `--debug` run that returns the dumped
//! frame and every stats line, and the readers of both. Included by `rt_canvas_damage.rs`
//! and `rt_canvas_damage_groups.rs`, each of which declares `common` beside it.

use super::common;
use std::process::Command;

pub(crate) const WIDTH: usize = 900;

/// Render headless and return the frame plus every stats line.
pub(crate) fn render(name: &str, source: &str, damage: bool) -> (Vec<u8>, Vec<String>) {
    render_env(name, source, damage, &[])
}

/// As `render`, plus extra environment variables.
pub(crate) fn render_env(
    name: &str,
    source: &str,
    damage: bool,
    env: &[(&str, &str)],
) -> (Vec<u8>, Vec<String>) {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_SYNC", "1");
    if damage {
        command.env("MFB_CANVAS_DAMAGE", "1");
    }
    command.envs(env.iter().copied());
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let pixels = std::fs::read(&frame).expect("canvas dump written");
    let lines: Vec<String> = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    (pixels, lines)
}

/// One pixel of a dumped frame, as RGBA.
pub(crate) fn pixel(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8, u8) {
    let i = (y * WIDTH + x) * 4;
    (frame[i], frame[i + 1], frame[i + 2], frame[i + 3])
}

/// A `key=value` field of the last stats line.
pub(crate) fn last(lines: &[String], key: &str) -> String {
    let line = lines
        .iter()
        .rev()
        .find(|l| l.contains(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no stats line carries `{key}=`:\n{}", lines.join("\n")));
    line.split(&format!("{key}="))
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .to_string()
}

pub(crate) fn number(lines: &[String], key: &str) -> i64 {
    last(lines, key)
        .parse()
        .unwrap_or_else(|e| panic!("`{key}` is not a number: {e}"))
}
