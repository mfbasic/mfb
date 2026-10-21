//! plan-176-E: `cli_dbg`'s thread, `show`, watchpoint, breakpoint, report and
//! run-to-end tests. The native-debugger support lives in
//! `session_launch_and_list.rs`, re-exported by the root, `cli_dbg.rs`.

use super::*;

/// The system function the program's entry ends it with, by its symbol:
/// `_exit` on both formats, which is C's `exit` on Mach-O and C's `_exit`
/// on ELF.
const EXIT_SYMBOL: &str = "_exit";

/// That function as a stop line names it: `(<C name>) <symbol>`, the symbol
/// alone where the format spells the C name as the symbol.
fn exit_stop() -> String {
    if exit_name() == EXIT_SYMBOL {
        EXIT_SYMBOL.to_string()
    } else {
        format!("({}) {EXIT_SYMBOL}", exit_name())
    }
}

/// That function's C name, as listings write it.
fn exit_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "_exit"
    } else {
        "exit"
    }
}

/// The file a frame or listing names the C library by.
fn c_library() -> &'static str {
    if cfg!(target_os = "macos") {
        "libsystem_c.dylib"
    } else if host_libc_flavor() == "musl" {
        if cfg!(target_arch = "riscv64") {
            "ld-musl-riscv64.so.1"
        } else {
            "ld-musl-aarch64.so.1"
        }
    } else {
        "libc.so.6"
    }
}

/// A `--debug` build of a program that prints `[<value>]` for the
/// environment variable `MFB_DBG_GREETING`, or `[unset]`.
fn greeting_program() -> PathBuf {
    let project = std::env::temp_dir().join(format!("mfb_dbg_env_{}", unique_nonce()));
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("project.json"),
        r#"{
  "name": "greeting",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main",
  "targets": ["native"]
}
"#,
    )
    .unwrap();
    std::fs::write(
        project.join("src").join("main.mfb"),
        "IMPORT io\nIMPORT os\n\nFUNC main() AS Nothing\n  \
             io::print(\"[\" & os::getEnvOr(\"MFB_DBG_GREETING\", \"unset\") & \"]\")\n\
             END FUNC\n",
    )
    .unwrap();
    build_debug(&project, "greeting")
}

const THREAD_COLUMNS: &[&str] = &["ID", "Name", "PC", "Task"];

const ARENA_COLUMNS: &[&str] = &["ID", "Thread", "Address", "Size (KB)"];

const BREAKPOINT_COLUMNS: &[&str] = &["Address", "Count", "Symbol", "Written name"];

