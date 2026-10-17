use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{parse, session, Request};
use crate::cli::CommandError;
use crate::debugger::prompt::PROMPT;
use crate::debugger::tests::{linked_probes, native_probe};
use crate::debugger::{
    check_debuggable, Backend, Ending, FunctionRange, FunctionSymbol, Inferior, Register, RunStop,
    Step, ThreadFacts,
};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

/// The message of a refused command line, tagged with its exit class.
fn refusal(values: &[&str]) -> (&'static str, String) {
    match parse(&args(values)) {
        Ok(request) => panic!("{values:?} parsed as {request:?}"),
        Err(CommandError::Usage(message)) => ("usage", message),
        Err(CommandError::Failed(message)) => ("failed", message),
    }
}

/// An existing file: this test binary.
fn existing() -> String {
    std::env::current_exe()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

#[test]
fn an_existing_file_is_launched_with_the_arguments_after_the_separator() {
    let file = existing();
    assert_eq!(
        parse(&args(&[&file])).ok(),
        Some(Request::Launch {
            program: PathBuf::from(&file),
            args: Vec::new()
        })
    );
    assert_eq!(
        parse(&args(&[&file, "--", "-x", "--", "--help"])).ok(),
        Some(Request::Launch {
            program: PathBuf::from(&file),
            args: ["-x", "--", "--help"].map(OsString::from).to_vec()
        })
    );
}

#[test]
fn digits_that_name_no_file_are_a_pid() {
    assert_eq!(
        parse(&args(&["4242"])).ok(),
        Some(Request::Attach { pid: 4242 })
    );
}

#[test]
fn a_pid_takes_no_program_arguments() {
    let (class, message) = refusal(&["4242", "--", "x"]);
    assert_eq!(class, "usage");
    assert!(message.starts_with("program arguments after -- need a <binary>"));
}

#[test]
fn a_pid_out_of_range_names_no_process() {
    for pid in ["0", "99999999999"] {
        assert_eq!(
            refusal(&[pid]),
            ("failed", format!("no process with PID {pid}"))
        );
    }
}

#[test]
fn the_operand_count_is_exactly_one() {
    for line in [&[][..], &["--"][..], &["--", "x"][..], &["a", "b"][..]] {
        assert_eq!(refusal(line).0, "usage", "{line:?}");
    }
}

#[test]
fn an_unknown_option_is_a_usage_error() {
    let (class, message) = refusal(&["-x"]);
    assert_eq!(class, "usage");
    assert!(message.starts_with("mfb dbg got unknown option '-x'"));
}

#[test]
fn a_missing_file_that_is_not_a_pid_fails() {
    assert_eq!(
        refusal(&["no-such-program.out"]),
        (
            "failed",
            "'no-such-program.out' is neither a file nor a process ID".to_string()
        )
    );
}

// --- the session, over a backend that touches no process -------------------

/// A debuggee that ends the way a real one would for how it was reached.
struct Paused {
    ending: Ending,
    symbols: Result<Vec<FunctionSymbol>, String>,
}

impl Inferior for Paused {
    fn pid(&self) -> u32 {
        7
    }
    fn end(&mut self) -> Result<Ending, String> {
        Ok(self.ending)
    }
    fn functions(&mut self) -> Result<Vec<FunctionSymbol>, String> {
        self.symbols.clone()
    }
    fn data_symbol(&mut self, _: &str) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn read_code(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn function_at(&mut self, _: u64) -> Result<Option<FunctionRange>, String> {
        Ok(None)
    }
    fn data_symbol_at(&mut self, _: u64) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn library_functions(&mut self, _: &str) -> Result<Vec<FunctionRange>, String> {
        Ok(Vec::new())
    }
    fn read_bytes(&mut self, address: u64, _: usize) -> Result<Vec<u8>, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn read_u64(&mut self, address: u64) -> Result<u64, String> {
        Err(format!("unmapped {address:#x}"))
    }
    fn threads(&mut self) -> Result<Vec<ThreadFacts>, String> {
        Ok(Vec::new())
    }
    fn registers(&mut self, _: usize) -> Result<Vec<Register>, String> {
        Ok(Vec::new())
    }
    fn set_registers(&mut self, _: usize, _: &[(String, u64)]) -> Result<(), String> {
        Err("the paused program has no threads".to_string())
    }
    fn step(&mut self, _: &[usize], _: u64) -> Result<Vec<Step>, String> {
        Err("not stepped".to_string())
    }
    fn restart(&mut self, _: &[(OsString, OsString)]) -> Result<u32, String> {
        Err("restart needs a program the debugger launched".to_string())
    }
    fn entry(&mut self) -> Result<Option<u64>, String> {
        Ok(None)
    }
    fn insert_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn remove_breakpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn insert_watchpoint(&mut self, _: u64, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn remove_watchpoint(&mut self, _: u64) -> Result<(), String> {
        Ok(())
    }
    fn run(&mut self) -> Result<RunStop, String> {
        Err("not run".to_string())
    }
}

fn symbol(name: &str, linked: bool) -> FunctionSymbol {
    FunctionSymbol {
        name: name.to_string(),
        linked,
        loaded: false,
        address: None,
        size: None,
    }
}

/// A backend whose process 7 runs `executable`, recording every call.
struct Recorded {
    executable: Result<PathBuf, String>,
    refuse: Option<String>,
    /// What the debuggee's `functions` answers; a debug build by default.
    symbols: Result<Vec<FunctionSymbol>, String>,
    calls: RefCell<Vec<String>>,
}

impl Recorded {
    fn running(executable: &Path) -> Self {
        Recorded {
            executable: Ok(executable.to_path_buf()),
            refuse: None,
            symbols: Ok(vec![symbol("_mfb_fn_main", false), symbol("_exit", true)]),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn answer(&self, ending: Ending) -> Result<Box<dyn Inferior>, String> {
        match &self.refuse {
            Some(message) => Err(message.clone()),
            None => Ok(Box::new(Paused {
                ending,
                symbols: self.symbols.clone(),
            })),
        }
    }
}

impl Backend for Recorded {
    fn executable_of(&self, pid: u32) -> Result<PathBuf, String> {
        self.calls.borrow_mut().push(format!("executable_of {pid}"));
        self.executable.clone()
    }
    fn launch(
        &self,
        program: &Path,
        args: &[OsString],
        _: &[(OsString, OsString)],
    ) -> Result<Box<dyn Inferior>, String> {
        self.calls
            .borrow_mut()
            .push(format!("launch {} {args:?}", program.display()));
        self.answer(Ending::Killed)
    }
    fn attach(&self, pid: u32, program: &Path) -> Result<Box<dyn Inferior>, String> {
        self.calls
            .borrow_mut()
            .push(format!("attach {pid} {}", program.display()));
        self.answer(Ending::Detached)
    }
}

/// Run a session over `input`; answer its stdout, or the failure message.
fn run_session(request: Request, backend: &Recorded, input: &str) -> Result<String, String> {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = session(
        request,
        backend,
        &mut input.as_bytes(),
        &mut out,
        &mut err,
        false,
    );
    match result {
        Ok(()) => Ok(String::from_utf8(out).unwrap()),
        Err(CommandError::Failed(message)) => Err(message),
        Err(CommandError::Usage(message)) => panic!("a session is never a usage error: {message}"),
    }
}

#[test]
fn a_launched_program_is_paused_at_the_prompt_and_killed_on_quit() {
    let dir = tempfile::tempdir().unwrap();
    let program = native_probe(dir.path());
    let backend = Recorded::running(&program);
    let out = run_session(
        Request::Launch {
            program: program.clone(),
            args: vec![OsString::from("-x")],
        },
        &backend,
        "q\n",
    )
    .unwrap();
    assert_eq!(
        out,
        format!(
            "Launched {} as process 7; paused before its first instruction.\n\
             {PROMPT}Killed process 7.\n",
            program.display()
        )
    );
    assert_eq!(
        *backend.calls.borrow(),
        [format!("launch {} [\"-x\"]", program.display())]
    );
}

#[test]
fn an_attached_program_is_paused_at_the_prompt_and_detached_on_quit() {
    let dir = tempfile::tempdir().unwrap();
    let program = native_probe(dir.path());
    let backend = Recorded::running(&program);
    let out = run_session(Request::Attach { pid: 7 }, &backend, "").unwrap();
    assert_eq!(
        out,
        format!(
            "Attached to process 7 ({}); paused.\n\
             {PROMPT}\nDetached from process 7; it continues running.\n",
            program.display()
        )
    );
    assert_eq!(
        *backend.calls.borrow(),
        [
            "executable_of 7".to_string(),
            format!("attach 7 {}", program.display())
        ]
    );
}

#[test]
fn a_program_that_is_not_debuggable_is_refused_before_any_process_is_touched() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "hello").unwrap();
    let host = (std::env::consts::OS, std::env::consts::ARCH);
    let (_, _, foreign) = linked_probes(dir.path())
        .into_iter()
        .find(|(os, arch, _)| (*os, *arch) != host)
        .unwrap();
    for program in [text, foreign] {
        let refusal = check_debuggable(&program).unwrap_err();
        let backend = Recorded::running(&program);
        let launched = run_session(
            Request::Launch {
                program: program.clone(),
                args: Vec::new(),
            },
            &backend,
            "q\n",
        );
        assert_eq!(launched, Err(refusal.clone()));
        assert!(backend.calls.borrow().is_empty());

        let attached = run_session(Request::Attach { pid: 7 }, &backend, "q\n");
        assert_eq!(attached, Err(refusal));
        assert_eq!(*backend.calls.borrow(), ["executable_of 7"]);
    }
}

#[test]
fn a_backend_refusal_is_the_command_s_failure() {
    let dir = tempfile::tempdir().unwrap();
    let program = native_probe(dir.path());
    let mut backend = Recorded::running(&program);
    let denied = "cannot attach to process 7: denied".to_string();
    backend.refuse = Some(denied.clone());
    let launched = run_session(
        Request::Launch {
            program,
            args: Vec::new(),
        },
        &backend,
        "q\n",
    );
    assert_eq!(launched, Err(denied.clone()));
    let attached = run_session(Request::Attach { pid: 7 }, &backend, "q\n");
    assert_eq!(attached, Err(denied));

    backend.executable = Err("no process with PID 7".to_string());
    let missing = run_session(Request::Attach { pid: 7 }, &backend, "q\n");
    assert_eq!(missing, Err("no process with PID 7".to_string()));
}

#[test]
fn a_program_without_function_names_is_announced_as_not_a_debug_build() {
    let dir = tempfile::tempdir().unwrap();
    let program = native_probe(dir.path());
    let mut backend = Recorded::running(&program);
    backend.symbols = Ok(vec![symbol("_exit", true), symbol("_malloc", true)]);
    let out = run_session(Request::Attach { pid: 7 }, &backend, "q\n").unwrap();
    assert_eq!(
        out,
        format!(
            "Attached to process 7 ({}); paused.\n\
             note: this is not a debug build, so the program's function names are missing.\n      \
             Rebuild it with: mfb build --debug -O0 <project>\n\
             {PROMPT}Detached from process 7; it continues running.\n",
            program.display()
        )
    );

    // A symbol table that cannot be read is not called a normal build.
    backend.symbols = Err("cannot read".to_string());
    let out = run_session(Request::Attach { pid: 7 }, &backend, "q\n").unwrap();
    assert!(!out.contains("note:"), "{out}");
}

/// `mfb dbg --help` lists the prompt's commands exactly as `help` at the
/// prompt does, so the two cannot drift apart.
#[test]
fn the_dbg_help_lists_every_prompt_command() {
    let commands = crate::debugger::prompt::HELP
        .split_once('\n')
        .expect("a heading line")
        .1;
    assert!(
        crate::cli::help::dbg_help().contains(commands),
        "mfb dbg --help does not carry the prompt's command list"
    );
}
