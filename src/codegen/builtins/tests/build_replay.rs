//! The committed build goldens, replayed through `mfb build` itself, in process.
//!
//! Every fixture under `tests/rt-behavior`, `tests/rt-error` and `tests/syntax`
//! carries a `golden/build.log` whose first section is the build that recorded
//! it — `$ mfb build -ast -ir <fixture>`, its diagnostics, `[exit N]` — and,
//! when that build succeeded, the `<project>.ast` and `<project>.ir` dumps it
//! wrote. `scripts/test-accept.sh` replays them with a spawned `mfb`, so nothing
//! they exercise counts toward this process's coverage (Finding F1): not the
//! manifest validation, `scope_privates`, the TESTING lowering, the resolver
//! over every file of a project, monomorphization, entry validation, the two
//! source checkers, the EXPORT checks, IR lowering, or a fixture's own
//! dependency packages.
//!
//! This runs `cli::build::build_project` with the golden's own flags on a copy
//! of each fixture (so the dumps land in a temporary directory, never in the
//! tree) with its diagnostics collected, and requires **the same rule codes in
//! the same order and the same exit status** as the golden's first section, and
//! the dumps it writes **byte-identical** (line endings aside) to the golden
//! ones. The dump comparison is what makes IR lowering a contract here rather
//! than a smoke test: a lowering change that alters what a program means shows
//! up as a different `.ir`.
//!
//! The list is explicit and its length asserted, for the reason the other
//! corpora give: a sweep that skipped whatever stopped matching would hollow out
//! silently. It is every such fixture but two, each left out because its golden
//! records what an in-process replay cannot reproduce by design:
//! `syntax/general/diagnostic_render_cap` (the terminal's rendering cap, which a
//! collecting test bypasses on purpose) and `syntax/security/pkg-01-tampered-signature`
//! (its verdict depends on the machine's pinned registry key, state outside the
//! fixture, and is reported through the package-verification report rather than
//! the diagnostic stream).
//! Symlinks in a fixture (`fs` sandbox fixtures point one at `/etc`) are runtime
//! data, not build input, and are not copied.

use std::path::{Path, PathBuf};

/// The corpus, one fixture path (under `tests/`) per line of
/// `build_replay_corpus.txt`; `#` lines are comments.
fn replay_corpus() -> Vec<&'static str> {
    include_str!("build_replay_corpus.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// The number of fixtures in `build_replay_corpus.txt`. A fixture that is added
/// or removed must be added or removed here too, deliberately.
const REPLAY_CORPUS_LEN: usize = 1677;

/// The golden's first section: the recorded build, up to its `[exit N]` line.
/// A run fixture's golden continues with the build it executes and the run.
fn first_section(log: &str) -> Vec<&str> {
    let mut section = Vec::new();
    for line in log.lines() {
        section.push(line);
        if line.starts_with("[exit ") {
            break;
        }
    }
    section
}

/// The rule NAMES a section records, in order — errors and warnings both.
fn golden_rules(section: &[&str]) -> Vec<String> {
    let mut rules = Vec::new();
    for line in section {
        for marker in ["error[", "warn["] {
            if let Some(at) = line.find(marker) {
                let head = line[at + marker.len()..].split(']').next().unwrap_or("");
                if let Some(name) = head.split_whitespace().nth(1) {
                    rules.push(name.to_string());
                }
            }
        }
    }
    rules
}

/// The section's recorded exit status.
fn golden_exit(section: &[&str]) -> Option<i32> {
    section
        .iter()
        .find_map(|line| line.strip_prefix("[exit ")?.strip_suffix(']')?.parse().ok())
}

/// The flags the section was recorded with: `$ mfb build <flags...> <path>`.
fn golden_flags(section: &[&str]) -> Vec<String> {
    let header = section.first().copied().unwrap_or("");
    let mut words: Vec<String> = header
        .split_whitespace()
        .skip(3)
        .map(str::to_string)
        .collect();
    words.pop();
    words
}

/// Copy a fixture without its recorded outputs or its symlinks.
fn copy_fixture(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create the fixture copy");
    for entry in std::fs::read_dir(from).expect("read the fixture") {
        let entry = entry.expect("fixture entry");
        if entry.file_type().expect("fixture entry type").is_symlink() {
            continue;
        }
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            if entry.file_name() == "golden" || entry.file_name() == "build" {
                continue;
            }
            copy_fixture(&source, &destination);
        } else {
            std::fs::copy(&source, &destination).expect("copy a fixture file");
        }
    }
}

/// A dump's text as compared: line endings normalized (a Windows checkout
/// carries CRLF), and on Windows the path separator too, on both sides alike.
fn comparable(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}

