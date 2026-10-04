//! `mfb dbg <binary> [-- <args>...]` and `mfb dbg <pid>`: start or attach
//! the native debugger (`crate::debugger`) and hand the paused program to the
//! `DBG>` prompt.
//!
//! The operand is an executable when a file by that name exists, and otherwise
//! a process ID when it is all digits. Either way the program must be an
//! MFBASIC executable built for this host; anything else is refused before the
//! debugger touches a process.

use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use crate::cli::help::DBG_HELP;
use crate::cli::CommandError;
use crate::debugger::{check_debuggable, host_backend, prompt, shown, Backend, Inferior};

#[cfg(test)]
#[path = "dbg/tests.rs"]
mod tests;

/// What the command line asks the debugger to do.
#[derive(Debug, PartialEq, Eq)]
enum Request {
    Launch {
        program: PathBuf,
        args: Vec<OsString>,
    },
    Attach {
        pid: u32,
    },
}

/// Read the operand and any program arguments. `--` ends the debugger's own
/// arguments; everything after it goes to a launched program unchanged.
fn parse(args: &[String]) -> Result<Request, CommandError> {
    let (own, program_args) = match args.iter().position(|arg| arg == "--") {
        Some(split) => (&args[..split], Some(&args[split + 1..])),
        None => (args, None),
    };
    let operand = match own {
        [operand] => operand,
        [] => {
            return Err(CommandError::Usage(format!(
                "mfb dbg requires a <binary> or a <pid>\n\n{DBG_HELP}"
            )))
        }
        _ => {
            return Err(CommandError::Usage(format!(
            "mfb dbg accepts one <binary> or <pid>; pass program arguments after --\n\n{DBG_HELP}"
        )))
        }
    };
    let program = PathBuf::from(operand);
    if program.exists() {
        let args = program_args
            .unwrap_or_default()
            .iter()
            .map(OsString::from)
            .collect();
        return Ok(Request::Launch { program, args });
    }
    if operand.starts_with('-') {
        return Err(CommandError::Usage(format!(
            "mfb dbg got unknown option '{operand}'\n\n{DBG_HELP}"
        )));
    }
    if !operand.is_empty() && operand.bytes().all(|byte| byte.is_ascii_digit()) {
        if program_args.is_some() {
            return Err(CommandError::Usage(format!(
                "program arguments after -- need a <binary>, not a <pid>\n\n{DBG_HELP}"
            )));
        }
        let pid = operand
            .parse::<u32>()
            .ok()
            .filter(|pid| *pid != 0)
            .ok_or_else(|| CommandError::Failed(format!("no process with PID {operand}")))?;
        return Ok(Request::Attach { pid });
    }
    Err(CommandError::Failed(format!(
        "'{}' is neither a file nor a process ID",
        shown(&program)
    )))
}

/// `mfb dbg`. Exits `0` when the session ends with `quit` or end of input.
pub(crate) fn run_dbg_command(args: &[String]) -> Result<(), CommandError> {
    let request = parse(args)?;
    let Some(backend) = host_backend() else {
        return Err(CommandError::Failed(format!(
            "mfb dbg is not supported on {}",
            std::env::consts::OS
        )));
    };
    let stdin = std::io::stdin();
    let echo = !stdin.is_terminal();
    session(
        request,
        backend,
        &mut stdin.lock(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
        echo,
    )
}

/// Check the program, start or attach to it through `backend`, and run the
/// prompt over it until the session ends.
fn session(
    request: Request,
    backend: &dyn Backend,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    echo: bool,
) -> Result<(), CommandError> {
    let (mut inferior, banner, facts) = match request {
        Request::Launch { program, args } => {
            let facts = check_debuggable(&program).map_err(CommandError::Failed)?;
            let inferior = backend
                .launch(&program, &args)
                .map_err(CommandError::Failed)?;
            let banner = format!(
                "Launched {} as process {}; paused before its first instruction.",
                shown(&program),
                inferior.pid()
            );
            (inferior, banner, facts)
        }
        Request::Attach { pid } => {
            let program = backend.executable_of(pid).map_err(CommandError::Failed)?;
            let facts = check_debuggable(&program).map_err(CommandError::Failed)?;
            let inferior = backend
                .attach(pid, &program)
                .map_err(CommandError::Failed)?;
            let banner = format!("Attached to process {pid} ({}); paused.", shown(&program));
            (inferior, banner, facts)
        }
    };
    writeln!(out, "{banner}").map_err(|e| CommandError::Failed(format!("cannot write: {e}")))?;
    if let Some(note) = debug_build_note(inferior.as_mut()) {
        writeln!(out, "{note}").map_err(|e| CommandError::Failed(format!("cannot write: {e}")))?;
    }
    prompt::run(inferior.as_mut(), facts, input, out, err, echo).map_err(CommandError::Failed)
}

/// The note printed after the banner when the program carries no function
/// names: a build without `--debug` defines no symbols, only its imports. A
/// symbol table that cannot be read says nothing here; `list func` reports it.
fn debug_build_note(inferior: &mut dyn Inferior) -> Option<String> {
    let functions = inferior.functions().ok()?;
    if functions.iter().any(|function| !function.linked) {
        return None;
    }
    Some(
        "note: this is not a debug build, so the program's function names are missing.\n      \
         Rebuild it with: mfb build --debug -O0 <project>"
            .to_string(),
    )
}
