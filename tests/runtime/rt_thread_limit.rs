//! `options.maxThreads` limits how many threads a program holds at once (plan-183-B).
//!
//! Each `thread::start` takes two slices of the startup arena reservation — its
//! worker's arena and its control block's arena — and gives them back when the thread
//! has finished and its `Thread` value has been dropped. With every slice taken, a
//! start fails with `ErrThreadLimit` before it creates anything, and the message names
//! the build's limit.
//!
//! Set `MFB_TEST_REMOTE` to run the program on another box (`tests/common/remote.rs`).

#[path = "../common/mod.rs"]
mod common;

const PROGRAM: &str = r#"IMPORT io
IMPORT thread
IMPORT errorCode

ISOLATED FUNC hold(w AS ThreadWorker OF String TO Integer, seed AS Integer) AS Integer
  LET go AS String = thread::receive(w)
  RETURN seed + len(go)
END FUNC

ISOLATED FUNC quick(w AS ThreadWorker OF String TO Integer, seed AS Integer) AS Integer
  RETURN seed * 10
END FUNC

FUNC startAndWait(seed AS Integer) AS Integer
  RES t AS Thread OF String TO Integer = thread::start(quick, seed)
  RETURN thread::waitFor(t)
END FUNC

FUNC attempt(seed AS Integer) AS Integer
  LET r AS Integer = startAndWait(seed) TRAP(e)
    IF e.code = errorCode::ErrThreadLimit THEN
      io::print("limit: " & e.message)
      RECOVER -1
    END IF
    io::print("other: " & e.message)
    RECOVER -2
  END TRAP
  RETURN r
END FUNC

' Starts a second held thread beside main's, tries a third, then lets the second
' finish; its handle is dropped when this function returns.
FUNC withSecond() AS Integer
  RES a AS Thread OF String TO Integer = thread::start(hold, 1)
  io::print("third: " & toString(attempt(3)))
  thread::send(a, "go")
  RETURN thread::waitFor(a)
END FUNC

FUNC main AS Integer
  RES b AS Thread OF String TO Integer = thread::start(hold, 2)
  io::print("second: " & toString(withSecond()))
  io::print("after: " & toString(attempt(4)))
  thread::send(b, "go")
  io::print("first: " & toString(thread::waitFor(b)))
  RETURN 0
END FUNC
"#;

fn project(name: &str, max_threads: Option<u64>) -> std::path::PathBuf {
    let root = common::temp_project(name, PROGRAM);
    let options = match max_threads {
        Some(n) => format!(",\"options\":{{\"maxThreads\":{n}}}"),
        None => String::new(),
    };
    std::fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"entry\":\"main\",\"targets\":[\"native\"]{options}}}\n"
        ),
    )
    .expect("write project.json");
    root
}

fn run(name: &str, max_threads: Option<u64>) -> String {
    let root = project(name, max_threads);
    let exe = common::debug_report::build_debug_project(name, &root);
    let output = common::remote::output(&exe, &[], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name}: {}\n{stdout}\n{}",
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
    stdout
}

/// Two held threads fill `maxThreads: 2`; the third start fails with the limit and
/// creates nothing; once one has finished and its handle is gone, a start succeeds.
#[test]
fn a_start_past_max_threads_fails_until_a_thread_is_released() {
    let stdout = run("thread_limit_two", Some(2));
    assert_eq!(
        stdout,
        "limit: 2 threads are already running; raise options.maxThreads in project.json to allow more\n\
         third: -1\n\
         second: 3\n\
         after: 40\n\
         first: 4\n"
    );
}

/// The default of 10 leaves room for the same program's three.
#[test]
fn the_default_limit_admits_three_threads() {
    let stdout = run("thread_limit_default", None);
    assert_eq!(stdout, "third: 30\nsecond: 3\nafter: 40\nfirst: 4\n");
}