/// Replay one fixture; `None` when it reproduces its golden, else what differs.
fn replay(relative: &str) -> Option<String> {
    let fixture: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(relative);
    let golden = fixture.join("golden");
    let log = std::fs::read_to_string(golden.join("build.log"))
        .unwrap_or_else(|err| panic!("{relative}: read its golden: {err}"));
    let section = first_section(&log);
    let scratch = tempfile::tempdir().expect("temp dir");
    let copy = scratch
        .path()
        .join(fixture.file_name().expect("a fixture directory has a name"));
    copy_fixture(&fixture, &copy);
    let mut args = golden_flags(&section);
    args.push(copy.to_string_lossy().into_owned());
    let options = crate::cli::build::parse_build_options(args)
        .unwrap_or_else(|err| panic!("{relative}: its golden's flags do not parse: {err}"));
    let (result, collected) =
        crate::rules::collecting_diagnostics(|| crate::cli::build::build_project(&options));
    let got: Vec<String> = collected.into_iter().map(|(rule, _)| rule).collect();
    let exit = if result.is_ok() { 0 } else { 1 };
    let want = golden_rules(&section);
    let want_exit = golden_exit(&section);
    let mut wrong = Vec::new();
    if got != want || Some(exit) != want_exit {
        wrong.push(format!(
            "got {got:?} exit {exit}, golden {want:?} exit {want_exit:?}"
        ));
    }
    for entry in std::fs::read_dir(&golden).expect("read the golden directory") {
        let name = entry.expect("golden entry").file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !(name.ends_with(".ast") || name.ends_with(".ir")) {
            continue;
        }
        let expected = std::fs::read_to_string(golden.join(name.as_ref()))
            .unwrap_or_else(|err| panic!("{relative}: read golden {name}: {err}"));
        match std::fs::read_to_string(copy.join(name.as_ref())) {
            Ok(actual) if comparable(&actual) == comparable(&expected) => {}
            Ok(_) => wrong.push(format!("{name} differs from its golden")),
            Err(_) => wrong.push(format!("{name} was not written")),
        }
    }
    (!wrong.is_empty()).then(|| format!("{relative}: {}", wrong.join("; ")))
}

/// Replay every `SHARDS`-th fixture of the corpus, starting at `offset`, each
/// on a thread with the stack a release build's main thread gives the front
/// end.
fn replay_shard(offset: usize) {
    let wrong: Vec<String> = replay_corpus()
        .into_iter()
        .skip(offset)
        .step_by(SHARDS)
        .filter_map(|relative| {
            let relative = relative.to_string();
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || replay(&relative))
                .expect("spawn")
                .join()
                .expect("a build must not panic")
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} fixture(s) no longer reproduce their golden through `mfb build`:\n  {}",
        wrong.len(),
        wrong.join("\n  ")
    );
}

const SHARDS: usize = 24;

#[test]
fn the_replay_corpus_is_the_size_it_was_reviewed_at() {
    let corpus = replay_corpus();
    assert_eq!(corpus.len(), REPLAY_CORPUS_LEN);
    let mut sorted = corpus.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), corpus.len(), "a fixture is listed twice");
    // Every listed fixture exists, with the golden the replay reads.
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let missing: Vec<&str> = corpus
        .iter()
        .copied()
        .filter(|relative| {
            !tests
                .join(relative)
                .join("golden")
                .join("build.log")
                .is_file()
        })
        .collect();
    assert!(
        missing.is_empty(),
        "listed fixtures without a golden: {missing:?}"
    );
}

#[test]
fn replay_shard_00() {
    replay_shard(0);
}

#[test]
fn replay_shard_01() {
    replay_shard(1);
}

#[test]
fn replay_shard_02() {
    replay_shard(2);
}

#[test]
fn replay_shard_03() {
    replay_shard(3);
}

#[test]
fn replay_shard_04() {
    replay_shard(4);
}

#[test]
fn replay_shard_05() {
    replay_shard(5);
}

#[test]
fn replay_shard_06() {
    replay_shard(6);
}

#[test]
fn replay_shard_07() {
    replay_shard(7);
}

#[test]
fn replay_shard_08() {
    replay_shard(8);
}

#[test]
fn replay_shard_09() {
    replay_shard(9);
}

#[test]
fn replay_shard_10() {
    replay_shard(10);
}

#[test]
fn replay_shard_11() {
    replay_shard(11);
}

#[test]
fn replay_shard_12() {
    replay_shard(12);
}

#[test]
fn replay_shard_13() {
    replay_shard(13);
}

#[test]
fn replay_shard_14() {
    replay_shard(14);
}

#[test]
fn replay_shard_15() {
    replay_shard(15);
}

#[test]
fn replay_shard_16() {
    replay_shard(16);
}

#[test]
fn replay_shard_17() {
    replay_shard(17);
}

#[test]
fn replay_shard_18() {
    replay_shard(18);
}

#[test]
fn replay_shard_19() {
    replay_shard(19);
}

#[test]
fn replay_shard_20() {
    replay_shard(20);
}

#[test]
fn replay_shard_21() {
    replay_shard(21);
}

#[test]
fn replay_shard_22() {
    replay_shard(22);
}

#[test]
fn replay_shard_23() {
    replay_shard(23);
}
