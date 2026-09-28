//! bug-761: `compress::deflate` at `-O3` raised `ErrIndexOutOfRange` on a one-byte input.
//!
//! The check-elision rows (`optimizer::opt2::checks`) read their facts from the range
//! lattice's optimistic worklist fixpoint (`plans::ranges::global_ranges`). On
//! `__compress_deflateCore` — tens of thousands of SSA values — the fixpoint ran out of budget
//! with `maxLen = n - pos` still unevaluated, while the phi after `IF maxLen > 258 THEN maxLen =
//! 258` had already hulled only its `258` arm. The truncated `[258, 258]` decided `IF maxLen >= 3`
//! true, so a one-byte input read `data[pos + 2]` past its end. The fix pessimizes every value a
//! stopped fixpoint had not settled (`plans::ranges::pessimize_unsettled`).
//!
//! The program round-trips every level 0-9 through each encoder at `-O3` and prints the encoded
//! length, so the output must equal the `-O1` run byte for byte.

#[path = "../common/mod.rs"]
mod common;

const PROGRAM: &str = r#"IMPORT io
IMPORT compress
IMPORT strings
IMPORT collections

FUNC same(a AS List OF Byte, b AS List OF Byte) AS Boolean
  IF len(a) <> len(b) THEN
    RETURN FALSE
  END IF
  MUT i AS Integer = 0
  WHILE i < len(a)
    IF collections::get(a, i) <> collections::get(b, i) THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC

FUNC check(label AS String, data AS List OF Byte) AS Nothing
  MUT level AS Integer = 0
  WHILE level <= 9
    LET packed AS List OF Byte = compress::deflate(data, level)
    LET back AS List OF Byte = compress::inflate(packed)
    LET z AS List OF Byte = compress::zlibDecode(compress::zlibEncode(data, level))
    LET g AS List OF Byte = compress::gzipDecode(compress::gzipEncode(data, level))
    io::print(label & " L" & toString(level) & " " & toString(len(packed)) & " " & toString(same(back, data) AND same(z, data) AND same(g, data)))
    level = level + 1
  END WHILE
END FUNC

FUNC main() AS Nothing
  check("empty", strings::toBytes(""))
  check("a", strings::toBytes("a"))
  check("ab", strings::toBytes("ab"))
  check("abc", strings::toBytes("abc"))
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 300
    s = s & "hello world " & toString(i MOD 7)
    i = i + 1
  END WHILE
  check("text", strings::toBytes(s))
END FUNC
"#;

fn run(level: &str) -> String {
    let project = common::temp_project(&format!("compress_deflate_o{level}"), PROGRAM);
    let binary = common::build_project_opt(&project, level);
    let output = std::process::Command::new(&binary)
        .output()
        .expect("run the compress program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "-O{level} run failed ({}):\nstdout:\n{stdout}\nstderr:\n{}",
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
fn every_encoder_round_trips_at_o3_exactly_as_at_o1() {
    let o3 = run("3");
    assert!(o3.contains("a L1 3 TRUE\n"), "the bug-761 input:\n{o3}");
    assert_eq!(o3.lines().count(), 50, "five inputs x ten levels:\n{o3}");
    assert!(
        o3.lines().all(|line| line.ends_with(" TRUE")),
        "every level round-trips through inflate, zlibDecode and gzipDecode:\n{o3}"
    );
    assert_eq!(o3, run("1"), "-O3 and -O1 encode identically");
}
