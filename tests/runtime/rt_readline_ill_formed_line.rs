//! bug-891: `io::readLine` / `io::input` consume a whole ill-formed line and raise
//! `ErrEncoding` once; the next call starts at the next line (as `fs::readLine` does).
//! Before the fix the error fired at the bad byte and the tail of the line came back
//! as a separate, error-free line (and a bad byte before the LF produced a phantom
//! empty line, one error per bad byte).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::{build_project, run_pty_prompt_interaction_echo_off, run_with_stdin, temp_project};

const READ_LINE: &str = "IMPORT io\n\nFUNC one() AS Boolean\n  io::print(\"[\" & io::readLine() & \"]\")\n  RETURN TRUE\n  TRAP(err)\n    io::print(\"ERR \" & toString(err.code))\n    RETURN err.code <> 77020003\n  END TRAP\nEND FUNC\n\nFUNC main AS Integer\n  WHILE one()\n  END WHILE\n  RETURN 0\nEND FUNC\n";

const INPUT: &str = "IMPORT io\n\nFUNC one() AS Boolean\n  io::print(\"[\" & io::input(\"\") & \"]\")\n  RETURN TRUE\n  TRAP(err)\n    io::print(\"ERR \" & toString(err.code))\n    RETURN err.code <> 77020003\n  END TRAP\nEND FUNC\n\nFUNC main AS Integer\n  WHILE one()\n  END WHILE\n  RETURN 0\nEND FUNC\n";

fn lines(out: &str) -> Vec<&str> {
    out.lines().collect()
}

fn check(name: &str, source: &str) {
    let exe = build_project(&temp_project(name, source));
    let out = run_with_stdin(&exe, b"ok\nab\xffcd\nnext\n");
    assert_eq!(
        lines(&out),
        ["[ok]", "ERR 77020004", "[next]", "ERR 77020003"],
        "{name}: tail of the bad line must not come back as a line"
    );
    let out = run_with_stdin(&exe, b"\xc0\xaf\nx\n");
    assert_eq!(
        lines(&out),
        ["ERR 77020004", "[x]", "ERR 77020003"],
        "{name}: one error per bad line, no phantom empty line"
    );
    // A sequence cut short by the LF, a bad line at end of input, and a lead byte
    // that a bad continuation byte starts afresh.
    let out = run_with_stdin(&exe, b"a\xe2\n\xe2\x82\xac\nz\xff");
    assert_eq!(
        lines(&out),
        ["ERR 77020004", "[\u{20ac}]", "ERR 77020004", "ERR 77020003"],
        "{name}: truncated sequence, valid scalar line, final bad line"
    );
}

#[test]
fn read_line_consumes_the_whole_ill_formed_line() {
    check("readline_ill_formed", READ_LINE);
}

#[test]
fn input_consumes_the_whole_ill_formed_line() {
    check("input_ill_formed", INPUT);
}

#[test]
fn read_line_on_a_terminal_consumes_the_whole_ill_formed_line() {
    let source = "IMPORT io\n\nFUNC one() AS Boolean\n  io::print(\"[\" & io::readLine() & \"]\")\n  RETURN TRUE\n  TRAP(err)\n    io::print(\"ERR \" & toString(err.code))\n    RETURN err.code <> 77020003\n  END TRAP\nEND FUNC\n\nFUNC main AS Integer\n  io::write(\"go> \")\n  IF one() THEN\n    io::write(\"\")\n  END IF\n  IF one() THEN\n    io::write(\"\")\n  END IF\n  RETURN 0\nEND FUNC\n";
    let exe = build_project(&temp_project("readline_ill_formed_pty", source));
    let transcript = run_pty_prompt_interaction_echo_off(&exe, "go> ", b"ab\xffcd\nnext\n");
    let normalized = transcript.replace("\r\n", "\n");
    assert!(
        normalized.contains("ERR 77020004\n[next]\n"),
        "terminal source: {normalized:?}"
    );
    assert!(
        !normalized.contains("[cd]"),
        "terminal source: {normalized:?}"
    );
}
