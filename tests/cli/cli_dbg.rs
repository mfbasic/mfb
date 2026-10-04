//! `mfb dbg`: the command line on every host, and on macOS real sessions — a
//! launched program paused before it runs and killed on `q`, and a running one
//! attached to, paused, and left running on `q`.
//!
//! macOS hands a debugger another program's task port only when the debugger is
//! signed with the `com.apple.security.cs.debugger` entitlement. The cargo-built
//! `mfb` is not, so the live sessions run a copy signed by
//! `scripts/sign-debugger.sh` — which is also the proof that script works.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[path = "../common/mod.rs"]
mod common;
use common::*;

fn dbg(mfb: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(mfb)
        .arg("dbg")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run mfb dbg");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn help_and_usage_errors_on_every_host() {
    let mfb = PathBuf::from(mfb_exe());
    let (code, stdout, _) = dbg(&mfb, &["--help"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.starts_with("Usage: mfb dbg <binary> [-- <args>...]"),
        "{stdout}"
    );

    let (code, _, stderr) = dbg(&mfb, &[]);
    assert_eq!(code, Some(2));
    assert!(
        stderr.starts_with("error: mfb dbg requires a <binary> or a <pid>"),
        "{stderr}"
    );

    // `--help` after `--` is the program's, not the debugger's.
    let (code, _, stderr) = dbg(&mfb, &["4242", "--", "--help"]);
    assert_eq!(code, Some(2));
    assert!(
        stderr.starts_with("error: program arguments after --"),
        "{stderr}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_debugger_is_not_supported_off_macos() {
    let project = temp_project("dbg_probe", "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
    let binary = build_project(&project);
    let (code, stdout, stderr) = dbg(&PathBuf::from(mfb_exe()), &[binary.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{stdout}");
    assert_eq!(
        stderr,
        format!(
            "error: mfb dbg is not supported on {}\n",
            std::env::consts::OS
        )
    );
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    /// A copy of `mfb` signed for debugging, in a directory of its own.
    fn signed_mfb() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mfb_dbg_signed_{}", unique_nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        let mfb = dir.join("mfb");
        std::fs::copy(mfb_exe(), &mfb).expect("copy mfb");
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/sign-debugger.sh");
        let output = Command::new("sh")
            .arg(script)
            .arg(&mfb)
            .output()
            .expect("run sign-debugger.sh");
        assert!(
            output.status.success(),
            "sign-debugger.sh failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        mfb
    }

    /// A debugger session whose stdout is read up to each prompt.
    struct Session {
        child: Child,
        chunks: mpsc::Receiver<Vec<u8>>,
        seen: String,
    }

    impl Session {
        fn start(mfb: &Path, args: &[&str]) -> Self {
            let mut child = Command::new(mfb)
                .arg("dbg")
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start mfb dbg");
            let mut stdout = child.stdout.take().unwrap();
            let (send, chunks) = mpsc::channel();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 4096];
                while let Ok(read) = stdout.read(&mut buffer) {
                    if read == 0 || send.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
            });
            Session {
                child,
                chunks,
                seen: String::new(),
            }
        }

        /// Everything printed up to and including the next `DBG> `.
        fn until_prompt(&mut self) -> String {
            while !self.seen.ends_with("DBG> ") {
                let chunk = self
                    .chunks
                    .recv_timeout(Duration::from_secs(30))
                    .unwrap_or_else(|_| panic!("no DBG> prompt; output so far:\n{}", self.seen));
                self.seen.push_str(&String::from_utf8_lossy(&chunk));
            }
            std::mem::take(&mut self.seen)
        }

        fn send(&mut self, line: &str) {
            let stdin = self.child.stdin.as_mut().unwrap();
            stdin.write_all(line.as_bytes()).unwrap();
            stdin.flush().unwrap();
        }

        /// Close stdin and collect the exit code and the rest of stdout.
        fn finish(mut self) -> (Option<i32>, String) {
            drop(self.child.stdin.take());
            let status = self.child.wait().unwrap();
            while let Ok(chunk) = self.chunks.recv_timeout(Duration::from_secs(5)) {
                self.seen.push_str(&String::from_utf8_lossy(&chunk));
            }
            (status.code(), self.seen)
        }
    }

    /// The PID a launch or attach banner names.
    fn banner_pid(banner: &str) -> u32 {
        let first = banner.lines().next().unwrap();
        let digits = first
            .split(|c: char| !c.is_ascii_digit())
            .find(|word| !word.is_empty())
            .unwrap_or_else(|| panic!("no PID in {first:?}"));
        digits.parse().unwrap()
    }

    fn process_exists(pid: u32) -> bool {
        Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// What follows the banner for a program built without `--debug`.
    const NOT_A_DEBUG_BUILD: &str = "note: this is not a debug build, so the program's function names are missing.\n      Rebuild it with: mfb build --debug -O0 <project>\n";

    /// A program that prints a tick every 100 ms for a few minutes, through a
    /// function whose name needs escaping.
    const TICKER: &str = "IMPORT io\nIMPORT os\n\nFUNC next_tick(i AS Integer) AS Integer\n  \
        io::print(\"tick\")\n  os::sleep(100)\n  RETURN i + 1\nEND FUNC\n\n\
        FUNC main() AS Nothing\n  MUT i AS Integer = 0\n  WHILE i < 1500\n    \
        i = next_tick(i)\n  END WHILE\nEND FUNC\n";

    /// Project `name` built with `--debug -O0`, so it carries its function
    /// names.
    fn build_debug(project: &Path, name: &str) -> PathBuf {
        let output = Command::new(mfb_exe())
            .args(["build", "-q", "--debug", "-O0"])
            .arg(project)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        project.join("build").join(format!("{name}.out"))
    }

    /// The `list func` rows of a reply, split into their four columns.
    fn function_rows(reply: &str) -> Vec<[String; 4]> {
        table_rows(reply, FUNCTION_COLUMNS)
            .iter()
            .map(|line| {
                let words = line.split_whitespace().collect::<Vec<_>>();
                assert_eq!(words.len(), 4, "{line}");
                [0, 1, 2, 3].map(|at| words[at].to_string())
            })
            .collect()
    }

    fn is_address(word: &str) -> bool {
        word.len() == 18
            && word.starts_with("0x")
            && word[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    fn line_count(path: &Path) -> usize {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[test]
    fn a_launched_program_does_not_run_and_is_killed_on_quit() {
        let mfb = signed_mfb();
        let project = temp_project(
            "dbg_launch",
            "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(\"ran\")\nEND FUNC\n",
        );
        let binary = build_project(&project);
        let mut session = Session::start(&mfb, &[binary.to_str().unwrap(), "--", "alpha", "-b"]);
        let banner = session.until_prompt();
        assert!(
            banner.starts_with(&format!("Launched {} as process ", binary.display()))
                && banner.ends_with(&format!(
                    "; paused before its first instruction.\n{NOT_A_DEBUG_BUILD}DBG> "
                )),
            "{banner}"
        );
        let pid = banner_pid(
            banner
                .strip_prefix(&format!("Launched {}", binary.display()))
                .unwrap(),
        );
        let args = Command::new("ps")
            .args(["-o", "args=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&args.stdout).trim(),
            format!("{} alpha -b", binary.display())
        );

        session.send("help\n");
        let help = session.until_prompt();
        assert!(help.starts_with("help\nCommands:\n  help, h"), "{help}");
        session.send("q\n");
        let (code, rest) = session.finish();
        assert_eq!(code, Some(0));
        assert_eq!(rest, format!("q\nKilled process {pid}.\n"));
        // The program printed nothing: it never ran a single instruction.
        assert!(!rest.contains("ran") && !help.contains("ran"));
        assert!(!process_exists(pid), "process {pid} outlived the session");
    }

    #[test]
    fn an_attached_program_pauses_and_runs_on_after_quit() {
        let mfb = signed_mfb();
        let project = temp_project(
            "dbg_attach",
            "IMPORT io\nIMPORT os\n\nFUNC main() AS Nothing\n  MUT i AS Integer = 0\n  \
             WHILE i < 1500\n    io::print(\"tick\")\n    os::sleep(100)\n    i = i + 1\n  \
             END WHILE\nEND FUNC\n",
        );
        let binary = build_project(&project);
        let log = project.join("ticks.log");
        let mut ticker = Command::new(&binary)
            .stdout(std::fs::File::create(&log).unwrap())
            .spawn()
            .expect("start the ticker");
        while line_count(&log) < 3 {
            std::thread::sleep(Duration::from_millis(50));
        }

        let mut session = Session::start(&mfb, &[&ticker.id().to_string()]);
        let banner = session.until_prompt();
        assert_eq!(
            banner,
            format!(
                "Attached to process {} ({}); paused.\n{NOT_A_DEBUG_BUILD}DBG> ",
                ticker.id(),
                std::fs::canonicalize(&binary).unwrap().display()
            )
        );
        let paused = line_count(&log);
        std::thread::sleep(Duration::from_millis(1000));
        assert_eq!(line_count(&log), paused, "the program ran while paused");

        session.send("quit\n");
        let (code, rest) = session.finish();
        assert_eq!(code, Some(0));
        assert_eq!(
            rest,
            format!(
                "quit\nDetached from process {}; it continues running.\n",
                ticker.id()
            )
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while line_count(&log) == paused {
            assert!(
                std::time::Instant::now() < deadline,
                "the program did not run on"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        ticker.kill().unwrap();
        ticker.wait().unwrap();
    }

    #[test]
    fn programs_that_are_not_mfbasic_or_not_native_are_refused() {
        let mfb = signed_mfb();
        let (code, _, stderr) = dbg(&mfb, &["/bin/ls"]);
        assert_eq!(code, Some(1));
        assert_eq!(stderr, "error: '/bin/ls' is not an MFBASIC executable\n");

        let project = temp_project(
            "dbg_foreign",
            "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
        let output = Command::new(mfb_exe())
            .args(["build", "-q", "--target", "linux-x86_64"])
            .arg(&project)
            .output()
            .unwrap();
        assert!(output.status.success());
        let foreign = project.join("build/dbg_foreign-glibc.out");
        let (code, _, stderr) = dbg(&mfb, &[foreign.to_str().unwrap()]);
        assert_eq!(code, Some(1));
        assert_eq!(
            stderr,
            format!(
                "error: '{}' is built for ELF x86-64; this debugger runs Mach-O aarch64 programs\n",
                foreign.display()
            )
        );

        // A running process that is not an MFBASIC program: this test.
        let (code, _, stderr) = dbg(&mfb, &[&std::process::id().to_string()]);
        assert_eq!(code, Some(1));
        assert!(
            stderr.ends_with("is not an MFBASIC executable\n"),
            "{stderr}"
        );

        let (code, _, stderr) = dbg(&mfb, &["999999"]);
        assert_eq!(code, Some(1));
        assert_eq!(stderr, "error: no process with PID 999999\n");
    }

    /// The cargo-built `mfb` carries no debugger entitlement, so macOS refuses it
    /// the task port; the error says how to sign it, and the program it started
    /// is not left behind. Root is exempt from the entitlement check.
    #[test]
    fn an_unsigned_mfb_explains_how_to_sign_itself() {
        let uid = Command::new("id").arg("-u").output().unwrap();
        if String::from_utf8_lossy(&uid.stdout).trim() == "0" {
            return;
        }
        let project = temp_project(
            "dbg_unsigned",
            "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
        let binary = build_project(&project);
        let (code, stdout, stderr) = dbg(&PathBuf::from(mfb_exe()), &[binary.to_str().unwrap()]);
        assert_eq!(code, Some(1), "{stdout}");
        assert!(
            stderr.starts_with("error: cannot attach to process ")
                && stderr.contains("com.apple.security.cs.debugger")
                && stderr.contains("sh scripts/sign-debugger.sh "),
            "{stderr}"
        );
        let pid = banner_pid(&stderr);
        assert!(
            !process_exists(pid),
            "the refused launch left process {pid}"
        );
    }

    /// A debug build, launched: its own functions under their written names,
    /// the runtime's, and every import record, none bound yet; then thread 0's
    /// registers, every one in its group: gpr, sys, then simd.
    #[test]
    fn list_shows_a_launched_debug_build_s_functions_and_registers() {
        let mfb = signed_mfb();
        let project = temp_project("dbg_listl", TICKER);
        let binary = build_debug(&project, "dbg_listl");
        let mut session = Session::start(&mfb, &[binary.to_str().unwrap()]);
        let banner = session.until_prompt();
        assert!(!banner.contains("note:"), "{banner}");

        session.send("l func\n");
        let listing = session.until_prompt();
        let rows = function_rows(&listing);
        assert!(
            rows.iter().any(|row| row[0] == "MU"
                && is_address(&row[1])
                && row[2] == "_mfb_fn_next_5Ftick"
                && row[3] == "next_tick"),
            "{listing}"
        );
        assert!(rows.iter().any(|row| row[0] == "MU" && row[3] == "main"));
        assert!(rows
            .iter()
            .any(|row| row[0] == "MR" && row[2] == "_main" && row[3] == "-"));
        let linked = rows.iter().filter(|row| row[0] == "LN").collect::<Vec<_>>();
        assert!(!linked.is_empty(), "{listing}");
        assert!(
            linked.iter().all(|row| row[1] == "-"),
            "nothing is bound before the loader runs:\n{listing}"
        );
        // Groups come in order: MU, then MR, then LN.
        let order = rows.iter().map(|row| row[0].clone()).collect::<Vec<_>>();
        let mut sorted = order.clone();
        sorted.sort_by_key(|code| ["MU", "MR", "LN"].iter().position(|c| c == code));
        assert_eq!(order, sorted);

        session.send("list reg gpr sys simd\n");
        let registers = table_rows(&session.until_prompt(), REGISTER_COLUMNS);
        let lines = registers
            .iter()
            .map(|line| line.split_whitespace().collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let names = lines
            .iter()
            .map(|words| words[0].to_string())
            .collect::<Vec<_>>();
        let mut expected = (0..29).map(|index| format!("x{index}")).collect::<Vec<_>>();
        expected.extend(["fp", "lr", "sp", "pc", "nzcv", "fpsr", "fpcr"].map(String::from));
        expected.extend((0..32).map(|index| format!("v{index}")));
        assert_eq!(names, expected, "{registers:?}");
        // General registers print 16 hex digits, vector registers 32.
        for words in &lines {
            let digits = if words[0].starts_with('v') { 32 } else { 16 };
            let value = words[1].strip_prefix("0x").expect("a hex value");
            assert_eq!(value.len(), digits, "{words:?}");
            assert!(
                value.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{words:?}"
            );
        }

        session.send("q\n");
        let (code, _) = session.finish();
        assert_eq!(code, Some(0));
    }

    /// Attached to a running debug build, every import has been bound, and the
    /// program counter is inside the program or a library it calls.
    #[test]
    fn list_shows_an_attached_program_s_bound_imports() {
        let mfb = signed_mfb();
        let project = temp_project("dbg_lista", TICKER);
        let binary = build_debug(&project, "dbg_lista");
        let log = project.join("ticks.log");
        let mut ticker = Command::new(&binary)
            .stdout(std::fs::File::create(&log).unwrap())
            .spawn()
            .expect("start the ticker");
        while line_count(&log) < 3 {
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut session = Session::start(&mfb, &[&ticker.id().to_string()]);
        session.until_prompt();
        session.send("list func\n");
        let listing = session.until_prompt();
        let rows = function_rows(&listing);
        let linked = rows.iter().filter(|row| row[0] == "LN").collect::<Vec<_>>();
        assert!(!linked.is_empty(), "{listing}");
        assert!(
            linked.iter().all(|row| is_address(&row[1])),
            "every import is bound once the program runs:\n{listing}"
        );
        // Every record of one import is bound to the same function.
        for row in &linked {
            assert!(linked
                .iter()
                .filter(|other| other[2] == row[2])
                .all(|other| other[1] == row[1]));
        }
        session.send("q\n");
        let (code, _) = session.finish();
        assert_eq!(code, Some(0));
        ticker.kill().unwrap();
        ticker.wait().unwrap();
    }

    /// Without `--debug` the program names only its imports.
    #[test]
    fn list_func_on_a_normal_build_shows_only_linked_functions() {
        let mfb = signed_mfb();
        let project = temp_project("dbg_listn", TICKER);
        let binary = build_project(&project);
        let mut session = Session::start(&mfb, &[binary.to_str().unwrap()]);
        let banner = session.until_prompt();
        assert!(banner.contains(NOT_A_DEBUG_BUILD), "{banner}");
        session.send("list func\n");
        let listing = session.until_prompt();
        let rows = function_rows(&listing);
        assert!(!rows.is_empty(), "{listing}");
        assert!(rows.iter().all(|row| row[0] == "LN"), "{listing}");
        session.send("q\n");
        let (code, _) = session.finish();
        assert_eq!(code, Some(0));
    }

    /// A copy of `examples/debug`, built with `--debug -O0`: started with a
    /// third argument of 1 it runs each round on a `thread::start` worker.
    fn debug_example() -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/debug");
        let project = std::env::temp_dir().join(format!("mfb_dbg_example_{}", unique_nonce()));
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::copy(source.join("project.json"), project.join("project.json")).unwrap();
        for entry in std::fs::read_dir(source.join("src")).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), project.join("src").join(entry.file_name())).unwrap();
        }
        build_debug(&project, "debug")
    }

    /// The lines between a command's echo and the next prompt.
    fn body(reply: &str) -> Vec<String> {
        reply
            .lines()
            .skip(1)
            .take_while(|line| !line.starts_with("DBG>"))
            .map(str::to_string)
            .collect()
    }

    /// The columns of each listing's header line.
    const FUNCTION_COLUMNS: &[&str] = &["Type", "Address", "Symbol", "Written name"];
    const THREAD_COLUMNS: &[&str] = &["ID", "Name", "PC", "Task"];
    const REGISTER_COLUMNS: &[&str] = &["Register", "Hex", "Decimal"];
    const ARENA_COLUMNS: &[&str] = &["ID", "Thread", "Address", "Size (KB)"];
    const BREAKPOINT_COLUMNS: &[&str] = &["Address", "Symbol", "Written name"];

    /// The rows of a listing reply, after checking that its first line is the
    /// header naming `columns` and that every row starts each column where the
    /// header does.
    fn table_rows(reply: &str, columns: &[&str]) -> Vec<String> {
        let mut lines = body(reply);
        assert!(!lines.is_empty(), "no listing in {reply:?}");
        let header = lines.remove(0).chars().collect::<Vec<_>>();
        let text = header.iter().collect::<String>();
        let mut starts = Vec::new();
        let mut from = 0;
        for column in columns {
            let at = text[from..]
                .find(column)
                .map(|at| text[..from + at].chars().count())
                .unwrap_or_else(|| panic!("no column {column:?} in {text:?}"));
            starts.push(at);
            from = text.char_indices().nth(at).unwrap().0 + column.len();
        }
        assert_eq!(starts[0], 0, "{text:?}");
        assert_eq!(
            text.split_whitespace().collect::<Vec<_>>(),
            columns.join(" ").split_whitespace().collect::<Vec<_>>(),
            "{text:?}"
        );
        for line in &lines {
            let cells = line.chars().collect::<Vec<_>>();
            for start in &starts[1..] {
                assert!(
                    cells.get(start - 1) == Some(&' ')
                        && cells.get(*start).is_some_and(|cell| *cell != ' '),
                    "a column of {line:?} does not start at {start}, under {text:?}"
                );
            }
        }
        lines
    }

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
        let words = threads[0].split_whitespace().collect::<Vec<_>>();
        assert_eq!(words[0], "0");
        assert!(is_address(words[2]), "{threads:?}");
        assert_eq!(words[3..], ["MFB", "Main"]);

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
        let mut example = Command::new(&binary)
            .args(["3000", "20000", "1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the example");
        std::thread::sleep(Duration::from_millis(1000));
        let mut session = Session::start(&mfb, &[&example.id().to_string()]);
        session.until_prompt();

        session.send("list thread\n");
        let threads = table_rows(&session.until_prompt(), THREAD_COLUMNS);
        let task = |line: &String| {
            line.split_whitespace()
                .skip(3)
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(task(&threads[0]), "MFB Main", "{threads:?}");
        let workers = threads
            .iter()
            .filter(|line| task(line) == "User Thread")
            .map(|line| line.split_whitespace().next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(workers.len(), 1, "{threads:?}");

        session.send("list arena\n");
        let arenas = table_rows(&session.until_prompt(), ARENA_COLUMNS);
        let rows = arenas
            .iter()
            .map(|line| {
                line.split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
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
        let task = |line: &String| {
            line.split_whitespace()
                .skip(3)
                .collect::<Vec<_>>()
                .join(" ")
        };
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
        session.send("show type string x1\n");
        assert_eq!(
            body(&session.until_prompt()),
            ["Length: 18 Value: \"round 1: 37 primes\""]
        );
        session.send("show type int x1\n");
        assert_eq!(
            body(&session.until_prompt()),
            ["Value: 0x0000000000000012 (18)"]
        );
        session.send("q\n");
        let (code, _) = session.finish();
        assert_eq!(code, Some(0));
    }

    /// Breakpoints by written name and by a system function's symbol (pending
    /// until the libraries load), `run` to each, then to the end.
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
        session.send("bp _exit\n");
        assert_eq!(
            body(&session.until_prompt()),
            [
                "Set a pending breakpoint on _exit: it is placed when the program has loaded its \
              libraries."
            ]
        );

        session.send("run\n");
        let first = body(&session.until_prompt());
        assert!(
            first.len() == 2
                && first[0].starts_with("Placed the pending breakpoint on _exit at 0x")
                && first[1].starts_with("Thread 0 hit a breakpoint at 0x")
                && first[1].ends_with(" in isPrime."),
            "{first:?}"
        );

        // Stopped in isPrime: the current function, a few instructions, and a
        // system function by name.
        session.send("dis\n");
        let function = body(&session.until_prompt());
        assert_eq!(function[0], "debug.out`isPrime:", "{function:?}");
        assert!(function[1].starts_with("->  0x"), "{function:?}");
        assert!(
            function[1].ends_with("sub    sp, sp, #0xf0"),
            "{function:?}"
        );
        assert!(function.last().unwrap().ends_with("ret") || function.len() > 10);
        session.send("dis 3\n");
        let three = body(&session.until_prompt());
        assert_eq!(three.len(), 4, "{three:?}");
        session.send("dis exit\n");
        let exit = body(&session.until_prompt());
        assert_eq!(exit[0], "libsystem_c.dylib`exit:", "{exit:?}");

        // The calls that led to isPrime, innermost first, ending at the
        // program's entry; `bt 2` keeps the innermost two.
        session.send("bt\n");
        let frames = body(&session.until_prompt());
        assert_eq!(frames[0], "* thread #0, task = 'MFB Main'", "{frames:?}");
        assert!(frames[1].starts_with("  * frame #0: 0x"), "{frames:?}");
        assert!(frames[1].ends_with(" debug.out`isPrime"), "{frames:?}");
        for (line, function) in frames[2..].iter().zip([
            "debug.out`countPrimes",
            "debug.out`runRound",
            "debug.out`runRounds",
            "debug.out`main",
            "debug.out`main",
        ]) {
            assert!(line.contains(&format!(" {function} + ")), "{frames:?}");
        }
        assert_eq!(frames.len(), 7, "{frames:?}");
        session.send("bt 2\n");
        assert_eq!(body(&session.until_prompt()), frames[..3]);

        // The registers isPrime starts with, and the function's return.
        session.send("list reg x0 sp\n");
        let registers = table_rows(&session.until_prompt(), REGISTER_COLUMNS);
        assert!(
            registers.len() == 2
                && registers[0].starts_with("x0       0x")
                && registers[0].ends_with(')')
                && registers[1].starts_with("sp       0x"),
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
                && returned[0].contains(": x0 = 0x0000000000000000 (0), x1 = 0x"),
            "{returned:?}"
        );

        session.send("list breakpoint\n");
        let listed = table_rows(&session.until_prompt(), BREAKPOINT_COLUMNS);
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert!(listed
            .iter()
            .all(|line| is_address(line.split_whitespace().next().unwrap())));

        session.send("bp r isPrime\n");
        session.until_prompt();
        session.send("run\n");
        let second = body(&session.until_prompt());
        assert!(
            second
                .last()
                .is_some_and(|line| line.starts_with("Thread 0 hit a breakpoint at 0x")
                    && line.ends_with(" in exit.")),
            "{second:?}"
        );
        // The program printed its rounds before reaching `exit`.
        assert!(
            second.iter().any(|line| line == "total: 68 primes"),
            "{second:?}"
        );

        session.send("run\n");
        assert_eq!(
            body(&session.until_prompt()),
            ["The program exited with status 0."]
        );
        // After the end, restart: a new process, the breakpoint on exit
        // pending again until the libraries load.
        session.send("restart\n");
        let restarted = body(&session.until_prompt());
        assert!(
            restarted[0].starts_with("Restarted the program as process ")
                && restarted[0].ends_with("; paused before its first instruction."),
            "{restarted:?}"
        );
        session.send("list breakpoint\n");
        assert_eq!(
            body(&session.until_prompt()),
            [
                "Address            Symbol Written name",
                "-                  _exit  exit"
            ]
        );
        session.send("run\n");
        let again = body(&session.until_prompt());
        assert!(
            again.last().is_some_and(|line| line.ends_with(" in exit.")),
            "{again:?}"
        );
        session.send("q\n");
        let (code, rest) = session.finish();
        assert_eq!(code, Some(0));
        assert!(rest.contains("Killed process "), "{rest}");
    }
}
