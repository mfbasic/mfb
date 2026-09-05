//! The program entry, across every backend and build mode.
//!
//! `engine/function/entry.rs` emits the thing that runs before `main`: the arena
//! bootstrap, the globals initializer, the args capture, the RNG seed, the
//! signal handlers, the stdin broadcast subscription, and — in an `-app` build —
//! a *second* entry, because the toolkit owns `_main` and the program runs on a
//! worker. Which of those appear is decided by a `ProgramEntrySpec` whose flags
//! are computed from the program: `capture_args` from whether it calls
//! `os::args`, `seed_rng` from whether it uses the RNG, and so on.
//!
//! Every test in the tree lowers a console program with none of those flags set,
//! so the entry has exactly one shape in coverage terms. These vary the program
//! and the build mode instead, and assert the property that makes an entry an
//! entry: **the symbol the code plan names as its entry point is a function the
//! same plan defines.** A plan that names an entry nothing defines does not link,
//! and nothing else in process checks it.

use crate::testutil::{code_for_src_mode, CodeTarget};

/// `(name, source)` — programs chosen for the entry FLAGS they set, not for what
/// they compute.
const PROGRAMS: &[(&str, &str)] = &[
    (
        "plain",
        "\
FUNC main() AS Integer
  RETURN 0
END FUNC
",
    ),
    (
        "globals",
        "\
IMPORT io

MUT total AS Integer = 7

FUNC main() AS Integer
  total = total + 1
  io::print(toString(total))
  RETURN 0
END FUNC
",
    ),
    (
        "args",
        "\
IMPORT io
IMPORT os

FUNC main() AS Integer
  LET argv AS List OF String = os::args()
  io::print(toString(len(argv)))
  RETURN 0
END FUNC
",
    ),
    (
        "rng",
        "\
IMPORT crypto
IMPORT io

FUNC main() AS Integer
  LET n AS Integer = crypto::randomInt(1, 6)
  io::print(toString(n))
  RETURN 0
END FUNC
",
    ),
    (
        "stdin",
        "\
IMPORT io

FUNC main() AS Integer
  LET line AS String = io::readLine()
  io::print(line)
  RETURN 0
END FUNC
",
    ),
];

/// Every backend and build mode names an entry symbol its own plan defines.
#[test]
fn every_entry_symbol_is_defined_by_the_plan_that_names_it() {
    let mut checked = 0;
    for (name, source) in PROGRAMS {
        for target in CodeTarget::ALL {
            let mut modes = vec![crate::target::NativeBuildMode::Console];
            modes.extend(target.app_mode());
            for mode in modes {
                let plan = code_for_src_mode(source, target, mode);
                let entry = plan.entry_symbol.clone().unwrap_or_else(|| {
                    panic!(
                        "{name} on {} ({}): a program with an entry point must name \
                         its entry symbol",
                        target.name(),
                        mode.as_str()
                    )
                });
                let defined = plan.functions.iter().any(|f| f.symbol == entry);
                assert!(
                    defined,
                    "{name} on {} ({}): the plan names `{entry}` as its entry point \
                     but defines no function with that symbol — the executable would \
                     not link",
                    target.name(),
                    mode.as_str()
                );
                checked += 1;
            }
        }
    }
    // 5 programs x 5 backends x (console + app, except console-only rv64).
    assert_eq!(
        checked, 45,
        "expected every program to be lowered for every backend and mode"
    );
}

/// An `-app` build emits a SECOND entry: the toolkit owns `_main`.
///
/// The program's own entry moves to its own symbol and is called by the worker
/// thread the toolkit spawns. A build that emitted only one entry would either
/// run the program on the UI thread — where every blocking call freezes the
/// window — or never run it at all.
#[test]
fn an_app_build_runs_the_program_on_a_worker_under_its_own_symbol() {
    for target in CodeTarget::ALL {
        let Some(mode) = target.app_mode() else {
            continue;
        };
        let source = PROGRAMS[0].1;
        let console = code_for_src_mode(source, target, crate::target::NativeBuildMode::Console);
        let app = code_for_src_mode(source, target, mode);
        assert!(
            app.functions.len() > console.functions.len(),
            "{}: an -app build must emit the toolkit bootstrap on top of the \
             program entry ({} functions vs {} in console)",
            target.name(),
            app.functions.len(),
            console.functions.len()
        );
        let worker = app
            .functions
            .iter()
            .filter(|f| {
                f.name.contains("macapp") || f.name.contains("gtkapp") || f.name.contains("winapp")
            })
            .count();
        assert!(
            worker > 0,
            "{}: an -app build must emit the toolkit's own entry alongside the \
             program's",
            target.name()
        );
    }
}
