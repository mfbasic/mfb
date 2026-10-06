//! bug-841: `EXIT PROGRAM` is not catchable by an inline `TRAP`.
//!
//! `mfb spec language control-flow`: "`EXIT PROGRAM` is not catchable by `TRAP`: at
//! each returning frame it bypasses any handler, so it propagates past every `TRAP`
//! to the entry point." A callee's `EXIT PROGRAM n` comes back as a `Result` with
//! tag `2` (`RESULT_PROGRAM_EXIT_TAG`), the exit code in the value register and no
//! message. The inline-`TRAP` lowerings handed that tag to the trapped-`Error`
//! rebuild, which read the zeroed message register as a `String` pointer: every
//! form below died with SIGSEGV (exit 139) at every optimization level instead of
//! exiting 7. The function-level `TRAP` route already re-propagated the tag.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Each case is a whole program that must exit 7 with no output.
const CASES: &[(&str, &str)] = &[
    (
        "direct_let",
        "\
FUNC bye() AS Integer
  EXIT PROGRAM 7
END FUNC
FUNC main() AS Integer
  LET v = bye() TRAP
    RECOVER 1
  END TRAP
  RETURN v
END FUNC
",
    ),
    (
        "direct_nothing_statement",
        "\
FUNC bye() AS Nothing
  EXIT PROGRAM 7
END FUNC
FUNC main() AS Integer
  bye() TRAP
    RECOVER
  END TRAP
  RETURN 1
END FUNC
",
    ),
    (
        "error_read_by_handler",
        "\
IMPORT io
FUNC bye() AS Integer
  EXIT PROGRAM 7
END FUNC
FUNC main() AS Integer
  LET v = bye() TRAP(e)
    io::print(e.message)
    RECOVER 1
  END TRAP
  RETURN v
END FUNC
",
    ),
    (
        "string_result",
        "\
IMPORT io
FUNC bye(n AS Integer) AS String
  IF n > 0 THEN EXIT PROGRAM 7
  RETURN \"no\"
END FUNC
FUNC main() AS Integer
  LET s AS String = bye(1) TRAP
    RECOVER \"handler\"
  END TRAP
  io::print(s)
  RETURN 1
END FUNC
",
    ),
    (
        "one_frame_up",
        "\
FUNC bye() AS Integer
  EXIT PROGRAM 7
END FUNC
FUNC mid() AS Integer
  LET v = bye() TRAP
    RECOVER 1
  END TRAP
  RETURN v
END FUNC
FUNC main() AS Integer
  RETURN mid()
END FUNC
",
    ),
    (
        "inline_trap_inside_function_trap",
        "\
IMPORT io
FUNC bye() AS Integer
  EXIT PROGRAM 7
END FUNC
FUNC mid() AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  LET v = bye() TRAP
    RECOVER len(xs)
  END TRAP
  RETURN v
  TRAP(e)
    io::print(\"function trap\")
    RETURN 2
  END TRAP
END FUNC
FUNC main() AS Integer
  RETURN mid()
END FUNC
",
    ),
    (
        "indirect_func_value",
        "\
FUNC bye(n AS Integer) AS Integer
  EXIT PROGRAM 7
END FUNC
FUNC main() AS Integer
  LET g = bye
  LET v = g(1) TRAP
    RECOVER 1
  END TRAP
  RETURN v
END FUNC
",
    ),
    (
        "builtin_hof_callback",
        "\
IMPORT collections
FUNC bye(n AS Integer) AS Integer
  IF n = 2 THEN EXIT PROGRAM 7
  RETURN n
END FUNC
FUNC main() AS Integer
  LET v = collections::transform([1, 2, 3], bye) TRAP
    RECOVER []
  END TRAP
  RETURN len(v)
END FUNC
",
    ),
];

#[test]
fn exit_program_bypasses_every_inline_trap_form() {
    let mut failures = Vec::new();
    for (name, source) in CASES {
        let project = common::temp_project(&format!("exit_inline_trap_{name}"), source);
        for level in ["0", "3"] {
            let exe = common::build_project_opt(&project, level);
            let output = Command::new(&exe).output().expect("run program");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if output.status.code() != Some(7) || !stdout.is_empty() || !stderr.is_empty() {
                failures.push(format!(
                    "{name} -O{level}: want exit 7 and no output, got {} stdout {stdout:?} stderr {stderr:?}",
                    common::exit_description(&output.status)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
