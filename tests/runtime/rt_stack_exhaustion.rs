//! bug-789: unbounded recursion raises `ErrStackOverflow` (7-701-0002) instead of
//! dying by SIGSEGV on the host stack guard.
//!
//! Every function checks, at entry, the stack pointer against a per-thread limit a
//! reserve above the thread's real stack bottom, and raises the error there — an
//! ordinary failure the caller's `TRAP` catches and every frame's cleanup runs for.
//! Covered: direct and mutual recursion, recursion through a function value, a
//! worker thread, a `TRAP` that recovers and keeps running, and shallow recursion
//! that must still return its value. Each program runs as a subprocess with a
//! timeout; a signal death is a failure, never a pass.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;
use std::time::Duration;

fn run(name: &str, source: &str) -> (Option<i32>, String, String) {
    let project = common::temp_project(name, source);
    let executable = common::build_project(&project);
    let mut child = Command::new(&executable)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the probe");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait().expect("poll the probe").is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("{name} did not finish within 60 s");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().expect("collect the probe");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            output.status.signal(),
            None,
            "{name} died by a signal:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

const RECUR: &str = r#"
FUNC recur(n AS Integer) AS Integer
  IF n = 0 THEN
    RETURN 0
  END IF
  RETURN recur(n - 1) + 1
END FUNC
"#;

#[test]
fn shallow_recursion_still_returns_its_value() {
    let source = format!(
        "IMPORT io\n{RECUR}\nFUNC main() AS Integer\n  io::print(toString(recur(10000)))\n  RETURN 0\nEND FUNC\n"
    );
    let (code, stdout, stderr) = run("b789_shallow", &source);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout.trim(), "10000");
}

#[test]
fn unbounded_recursion_raises_stack_overflow_unhandled() {
    let source = format!("{RECUR}\nFUNC main() AS Integer\n  RETURN recur(100000000)\nEND FUNC\n");
    let (code, _stdout, stderr) = run("b789_unhandled", &source);
    assert_ne!(code, Some(0));
    assert!(stderr.contains("7-701-0002"), "stderr:\n{stderr}");
}

#[test]
fn a_trap_recovers_from_stack_overflow_and_keeps_running() {
    let source = format!(
        "IMPORT io\n{RECUR}\nFUNC main() AS Integer\n  \
         FOR attempt = 1 TO 3\n    \
           LET r = recur(100000000) TRAP(e)\n      \
             io::print(\"caught \" & toString(e.code))\n      \
             RECOVER -1\n    \
           END TRAP\n  \
         NEXT\n  \
         io::print(toString(recur(100)))\n  \
         RETURN 0\nEND FUNC\n"
    );
    let (code, stdout, stderr) = run("b789_trap", &source);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "caught 77010002",
            "caught 77010002",
            "caught 77010002",
            "100"
        ]
    );
}

#[test]
fn mutual_recursion_raises_stack_overflow() {
    let source = "IMPORT io\n\
FUNC ping(n AS Integer) AS Integer\n  IF n = 0 THEN\n    RETURN 0\n  END IF\n  RETURN pong(n - 1) + 1\nEND FUNC\n\
FUNC pong(n AS Integer) AS Integer\n  IF n = 0 THEN\n    RETURN 0\n  END IF\n  RETURN ping(n - 1) + 1\nEND FUNC\n\
FUNC main() AS Integer\n  LET r = ping(100000000) TRAP(e)\n    io::print(toString(e.code))\n    RECOVER 0\n  END TRAP\n  RETURN 0\nEND FUNC\n";
    let (code, stdout, stderr) = run("b789_mutual", source);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout.trim(), "77010002");
}

#[test]
fn recursion_through_a_function_value_raises_stack_overflow() {
    let source = "IMPORT io\n\
FUNC apply(f AS FUNC(Integer) AS Integer, n AS Integer) AS Integer\n  RETURN f(n)\nEND FUNC\n\
FUNC down(n AS Integer) AS Integer\n  IF n = 0 THEN\n    RETURN 0\n  END IF\n  RETURN apply(down, n - 1) + 1\nEND FUNC\n\
FUNC main() AS Integer\n  LET r = down(100000000) TRAP(e)\n    io::print(toString(e.code))\n    RECOVER 0\n  END TRAP\n  RETURN 0\nEND FUNC\n";
    let (code, stdout, stderr) = run("b789_callback", source);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout.trim(), "77010002");
}

#[test]
fn a_worker_thread_raises_stack_overflow() {
    let source = format!(
        "IMPORT io\nIMPORT thread\n{RECUR}\n\
ISOLATED FUNC worker(t AS ThreadWorker OF Nothing TO Integer, n AS Integer) AS Integer\n  \
LET r = recur(n) TRAP(e)\n    RECOVER e.code\n  END TRAP\n  RETURN r\nEND FUNC\n\
FUNC main() AS Integer\n  RES t AS Thread OF Nothing TO Integer = thread::start(worker, 100000000)\n  \
io::print(toString(thread::waitFor(t)))\n  RETURN 0\nEND FUNC\n"
    );
    let (code, stdout, stderr) = run("b789_worker", &source);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout.trim(), "77010002");
}

/// The raise belongs to the call that could not be entered: the overflowing
/// function's own function-level `TRAP` is not yet in force at its entry, so the
/// caller's frame (here the same function one level up) catches it and every frame
/// above still returns normally.
#[test]
fn a_function_level_trap_catches_the_overflow_of_the_call_below() {
    let source = "IMPORT io\n\
FUNC deep(n AS Integer) AS Integer\n  RETURN deep(n + 1) + 1\n  TRAP(e)\n    RETURN e.code\n  END TRAP\nEND FUNC\n\
FUNC main() AS Nothing\n  io::print(toString(deep(0) > 77010002))\nEND FUNC\n";
    let (code, stdout, stderr) = run("b789_function_trap", source);
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(stdout.trim(), "TRUE");
}