/// A launched program has one thread, the one that will run `main`, and no
/// arenas until its runtime starts; `step` runs it an instruction at a time.
#[test]
fn a_launched_program_lists_its_one_thread_and_steps() {
    let mfb = signed_mfb();
    let project = temp_project("dbg_steps", TICKER);
    let binary = build_debug(&project, "dbg_steps");
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap()]);
    session.until_prompt();

    session.send("list thread\n");
    let threads = table_rows(&session.until_prompt(), THREAD_COLUMNS);
    assert_eq!(threads.len(), 1, "{threads:?}");
    let cells = &threads[0];
    assert_eq!(cells[0], "0");
    assert!(is_address(&cells[2]), "{threads:?}");
    assert_eq!(cells[3], "MFB Main");

    session.send("list arena\n");
    assert_eq!(
        body(&session.until_prompt()),
        ["no arenas yet: the program's runtime has not started"]
    );

    session.send("step\n");
    let first = body(&session.until_prompt());
    assert!(
        first.len() == 1 && first[0].starts_with("Stepped 1 instruction; thread 0 is at 0x"),
        "{first:?}"
    );
    session.send("step 3\n");
    let more = body(&session.until_prompt());
    assert!(
        more.len() == 1 && more[0].starts_with("Stepped 3 instructions; thread 0 is at 0x"),
        "{more:?}"
    );
    // Four instructions of the loader moved the program counter.
    let pc = |line: &str| {
        line.split("is at ")
            .nth(1)
            .unwrap()
            .get(..18)
            .unwrap()
            .to_string()
    };
    assert_ne!(pc(&first[0]), pc(&more[0]));

    session.send("s 0\n");
    let still = body(&session.until_prompt());
    assert_eq!(still, [format!("Thread 0 is at {}.", pc(&more[0]))]);

    session.send("step -1\nlist reg 1 gpr\nq\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// Attached to a program running a round on a worker: the initial thread
/// runs `main`, the worker is a user thread, and each has its arena.
#[test]
fn an_attached_threaded_program_lists_its_threads_and_arenas() {
    let mfb = signed_mfb();
    let binary = debug_example();
    // One round on one worker, sized to outlast the test (minutes): the
    // `--debug` arena registry never reuses a slot, so a run of many short
    // rounds fills its 1024 slots in a few seconds (`mfb spec tooling
    // debug-report`, `arena.registry_overflow`), and an attach after that sees
    // a worker whose arena is not listed.
    let mut example = Command::new(&binary)
        .args(["1", "200000000", "1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the example");
    if !attach_allowed(&mfb, example.id()) {
        example.kill().unwrap();
        example.wait().unwrap();
        return;
    }
    let task = |cells: &Vec<String>| cells[3].clone();
    // Do not guess how long the process needs to start its worker. Under the
    // full parallel suite a fixed sleep can expire while the child is still
    // in startup, and the thread table records a worker before the worker
    // registers its arena. Attach/list/detach until the debugger reports the
    // worker thread and an arena it owns.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let (mut session, threads, workers, arenas) = loop {
        let mut session = Session::start(&mfb, &[&example.id().to_string()]);
        session.until_prompt();
        session.send("list thread\n");
        let threads = table_rows(&session.until_prompt(), THREAD_COLUMNS);
        let workers = threads
            .iter()
            .filter(|cells| task(cells) == "User Thread")
            .map(|cells| cells[0].clone())
            .collect::<Vec<_>>();
        session.send("list arena\n");
        let arenas = table_rows(&session.until_prompt(), ARENA_COLUMNS);
        if workers.len() == 1 && arenas.iter().any(|row| row.get(1) == Some(&workers[0])) {
            break (session, threads, workers, arenas);
        }
        session.send("q\n");
        let (code, _) = session.finish();
        assert_eq!(code, Some(0));
        assert!(
            std::time::Instant::now() < deadline,
            "the example never published a worker thread with its arena: \
             {threads:?} {arenas:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(task(&threads[0]), "MFB Main", "{threads:?}");
    assert_eq!(workers.len(), 1, "{threads:?}");

    let rows = arenas.clone();
    assert!(
        rows.iter().all(|row| row.len() == 4
            && row[0].parse::<usize>().is_ok()
            && is_address(&row[2])
            && row[3].parse::<u64>().is_ok()),
        "{arenas:?}"
    );
    // The main arena is slot 0 and belongs to thread 0; the worker's arena
    // belongs to the worker.
    assert_eq!(
        rows[0][..2],
        ["0".to_string(), "0".to_string()],
        "{arenas:?}"
    );
    assert!(rows.iter().any(|row| row[1] == workers[0]), "{arenas:?}");

    // The main arena's state and blocks, and a dump of its oldest block.
    session.send("show arena\n");
    let shown = body(&session.until_prompt());
    assert_eq!(shown[0], "Arena State:", "{shown:?}");
    assert!(
        shown.iter().any(|line| line == "Arena Block (0):"),
        "{shown:?}"
    );
    assert!(
        shown
            .iter()
            .any(|line| line.starts_with("  quickBin[128]     ")),
        "{shown:?}"
    );
    session.send("show arena 0 0 0 32\n");
    let dump = body(&session.until_prompt());
    assert_eq!(dump.len(), 3, "{dump:?}");
    assert_eq!(dump[0], "Arena Block (0,0):");
    assert!(dump[1].starts_with("0x0000_0000 ") && dump[2].starts_with("0x0000_0010 "));

    // The worker is running MFBASIC code, so it steps.
    session.send(&format!("step 2 {}\n", workers[0]));
    let stepped = body(&session.until_prompt());
    assert!(
        stepped.len() == 1
            && stepped[0].starts_with(&format!(
                "Stepped 2 instructions; thread {} is at 0x",
                workers[0]
            )),
        "{stepped:?}"
    );
    // Every thread answers a step-all, each on its own line.
    session.send("sa 2\n");
    let all = body(&session.until_prompt());
    assert_eq!(all.len(), threads.len(), "{all:?}");
    for (id, line) in all.iter().enumerate() {
        assert!(
            line.starts_with(&format!("Thread {id}: stepped "))
                || line.starts_with(&format!("Thread {id}: stopped after ")),
            "{all:?}"
        );
    }

    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
    example.kill().unwrap();
    example.wait().unwrap();
}

/// An app's initial thread runs the window system and `main` runs on
/// another thread.
/// An AppKit app; a Linux GTK app needs a display and its AppImage
/// unpacked, so it is checked by hand (`mfb man debug start`).
#[cfg(target_os = "macos")]
#[test]
fn an_app_s_threads_are_the_app_and_mfb_main() {
    let mfb = signed_mfb();
    let project = temp_project("dbg_app", TICKER);
    let output = Command::new(mfb_exe())
        .args(["build", "-q", "--app", "--debug", "-O0"])
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let binary = project.join("build/dbg_app.app/Contents/MacOS/dbg_app");
    let mut app = Command::new(&binary)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the app");
    std::thread::sleep(Duration::from_millis(2000));
    let mut session = Session::start(&mfb, &[&app.id().to_string()]);
    session.until_prompt();
    session.send("list thread\n");
    let threads = table_rows(&session.until_prompt(), THREAD_COLUMNS);
    let task = |cells: &Vec<String>| cells[3].clone();
    assert_eq!(task(&threads[0]), "App", "{threads:?}");
    assert_eq!(
        threads
            .iter()
            .filter(|line| task(line) == "MFB Main")
            .count(),
        1,
        "{threads:?}"
    );
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
    app.kill().unwrap();
    app.wait().unwrap();
}

/// The String `formatLine` returns, read from the arena by `show type`
/// at the address `finish` reports in `x1`.
#[test]
fn show_type_reads_a_returned_string_from_the_arena() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();
    session.send("bp formatLine\n");
    session.until_prompt();
    session.send("run\n");
    session.until_prompt();
    session.send("finish\n");
    let returned = body(&session.until_prompt());
    assert!(
        returned.len() == 1 && returned[0].contains(" returned from formatLine "),
        "{returned:?}"
    );
    session.send(&format!("show type string {}\n", cpu::ARG1));
    assert_eq!(
        body(&session.until_prompt()),
        ["Length: 18 Value: \"round 1: 37 primes\""]
    );
    session.send(&format!("show type int {}\n", cpu::ARG1));
    assert_eq!(
        body(&session.until_prompt()),
        ["Value: 0x0000000000000012 (18)"]
    );
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// `env` changes the environment the program gets when it restarts, and
/// the running program keeps its own.
#[test]
fn env_gives_a_restarted_program_its_changes() {
    let mfb = signed_mfb();
    let binary = greeting_program();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap()]);
    session.until_prompt();
    session.send("env s MFB_DBG_GREETING hello,  world\n");
    assert_eq!(
        body(&session.until_prompt()),
        ["Set MFB_DBG_GREETING=hello,  world; the program gets it when it restarts."]
    );
    session.send("restart\n");
    session.until_prompt();
    session.send("run\n");
    let ran = body(&session.until_prompt());
    assert_eq!(
        ran.first().map(String::as_str),
        Some("[hello,  world]"),
        "{ran:?}"
    );
    session.send("env r MFB_DBG_GREETING\n");
    session.until_prompt();
    session.send("list env\n");
    // The running program has it; the next restart does not.
    let listed = session.until_prompt();
    let row = listed
        .lines()
        .find(|line| line.starts_with("MFB_DBG_GREETING "))
        .unwrap_or_else(|| panic!("no MFB_DBG_GREETING row in {listed}"))
        .split(" | ")
        .map(str::trim_end)
        .collect::<Vec<_>>();
    assert_eq!(row, ["MFB_DBG_GREETING", "hello,  world", ""], "{listed}");
    assert!(
        listed.contains("Name ")
            && listed.contains(" | Current ")
            && listed.contains(" | After restart"),
        "{listed}"
    );
    session.send("restart\n");
    session.until_prompt();
    session.send("run\n");
    let ran = body(&session.until_prompt());
    assert_eq!(ran.first().map(String::as_str), Some("[unset]"), "{ran:?}");
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// A write by the instruction under the breakpoint a thread is paused on
/// is seen: `countPrimes` saves `lr` with its second instruction, and a
/// breakpoint there, then `run`, stops for the write before anything else.
// riscv64 Linux has no watchpoints: `watchpoints_are_refused_on_riscv64`.
#[cfg(not(target_arch = "riscv64"))]
#[test]
fn a_watched_write_under_the_breakpoint_a_thread_is_paused_on_is_seen() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();
    session.send("bp countPrimes\n");
    session.until_prompt();
    session.send("run\n");
    session.until_prompt();
    session.send("dis 1\n");
    let first = body(&session.until_prompt());
    let frame = first[1]
        .split("sub    sp, sp, #")
        .nth(1)
        .unwrap_or_else(|| panic!("no frame in {first:?}"));
    session.send("list reg lr\n");
    let lr = table_rows(&session.until_prompt(), REGISTER_COLUMNS)[0][1].clone();
    session.send(&format!("wp sp-{frame}\n"));
    session.until_prompt();
    session.send("bp countPrimes+4\n");
    session.until_prompt();
    session.send("bp r countPrimes\n");
    session.until_prompt();
    session.send("run\n");
    let stopped = body(&session.until_prompt());
    assert!(
        stopped.len() == 1
            && stopped[0].starts_with("--[BP] T: 0 P: 0x")
            && stopped[0].ends_with(" F: (countPrimes) _mfb_fn_countPrimes+0x4"),
        "{stopped:?}"
    );
    session.send("run\n");
    let changed = body(&session.until_prompt());
    assert!(
        changed.len() == 1
            && changed[0].starts_with("--[WP] T: 0 P: 0x")
            && changed[0].contains(" F: (countPrimes) _mfb_fn_countPrimes+0x4 ")
            && changed[0].ends_with(&format!(
                " -> {lr} ({})",
                i64::from_str_radix(&lr[2..], 16).unwrap()
            )),
        "{changed:?} {lr}"
    );
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// A watchpoint stops the program when a thread changes the watched
/// bytes. `countPrimes` opens its frame with `sub sp, sp, #<n>` and then
/// saves `lr` at the new `sp`: a watchpoint at `sp - <n>`, set at the
/// breakpoint on its first instruction, sees the return address arrive.
// riscv64 Linux has no watchpoints: `watchpoints_are_refused_on_riscv64`.
#[cfg(not(target_arch = "riscv64"))]
#[test]
fn a_watchpoint_stops_when_a_thread_changes_the_watched_bytes() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();
    session.send("bp countPrimes\n");
    session.until_prompt();
    session.send("run\n");
    session.until_prompt();
    session.send("dis 1\n");
    let first = body(&session.until_prompt());
    let frame = first[1]
        .split("sub    sp, sp, #")
        .nth(1)
        .unwrap_or_else(|| panic!("no frame in {first:?}"));
    session.send("list reg lr\n");
    let lr = table_rows(&session.until_prompt(), REGISTER_COLUMNS)[0][1].clone();
    session.send(&format!("wp sp-{frame}\n"));
    let set = body(&session.until_prompt());
    assert!(
        set.len() == 1
            && set[0].starts_with("Set a watchpoint at 0x")
            && set[0].contains(" (8 bytes): 0x"),
        "{set:?}"
    );
    let address = set[0]
        .strip_prefix("Set a watchpoint at ")
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .to_string();
    session.send("bp r countPrimes\n");
    session.until_prompt();
    session.send("run\n");
    let changed = body(&session.until_prompt());
    assert!(
        changed.len() == 1
            && changed[0].starts_with("--[WP] T: 0 P: 0x")
            && changed[0].contains(" F: (countPrimes) _mfb_fn_countPrimes+0x4 ")
            && changed[0].contains(&format!(" M: {address}: 0x"))
            && changed[0].ends_with(&format!(
                " -> {lr} ({})",
                i64::from_str_radix(&lr[2..], 16).unwrap()
            )),
        "{changed:?} {lr}"
    );
    session.send("list watchpoint\n");
    // The one stop it made is counted.
    let listed = table_rows(
        &session.until_prompt(),
        &["Address", "Count", "Size", "Value"],
    );
    assert!(
        listed.len() == 1
            && listed[0][..3] == [address.to_string(), "1".to_string(), "8".to_string()]
            && listed[0][3].starts_with(&lr),
        "{listed:?}"
    );
    session.send(&format!("wp r {address}\n"));
    assert_eq!(
        body(&session.until_prompt()),
        [format!("Removed the watchpoint at {address} (8 bytes).")]
    );
    session.send("run\n");
    assert_eq!(
        body(&session.until_prompt()).last().map(String::as_str),
        Some(EXITED)
    );
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// A breakpoint with commands runs them at each stop and lets the program
/// run on, to its end.
#[test]
fn a_breakpoint_with_commands_runs_them_and_runs_on() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();
    let arg1 = cpu::ARG1;
    session.send(&format!("bp countPrimes list reg {arg1}; bt 1; run\n"));
    let set = body(&session.until_prompt());
    assert!(
        set.len() == 1
            && set[0].ends_with(&format!(
                " in countPrimes. Each stop runs 'list reg {arg1}; bt 1; run'."
            )),
        "{set:?}"
    );
    session.send("run\n");
    let run = body(&session.until_prompt());
    let stops = run
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            line.starts_with("--[BP] T: 0 P: 0x")
                && line.ends_with(" F: (countPrimes) _mfb_fn_countPrimes")
        })
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    assert_eq!(stops.len(), 2, "{run:?}");
    for at in stops {
        assert_eq!(
            run[at + 1],
            "Register | Hex                | Decimal",
            "{run:?}"
        );
        assert_eq!(
            run[at + 2],
            format!("{arg1:<8} | 0x00000000000001f4 | (500)"),
            "{run:?}"
        );
        assert_eq!(run[at + 3], "* thread #0, task = 'MFB Main'", "{run:?}");
        assert!(
            run[at + 4].ends_with(&format!(" {}`countPrimes", image())),
            "{run:?}"
        );
    }
    assert_eq!(run.last().map(String::as_str), Some(EXITED));
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// A fault is no MFBASIC error: `show error` names the fault the thread
/// stopped on, until the thread is moved or runs again.
#[test]
fn show_error_names_the_fault_a_thread_stopped_on() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();
    session.send("bp isPrime\n");
    session.until_prompt();
    session.send("run\n");
    session.until_prompt();
    session.send("show error\n");
    assert_eq!(
        body(&session.until_prompt()),
        ["Thread 0 has no error in flight."]
    );
    // With its stack pointer at 16, the function's first store faults.
    session.send("j pc sp=16\n");
    session.until_prompt();
    session.send("run\n");
    let stop = body(&session.until_prompt());
    let what = stop
        .last()
        .filter(|line| line.starts_with("--[XC] T: 0 P: 0x"))
        .and_then(|line| line.split_once(" X: "))
        .map(|(_, what)| what.to_string())
        .unwrap_or_else(|| panic!("no fault in {stop:?}"));
    assert!(what.starts_with("a bad memory access at 0x"), "{stop:?}");
    session.send("show error\n");
    assert_eq!(
        body(&session.until_prompt()),
        [format!(
            "Thread 0 has no error in flight: it stopped on {what}, which is a fault, not \
                 an error a TRAP can catch. Use bt and list reg to see where."
        )]
    );
    session.send("j pc\n");
    session.until_prompt();
    session.send("show error\n");
    assert_eq!(
        body(&session.until_prompt()),
        ["Thread 0 has no error in flight."]
    );
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// Breakpoints by written name and by a system function's symbol (pending
/// until the libraries load), `run` to each, then to the end.
/// `bpi` stops in `countPrimes` only for the second round's range (its
/// first argument, in `x0`), and `j` makes that call return at once with
/// a count of 7, which the round then prints.
#[test]
fn a_tested_breakpoint_stops_once_and_a_jump_returns_early() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();
    let (arg0, arg1, link) = (cpu::ARG0, cpu::ARG1, cpu::LINK);
    session.send(&format!("bpi {arg0} eq 824084 at countPrimes\n"));
    let set = body(&session.until_prompt());
    assert!(
        set.len() == 1
            && set[0].ends_with(&format!(
                " in countPrimes if {arg0} eq 0x00000000000c9314 (824084)."
            )),
        "{set:?}"
    );
    session.send("bpi [0x10] in at countPrimes bt 1\n");
    session.until_prompt();
    session.send("bpi r [0x10] in at countPrimes\n");
    session.until_prompt();
    session.send("list breakpoint\n");
    let listed = table_rows(
        &session.until_prompt(),
        &["Address", "Count", "Symbol", "Written name", "If"],
    );
    assert!(
        listed.len() == 1
            && listed[0][1..]
                == [
                    "0".to_string(),
                    "_mfb_fn_countPrimes".to_string(),
                    "countPrimes".to_string(),
                    format!("{arg0} eq 0x00000000000c9314 (824084)")
                ],
        "{listed:?}"
    );
    // The first round's call runs on unseen; the second's stops.
    session.send("run\n");
    let run = body(&session.until_prompt());
    assert_eq!(run.len(), 2, "{run:?}");
    assert_eq!(run[0], "round 1: 37 primes");
    assert!(
        run[1].starts_with("--[BP] T: 0 P: 0x")
            && run[1].ends_with(" F: (countPrimes) _mfb_fn_countPrimes"),
        "{run:?}"
    );
    session.send(&format!("list reg {arg0}\n"));
    assert_eq!(
        table_rows(&session.until_prompt(), REGISTER_COLUMNS),
        [[arg0, "0x00000000000c9314", "(824084)"]]
    );
    // At its first instruction the function has not touched its frame, so
    // going to the link register returns from it: no error (the first result
    // register) and a count of 7 (the second).
    session.send(&format!("j {link} {arg0}=0 {arg1}=7\n"));
    let moved = body(&session.until_prompt());
    assert!(
        moved.len() == 1
            && moved[0].starts_with("Moved thread 0 to 0x")
            && moved[0].contains(" in runRound+0x")
            && moved[0].ends_with(&format!(
                "; set {arg0} = 0x0000000000000000 (0), {arg1} = 0x0000000000000007 (7)."
            )),
        "{moved:?}"
    );
    session.send(&format!("list reg {arg0} {arg1}\n"));
    assert_eq!(
        table_rows(&session.until_prompt(), REGISTER_COLUMNS),
        [
            [arg0, "0x0000000000000000", "(0)"],
            [arg1, "0x0000000000000007", "(7)"]
        ]
    );
    session.send("run\n");
    assert_eq!(
        body(&session.until_prompt()),
        ["round 2: 7 primes", "total: 44 primes", EXITED]
    );
    session.send("q\n");
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
}

