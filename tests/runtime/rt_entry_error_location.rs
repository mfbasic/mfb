#[path = "../common/mod.rs"]
mod common;

fn run(name: &str, source: &str) -> (i32, String, String) {
    let project = common::temp_project(name, source);
    let binary = common::build_project(&project);
    common::run_capture_with_env(&binary, &[])
}

#[test]
fn uncaught_error_prints_origin_on_stderr() {
    let (code, stdout, stderr) = run(
        "entry_location",
        "FUNC main AS Integer\n  FAIL error(42, \"boom\")\nEND FUNC\n",
    );
    assert_eq!(code, 255);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "Thread:   0x00000000\nError:    42\nMessage:  boom\nLocation: src/main.mfb:2:8\n\n"
    );
}

#[test]
fn propagated_error_keeps_original_location() {
    let (code, stdout, stderr) = run("entry_propagated_location", "FUNC boom AS Integer\n  FAIL error(42, \"original\")\nEND FUNC\nFUNC main AS Integer\n  RETURN boom()\nTRAP(e)\n  PROPAGATE\nEND TRAP\nEND FUNC\n");
    assert_eq!(code, 255);
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "Thread:   0x00000000\nError:    42\nMessage:  original\nLocation: src/main.mfb:2:8\n\n"
    );
}

#[test]
fn entry_range_error_has_no_fabricated_location() {
    let (code, stdout, stderr) = run(
        "entry_no_location",
        "FUNC main AS Integer\n  RETURN 256\nEND FUNC\n",
    );
    assert_eq!(code, 255);
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with("Thread:   0x00000000\nError:    7-705-0010\nMessage:  "),
        "{stderr}"
    );
    assert!(!stderr.contains("Location:"), "{stderr}");
}

#[test]
fn worker_and_main_failures_are_both_reported() {
    let (code, stdout, stderr) = run(
        "worker_then_main_error",
        r#"IMPORT thread
ISOLATED FUNC worker(w AS ThreadWorker OF String TO Integer, seed AS Integer) AS Integer
  FAIL error(77050002, "worker failed")
END FUNC
FUNC main AS Integer
  RES t AS Thread OF String TO Integer = thread::start(worker, 0)
  LET n = thread::waitFor(t) TRAP(e)
    RECOVER 0
  END TRAP
  thread::send(t, "gem")
  RETURN n
END FUNC
"#,
    );
    assert_eq!(code, 255);
    assert_eq!(stdout, "");
    let reports: Vec<_> = stderr.trim_end().split("\n\n").collect();
    assert_eq!(reports.len(), 2, "{stderr}");
    let worker_id = reports[0]
        .lines()
        .next()
        .unwrap()
        .strip_prefix("Thread:   0x")
        .unwrap();
    assert!(worker_id.len() >= 8);
    assert_ne!(u64::from_str_radix(worker_id, 16).unwrap(), 0);
    assert!(
        reports[0].contains(
            "\nError:    7-705-0002\nMessage:  worker failed\nLocation: src/main.mfb:3:8"
        ),
        "{stderr}"
    );
    assert!(
        reports[1].starts_with("Thread:   0x00000000\nError:    7-705-0009\nMessage:  "),
        "{stderr}"
    );
    assert!(
        reports[1].ends_with("Location: src/main.mfb:10:3"),
        "{stderr}"
    );
}

#[test]
fn handled_errors_and_successful_threads_are_silent() {
    let (code, stdout, stderr) = run(
        "handled_worker_error",
        r#"IMPORT thread
ISOLATED FUNC worker(w AS ThreadWorker OF Integer TO Integer, seed AS Integer) AS Integer
  FAIL error(42, "handled")
TRAP(e)
  RETURN 0
END TRAP
END FUNC
FUNC main AS Integer
  RES t AS Thread OF Integer TO Integer = thread::start(worker, 0)
  RETURN thread::waitFor(t)
END FUNC
"#,
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
}

#[test]
fn failed_worker_is_reported_even_when_parent_handles_result() {
    let (code, stdout, stderr) = run(
        "handled_worker_result",
        r#"IMPORT thread
ISOLATED FUNC worker(w AS ThreadWorker OF Integer TO Integer, seed AS Integer) AS Integer
  FAIL error(42, "worker failed")
END FUNC
FUNC main AS Integer
  RES t AS Thread OF Integer TO Integer = thread::start(worker, 0)
  LET n = thread::waitFor(t) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN n
END FUNC
"#,
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr.matches("Thread:").count(), 1, "{stderr}");
    assert!(
        stderr.contains("Message:  worker failed\nLocation: src/main.mfb:3:8"),
        "{stderr}"
    );
}

#[test]
fn concurrent_worker_reports_keep_their_fields_together() {
    let (code, stdout, stderr) = run(
        "concurrent_worker_errors",
        r#"IMPORT thread
ISOLATED FUNC worker(w AS ThreadWorker OF Integer TO Integer, seed AS Integer) AS Integer
  LET go = thread::receive(w)
  FAIL error(seed, "worker failed")
END FUNC
FUNC main AS Integer
  RES a AS Thread OF Integer TO Integer = thread::start(worker, 41)
  RES b AS Thread OF Integer TO Integer = thread::start(worker, 42)
  thread::send(a, 1)
  thread::send(b, 1)
  LET first = thread::waitFor(a) TRAP(e)
    RECOVER 0
  END TRAP
  LET second = thread::waitFor(b) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN first + second
END FUNC
"#,
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    let reports: Vec<_> = stderr.trim_end().split("\n\n").collect();
    assert_eq!(reports.len(), 2, "{stderr}");
    let mut identities = Vec::new();
    let mut codes = Vec::new();
    for report in reports {
        let lines: Vec<_> = report.lines().collect();
        assert_eq!(lines.len(), 4, "{stderr}");
        identities
            .push(u64::from_str_radix(lines[0].strip_prefix("Thread:   0x").unwrap(), 16).unwrap());
        codes.push(lines[1]);
        assert_eq!(lines[2], "Message:  worker failed");
        assert_eq!(lines[3], "Location: src/main.mfb:4:8");
    }
    assert!(identities.iter().all(|id| *id != 0));
    assert_ne!(identities[0], identities[1]);
    codes.sort();
    assert_eq!(codes, ["Error:    41", "Error:    42"]);
}
