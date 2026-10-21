//! plan-176-E: `cli_dbg`'s native-debugger support (the signed `mfb`, `Session`, the
//! debug builds and the table readers, `pub(super)` for `threads_watch_and_run.rs`),
//! then the launch, attach and `list` tests. Declared from the root, `cli_dbg.rs`, under
//! the native-debugger `cfg`.

use super::*;

/// The `mfb` the sessions run: on Linux the one cargo built, which needs
/// nothing more.
#[cfg(target_os = "linux")]
pub(super) fn signed_mfb() -> PathBuf {
    PathBuf::from(mfb_exe())
}

/// A copy of `mfb` signed for debugging, in a directory of its own.
#[cfg(target_os = "macos")]
pub(super) fn signed_mfb() -> PathBuf {
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
pub(super) struct Session {
    pub(super) child: Child,
    chunks: mpsc::Receiver<Vec<u8>>,
    seen: String,
}

impl Session {
    pub(super) fn start(mfb: &Path, args: &[&str]) -> Self {
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
    pub(super) fn until_prompt(&mut self) -> String {
        while !self.seen.ends_with("DBG> ") {
            let chunk = self
                .chunks
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("no DBG> prompt; output so far:\n{}", self.seen));
            self.seen.push_str(&String::from_utf8_lossy(&chunk));
        }
        std::mem::take(&mut self.seen)
    }

    pub(super) fn send(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// Close stdin and collect the exit code and the rest of stdout.
    pub(super) fn finish(mut self) -> (Option<i32>, String) {
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
pub(super) fn image() -> String {
    executable_name("debug")
}

/// How the refusal of a foreign program names this host's programs.
fn host_image() -> &'static str {
    if cfg!(target_os = "macos") {
        "Mach-O aarch64"
    } else if cfg!(target_arch = "riscv64") {
        "ELF riscv64"
    } else {
        "ELF aarch64"
    }
}

/// Whether this system lets a debugger attach to a program it did not
/// start. When it does not, the attaching tests check that `mfb dbg`
/// says so and why.
pub(super) fn attach_allowed(mfb: &Path, pid: u32) -> bool {
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
pub(super) const TICKER: &str =
    "IMPORT io\nIMPORT os\n\nFUNC next_tick(i AS Integer) AS Integer\n  \
        io::print(\"tick\")\n  os::sleep(100)\n  RETURN i + 1\nEND FUNC\n\n\
        FUNC main() AS Nothing\n  MUT i AS Integer = 0\n  WHILE i < 1500\n    \
        i = next_tick(i)\n  END WHILE\nEND FUNC\n";

/// Project `name` built with `--debug -O0`, so it carries its function
/// names.
pub(super) fn build_debug(project: &Path, name: &str) -> PathBuf {
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

pub(super) fn is_address(word: &str) -> bool {
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
    assert_eq!(names, super::cpu::names(), "{registers:?}");
    // General registers print 16 hex digits, AArch64's vector registers 32.
    for words in &lines {
        let digits = super::cpu::digits(words[0]);
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
    assert_eq!(roles, super::cpu::ROLES, "{common}");
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
pub(super) fn debug_example() -> PathBuf {
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

/// How a `--debug` build that exits with status 0 ends: its report is
/// kept for `show report`.
pub(super) const EXITED: &str =
    "--[EX] Program exit (0) - type \"show report\" to see the performance and memory report";

/// The lines between a command's echo and the next prompt.
pub(super) fn body(reply: &str) -> Vec<String> {
    reply
        .lines()
        .skip(1)
        .take_while(|line| !line.starts_with("DBG>"))
        .map(str::to_string)
        .collect()
}

/// The columns of each listing's header line.
const FUNCTION_COLUMNS: &[&str] = &["Type", "Address", "Symbol", "Written name"];

pub(super) const REGISTER_COLUMNS: &[&str] = &["Register", "Hex", "Decimal"];

/// What separates a listing's columns.
const SEPARATOR: &str = " | ";

/// The cells of each row of a listing reply, after checking that its
/// first line is the header naming `columns`, separated by ` | `, and that
/// every row has a cell for each, its separators under the header's.
pub(super) fn table_rows(reply: &str, columns: &[&str]) -> Vec<Vec<String>> {
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