/// The report a `--debug` build writes as it exits never reaches stderr:
/// the debugger keeps it, and `show report` prints it.
#[test]
fn show_report_prints_the_report_the_program_wrote_as_it_exited() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();

    session.send("show report\n");
    assert_eq!(
        body(&session.until_prompt()),
        ["No performance and memory report yet: the program writes it as it exits."]
    );
    session.send("run\n");
    let end = body(&session.until_prompt());
    assert_eq!(end.last().map(String::as_str), Some(EXITED), "{end:?}");
    assert!(
        !end.iter().any(|line| line.starts_with("mfb.debug.")),
        "{end:?}"
    );
    session.send("show report\n");
    let report = body(&session.until_prompt());
    assert!(
        report
            .first()
            .is_some_and(|line| line == "mfb.debug.begin 1")
            && report.last().is_some_and(|line| line == "mfb.debug.end 1")
            && report.iter().any(|line| line == "arena.0.kind main"),
        "{report:?}"
    );
    // A restart starts over: the next run's report is not written yet.
    session.send("restart\n");
    session.until_prompt();
    session.send("show report\n");
    assert_eq!(
        body(&session.until_prompt()),
        ["No performance and memory report yet: the program writes it as it exits."]
    );
    session.send("q\n");
    let mut stderr = session.child.stderr.take().unwrap();
    let (code, _) = session.finish();
    assert_eq!(code, Some(0));
    let mut errors = String::new();
    stderr.read_to_string(&mut errors).unwrap();
    assert!(!errors.contains("mfb.debug."), "{errors}");
}

