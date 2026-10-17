//! `mfb dbg`: the command line on every host, and on macOS and Linux on AArch64
//! real sessions — a launched program paused before it runs and killed on `q`,
//! and a running one attached to, paused, and left running on `q`.
//!
//! macOS hands a debugger another program's task port only when the debugger is
//! signed with the `com.apple.security.cs.debugger` entitlement. The cargo-built
//! `mfb` is not, so the live sessions there run a copy signed by
//! `scripts/sign-debugger.sh` — which is also the proof that script works.
//! Linux needs no signing; a system whose `kernel.yama.ptrace_scope` keeps a
//! debugger from attaching to a program it did not start has each attaching
//! test check the refusal instead.

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

#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64"))))]
#[test]
fn the_debugger_is_not_supported_off_macos_and_linux_aarch64() {
    let project = temp_project("dbg_probe", "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
    let binary = build_project(&project);
    let (code, stdout, stderr) = dbg(&PathBuf::from(mfb_exe()), &[binary.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{stdout}");
    assert_eq!(
        stderr,
        format!(
            "error: mfb dbg is not supported on {} {}; it runs on macOS and on Linux on \
             AArch64\n",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    );
}

#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))]
mod native {
    use super::*;

    /// The `mfb` the sessions run: on Linux the one cargo built, which needs
    /// nothing more.
    #[cfg(target_os = "linux")]
    fn signed_mfb() -> PathBuf {
        PathBuf::from(mfb_exe())
    }

    /// A copy of `mfb` signed for debugging, in a directory of its own.
    #[cfg(target_os = "macos")]
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
        if cfg!(target_os = "linux") {
            return Path::new(&format!("/proc/{pid}")).exists();
        }
        Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// The executable `mfb build` writes for project `name` that this host
    /// runs: `<name>.out` on macOS, the host libc's `<name>-<libc>.out` on
    /// Linux.
    fn executable_name(name: &str) -> String {
        if cfg!(target_os = "linux") {
            format!("{name}-{}.out", host_libc_flavor())
        } else {
            format!("{name}.out")
        }
    }

    /// How a frame or a listing names `examples/debug`'s executable.
    fn image() -> String {
        executable_name("debug")
    }

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
            "ld-musl-aarch64.so.1"
        } else {
            "libc.so.6"
        }
    }

    /// How the refusal of a foreign program names this host's programs.
    fn host_image() -> &'static str {
        if cfg!(target_os = "macos") {
            "Mach-O aarch64"
        } else {
            "ELF aarch64"
        }
    }

    /// Whether this system lets a debugger attach to a program it did not
    /// start. When it does not, the attaching tests check that `mfb dbg`
    /// says so and why.
    fn attach_allowed(mfb: &Path, pid: u32) -> bool {
        let scope = std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
            .map(|text| text.trim().to_string())
            .unwrap_or_else(|_| "0".to_string());
        if !cfg!(target_os = "linux") || scope == "0" {
            return true;
        }
        let (code, _, stderr) = dbg(mfb, &[&pid.to_string()]);
        assert_eq!(code, Some(1), "{stderr}");
        assert!(
            stderr.starts_with(&format!("error: cannot attach to process {pid}: ptrace: "))
                && stderr.contains(&format!("kernel.yama.ptrace_scope is {scope}")),
            "{stderr}"
        );
        false
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
        project.join("build").join(executable_name(name))
    }

    /// The `list func` rows of a reply, split into their four columns.
    /// The rows of a `list func` reply, which ends with a legend after an
    /// empty line: one `<type>  <meaning>` line per type the rows show.
    fn function_rows(reply: &str) -> Vec<[String; 4]> {
        let (table, legend) = reply
            .split_once("\n\n")
            .unwrap_or_else(|| panic!("no legend after the listing in {reply:?}"));
        let rows = table_rows(table, FUNCTION_COLUMNS)
            .into_iter()
            .map(|cells| {
                // The written name may carry a tag (`exit <mfb>`).
                assert!(!cells[3].is_empty(), "{cells:?}");
                <[String; 4]>::try_from(cells).unwrap()
            })
            .collect::<Vec<_>>();
        let mut shown = rows.iter().map(|row| row[0].clone()).collect::<Vec<_>>();
        shown.dedup();
        let explained = legend
            .lines()
            .take_while(|line| !line.starts_with("DBG>"))
            .map(|line| line.split_whitespace().next().unwrap_or("").to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            explained, shown,
            "the legend explains the types shown: {reply:?}"
        );
        rows
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

        if !attach_allowed(&mfb, ticker.id()) {
            ticker.kill().unwrap();
            ticker.wait().unwrap();
            return;
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
                "error: '{}' is built for ELF x86-64; this debugger runs {} programs\n",
                foreign.display(),
                host_image()
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
    #[cfg(target_os = "macos")]
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
        // The program's `main` reads as written; the runtime's `_main` has
        // no written name, so the two do not share one.
        assert!(rows.iter().any(|row| row[0] == "MU" && row[3] == "main"));
        assert!(rows
            .iter()
            .any(|row| row[0] == "MR" && row[2] == "_main" && row[3] == "-"));
        let linked = rows.iter().filter(|row| row[0] == "SL").collect::<Vec<_>>();
        assert!(!linked.is_empty(), "{listing}");
        assert!(
            linked.iter().all(|row| row[1] == "-"),
            "nothing is bound before the loader runs:\n{listing}"
        );
        // Groups come in order: MU, MB, MR, then SL.
        let order = rows.iter().map(|row| row[0].clone()).collect::<Vec<_>>();
        let mut sorted = order.clone();
        sorted.sort_by_key(|code| ["MU", "MB", "MR", "SL"].iter().position(|c| c == code));
        assert_eq!(order, sorted);

        session.send("list reg gpr sys simd\n");
        let registers = table_rows(&session.until_prompt(), REGISTER_COLUMNS);
        let lines = registers
            .iter()
            .map(|cells| cells.iter().map(String::as_str).collect::<Vec<_>>())
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

        // On its own it shows the registers a call uses, each with its role,
        // and the usage under them.
        session.send("list reg\n");
        let common = session.until_prompt();
        let roles = common
            .lines()
            .filter_map(|line| {
                let cells = line.split(" | ").map(str::trim).collect::<Vec<_>>();
                (cells.len() == 4 && cells[0] != "Register").then(|| (cells[0], cells[3]))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                ("x0", "MFB arg, MFB result, C arg, C result"),
                ("x1", "MFB arg, MFB result, C arg, C result"),
                ("x2", "MFB arg, MFB result, C arg"),
                ("x3", "MFB arg, MFB result, C arg"),
                ("x4", "MFB arg, C arg"),
                ("x5", "MFB arg, C arg"),
                ("x6", "MFB arg, C arg"),
                ("x7", "MFB arg, C arg"),
                ("x19", "Arena"),
                ("x20", "Current Thread"),
                ("x28", "Closure"),
            ],
            "{common}"
        );
        assert!(
            common.contains("\n\nUsage: list reg [<id>] [<what>...]\n"),
            "{common}"
        );

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
        if !attach_allowed(&mfb, ticker.id()) {
            ticker.kill().unwrap();
            ticker.wait().unwrap();
            return;
        }
        let mut session = Session::start(&mfb, &[&ticker.id().to_string()]);
        session.until_prompt();
        session.send("list func\n");
        let listing = session.until_prompt();
        let rows = function_rows(&listing);
        let linked = rows.iter().filter(|row| row[0] == "SL").collect::<Vec<_>>();
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

    /// A program whose comparison overflows inside an inline `TRAP`.
    const RAISES: &str = "IMPORT io\n\nFUNC check(flag AS Boolean) AS Integer\n  RETURN 1\n\
        END FUNC\n\nFUNC main() AS Integer\n  MUT fx = 1F\n  MUT big = 3000000000\n  \
        LET b = check(fx < big) TRAP(e)\n    io::print(\"caught \" & toString(e.code))\n    \
        RECOVER 9\n  END TRAP\n  RETURN 0\nEND FUNC\n";

    /// `breakerror` stops where an error it watches is raised, before a `TRAP`
    /// takes it, and `show error` reads it from the thread's error slot.
    #[test]
    fn breakerror_stops_where_a_watched_error_is_raised() {
        let mfb = signed_mfb();
        let project = temp_project("dbg_raise", RAISES);
        let binary = build_debug(&project, "dbg_raise");
        let mut session = Session::start(&mfb, &[binary.to_str().unwrap()]);
        session.until_prompt();
        session.send("be ErrOverflow\n");
        assert_eq!(
            body(&session.until_prompt()),
            ["Stopping when a thread raises 77050010 (errorCode::ErrOverflow)."]
        );
        session.send("run\n");
        let raised = body(&session.until_prompt());
        assert!(
            raised.len() == 1
                && raised[0].starts_with("--[BE] T: 0 P: 0x")
                && raised[0].contains(" F: (main) _mfb_fn_main+0x")
                && raised[0].ends_with(" E: errorCode::ErrOverflow (77050010)"),
            "{raised:?}"
        );
        session.send("show error\n");
        let error = body(&session.until_prompt());
        assert!(
            error.len() == 4
                && error[0].starts_with("Thread 0's error in flight (the Error at 0x")
                && error[1] == "Code: 77050010 (errorCode::ErrOverflow)"
                && error[2].starts_with("Message: \"Arithmetic overflow")
                && error[3].starts_with("Location: src/main.mfb:10:"),
            "{error:?}"
        );
        session.send("be clear\n");
        assert_eq!(
            body(&session.until_prompt()),
            ["Stopped watching for errors."]
        );
        session.send("run\n");
        let end = body(&session.until_prompt());
        assert!(
            end.iter().any(|line| line == "caught 77050010")
                && end.last().is_some_and(|line| line == EXITED),
            "{end:?}"
        );
        session.send("q\n");
        let (code, _) = session.finish();
        assert_eq!(code, Some(0));
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
        assert!(rows.iter().all(|row| row[0] == "SL"), "{listing}");
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

    /// How a `--debug` build that exits with status 0 ends: its report is
    /// kept for `show report`.
    const EXITED: &str =
        "--[EX] Program exit (0) - type \"show report\" to see the performance and memory report";

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
    const BREAKPOINT_COLUMNS: &[&str] = &["Address", "Count", "Symbol", "Written name"];

    /// What separates a listing's columns.
    const SEPARATOR: &str = " | ";

    /// The cells of each row of a listing reply, after checking that its
    /// first line is the header naming `columns`, separated by ` | `, and that
    /// every row has a cell for each, its separators under the header's.
    fn table_rows(reply: &str, columns: &[&str]) -> Vec<Vec<String>> {
        let mut lines = body(reply);
        assert!(!lines.is_empty(), "no listing in {reply:?}");
        let header = lines.remove(0);
        let separators = |line: &str| {
            line.match_indices(SEPARATOR)
                .map(|(at, _)| line[..at].chars().count())
                .collect::<Vec<_>>()
        };
        let names = header
            .split(SEPARATOR)
            .map(str::trim_end)
            .collect::<Vec<_>>();
        assert_eq!(names, columns, "{header:?}");
        let under = separators(&header);
        lines
            .iter()
            .map(|line| {
                let cells = line
                    .split(SEPARATOR)
                    .map(|cell| cell.trim_end().to_string())
                    .collect::<Vec<_>>();
                assert_eq!(cells.len(), columns.len(), "{line:?} under {header:?}");
                assert_eq!(
                    separators(line),
                    under,
                    "the columns of {line:?} are not under {header:?}"
                );
                assert!(
                    cells
                        .iter()
                        .all(|cell| !cell.is_empty() && !cell.starts_with(' ')),
                    "{line:?} under {header:?}"
                );
                cells
            })
            .collect()
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
        let mut example = Command::new(&binary)
            .args(["3000", "20000", "1"])
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
        // Do not guess how long the process needs to start its first worker.
        // Under the full parallel suite a fixed one-second sleep can expire
        // while the child is still in startup. Attach/list/detach until the
        // runtime registry itself reports the state this test exercises.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let (mut session, threads, workers) = loop {
            let mut session = Session::start(&mfb, &[&example.id().to_string()]);
            session.until_prompt();
            session.send("list thread\n");
            let threads = table_rows(&session.until_prompt(), THREAD_COLUMNS);
            let workers = threads
                .iter()
                .filter(|cells| task(cells) == "User Thread")
                .map(|cells| cells[0].clone())
                .collect::<Vec<_>>();
            if workers.len() == 1 {
                break (session, threads, workers);
            }
            session.send("q\n");
            let (code, _) = session.finish();
            assert_eq!(code, Some(0));
            assert!(
                std::time::Instant::now() < deadline,
                "the example never published its worker thread: {threads:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(task(&threads[0]), "MFB Main", "{threads:?}");
        assert_eq!(workers.len(), 1, "{threads:?}");

        session.send("list arena\n");
        let arenas = table_rows(&session.until_prompt(), ARENA_COLUMNS);
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
        let listed = session.until_prompt();
        assert!(
            !listed.contains("MFB_DBG_GREETING")
                && listed
                    .contains("Changed since the program started: it gets these when it restarts."),
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
        session.send("bp countPrimes list reg x1; bt 1; run\n");
        let set = body(&session.until_prompt());
        assert!(
            set.len() == 1
                && set[0].ends_with(" in countPrimes. Each stop runs 'list reg x1; bt 1; run'."),
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
                "x1       | 0x00000000000001f4 | (500)",
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
        session.send("bpi x0 eq 824084 at countPrimes\n");
        let set = body(&session.until_prompt());
        assert!(
            set.len() == 1
                && set[0].ends_with(" in countPrimes if x0 eq 0x00000000000c9314 (824084)."),
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
                        "x0 eq 0x00000000000c9314 (824084)".to_string()
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
        session.send("list reg x0\n");
        assert_eq!(
            table_rows(&session.until_prompt(), REGISTER_COLUMNS),
            [["x0", "0x00000000000c9314", "(824084)"]]
        );
        // At its first instruction the function has not touched its frame,
        // so going to lr returns from it: no error (x0) and a count of 7 (x1).
        session.send("j lr x0=0 x1=7\n");
        let moved = body(&session.until_prompt());
        assert!(
            moved.len() == 1
                && moved[0].starts_with("Moved thread 0 to 0x")
                && moved[0].contains(" in runRound+0x")
                && moved[0]
                    .ends_with("; set x0 = 0x0000000000000000 (0), x1 = 0x0000000000000007 (7)."),
            "{moved:?}"
        );
        session.send("list reg x0 x1\n");
        assert_eq!(
            table_rows(&session.until_prompt(), REGISTER_COLUMNS),
            [
                ["x0", "0x0000000000000000", "(0)"],
                ["x1", "0x0000000000000007", "(7)"]
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
        assert!(function[1].contains(" sub    sp, sp, #0x"), "{function:?}");
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
        session.send("list reg x0 sp\n");
        let registers = table_rows(&session.until_prompt(), REGISTER_COLUMNS);
        assert!(
            registers.len() == 2
                && registers[0][0] == "x0"
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
                && returned[0].contains(": x0 = 0x0000000000000000 (0), x1 = 0x"),
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
}