#[test]
fn run_stops_at_breakpoints_and_at_the_end() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "2", "500"]);
    session.until_prompt();

    session.send("bp isPrime\n");
    let set = body(&session.until_prompt());
    assert!(
        set.len() == 1
            && set[0].starts_with("Set a breakpoint at 0x")
            && set[0].ends_with(" in isPrime."),
        "{set:?}"
    );
    session.send(&format!("bp {EXIT_SYMBOL}\n"));
    assert_eq!(
        body(&session.until_prompt()),
        [format!(
            "Set a pending breakpoint on {EXIT_SYMBOL}: it is placed when the program has \
                 loaded its libraries."
        )]
    );

    session.send("run\n");
    let first = body(&session.until_prompt());
    assert!(
        first.len() == 2
            && first[0].starts_with(&format!(
                "Placed the pending breakpoint on {EXIT_SYMBOL} at 0x"
            ))
            && first[1].starts_with("--[BP] T: 0 P: 0x")
            && first[1].ends_with(" F: (isPrime) _mfb_fn_isPrime"),
        "{first:?}"
    );

    // Stopped in isPrime: the current function, a few instructions, and a
    // system function by name.
    session.send("dis\n");
    let function = body(&session.until_prompt());
    assert_eq!(function[0], format!("{}`isPrime:", image()), "{function:?}");
    assert!(function[1].starts_with("->  0x"), "{function:?}");
    // The prologue's frame allocation; its size is the compiler's to choose.
    assert!(function[1].contains(cpu::PROLOGUE), "{function:?}");
    assert!(function.last().unwrap().ends_with("ret") || function.len() > 10);
    session.send("dis 3\n");
    let three = body(&session.until_prompt());
    assert_eq!(three.len(), 4, "{three:?}");
    session.send(&format!("dis {}\n", exit_name()));
    let exit = body(&session.until_prompt());
    assert_eq!(
        exit[0],
        format!("{}`{}:", c_library(), exit_name()),
        "{exit:?}"
    );

    // The calls that led to isPrime, innermost first, ending at the
    // program's entry; `bt 2` keeps the innermost two.
    session.send("bt\n");
    let frames = body(&session.until_prompt());
    assert_eq!(frames[0], "* thread #0, task = 'MFB Main'", "{frames:?}");
    assert!(frames[1].starts_with("  * frame #0: 0x"), "{frames:?}");
    assert!(
        frames[1].ends_with(&format!(" {}`isPrime", image())),
        "{frames:?}"
    );
    for (line, function) in
        frames[2..]
            .iter()
            .zip(["countPrimes", "runRound", "runRounds", "main", "_main"])
    {
        assert!(
            line.contains(&format!(" {}`{function} + ", image())),
            "{frames:?}"
        );
    }
    assert_eq!(frames.len(), 7, "{frames:?}");
    session.send("bt 2\n");
    assert_eq!(body(&session.until_prompt()), frames[..3]);

    // The registers isPrime starts with, and the function's return.
    session.send(&format!("list reg {} sp\n", cpu::ARG0));
    let registers = table_rows(&session.until_prompt(), REGISTER_COLUMNS);
    assert!(
        registers.len() == 2
            && registers[0][0] == cpu::ARG0
            && registers[0][1].starts_with("0x")
            && registers[0][2].ends_with(')')
            && registers[1][0] == "sp"
            && registers[1][1].starts_with("0x"),
        "{registers:?}"
    );
    session.send("show mem sp 16\n");
    let memory = body(&session.until_prompt());
    assert!(memory[0].starts_with("Memory at 0x") && memory[0].ends_with("(sp of thread 0):"));
    assert_eq!(memory.len(), 2, "{memory:?}");
    session.send("finish\n");
    let returned = body(&session.until_prompt());
    assert!(
        returned.len() == 1
            && returned[0].starts_with("Thread 0 returned from isPrime to 0x")
            && returned[0].contains(" in countPrimes+0x")
            && returned[0].contains(&format!(
                ": {} = 0x0000000000000000 (0), {} = 0x",
                cpu::ARG0,
                cpu::ARG1
            )),
        "{returned:?}"
    );
    // countPrimes calls isPrime again before it returns: the breakpoint
    // there ends this finish early, and says so.
    session.send("finish\n");
    let interrupted = body(&session.until_prompt());
    assert!(
        interrupted.len() == 2
            && interrupted[0].starts_with("--[BP] T: 0 P: 0x")
            && interrupted[0].ends_with(" F: (isPrime) _mfb_fn_isPrime"),
        "{interrupted:?}"
    );
    assert_eq!(
        interrupted[1],
        "The finish from countPrimes did not complete; type 'finish' to run it again from here."
    );

    session.send("list breakpoint\n");
    let listed = table_rows(&session.until_prompt(), BREAKPOINT_COLUMNS);
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().all(|cells| is_address(&cells[0])));

    session.send("bp r isPrime\n");
    session.until_prompt();
    session.send("run\n");
    let second = body(&session.until_prompt());
    assert!(
        second
            .last()
            .is_some_and(|line| line.starts_with("--[BP] T: 0 P: 0x")
                && line.ends_with(&format!(" F: {}", exit_stop()))),
        "{second:?}"
    );
    // The program printed its rounds before reaching `exit`.
    assert!(
        second.iter().any(|line| line == "total: 68 primes"),
        "{second:?}"
    );

    session.send("run\n");
    assert_eq!(body(&session.until_prompt()), [EXITED]);
    // After the end, restart: a new process, the breakpoint on exit
    // pending again until the libraries load.
    session.send("restart\n");
    let restarted = body(&session.until_prompt());
    assert!(
        restarted[0].starts_with("Restarted the program as process ")
            && restarted[0].ends_with("; paused before its first instruction."),
        "{restarted:?}"
    );
    // Only the pending breakpoint is left, so the address column is as
    // wide as its header (`prompt::table` aligns to the widest entry).
    session.send("list breakpoint\n");
    assert_eq!(
        body(&session.until_prompt()),
        [
            "Address | Count | Symbol | Written name".to_string(),
            format!("-       | 0     | {EXIT_SYMBOL:<6} | {}", exit_name())
        ]
    );
    session.send("run\n");
    let again = body(&session.until_prompt());
    // 031f64353 made every stop line a tagged `--[BP] T: P: F:` line; the
    // first run's stop above was updated, this restart's stop was not.
    assert!(
        again
            .last()
            .is_some_and(|line| line.starts_with("--[BP] T: 0 P: 0x")
                && line.ends_with(&format!(" F: {}", exit_stop()))),
        "{again:?}"
    );
    session.send("q\n");
    let (code, rest) = session.finish();
    assert_eq!(code, Some(0));
    assert!(rest.contains("Killed process "), "{rest}");
}

/// riscv64 Linux has no watchpoints: `wp` says so, and none is set.
#[cfg(target_arch = "riscv64")]
#[test]
fn watchpoints_are_refused_on_riscv64() {
    let mfb = signed_mfb();
    let binary = debug_example();
    let mut child = Command::new(&mfb)
        .args(["dbg", binary.to_str().unwrap(), "--", "2", "500"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start mfb dbg");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"wp sp 8\nlist watchpoint\nq\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(output.status.code(), Some(0), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        "error: this system has no watchpoints: the riscv64 kernel does not provide them\n"
    );
    assert!(
        stdout.contains("DBG> list watchpoint\nno watchpoints\n"),
        "{stdout}"
    );
}
