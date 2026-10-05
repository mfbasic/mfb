//! bug-774: `x = collections::append(x, collections::mid(x, start, count))` on a
//! `MUT` fixed-width list lowers to one in-place copy from the list's own data into
//! its tail, with no `mid` temporary (`lower_list_self_slice_append_in_place`).
//!
//! The values must be exactly the two-step form's. Each list below is grown by a
//! scripted sequence of self-slices — short and long, from the start, the middle and
//! the end, empty ones, and the doubling sequence a DEFLATE back-reference shorter
//! than its length uses — through several grows, and is compared element by element
//! with the same sequence applied to a separate list through `mid` of a snapshot
//! (which takes the ordinary copying path). A slice out of range must raise what
//! `mid` raises, before anything is written. The `-ncode` dump shows the fused path
//! is the one that ran.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, build_project, run_capture_with_env, temp_project};

const PROGRAM: &str = r#"IMPORT io
IMPORT collections

FUNC sameBytes(a AS List OF Byte, b AS List OF Byte) AS Boolean
  IF len(a) <> len(b) THEN
    RETURN FALSE
  END IF
  FOR i = 0 TO len(a) - 1
    IF collections::get(a, i) <> collections::get(b, i) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC

FUNC sameInts(a AS List OF Integer, b AS List OF Integer) AS Boolean
  IF len(a) <> len(b) THEN
    RETURN FALSE
  END IF
  FOR i = 0 TO len(a) - 1
    IF collections::get(a, i) <> collections::get(b, i) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC

FUNC sameFloats(a AS List OF Float, b AS List OF Float) AS Boolean
  IF len(a) <> len(b) THEN
    RETURN FALSE
  END IF
  FOR i = 0 TO len(a) - 1
    IF collections::get(a, i) <> collections::get(b, i) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC

FUNC sameBools(a AS List OF Boolean, b AS List OF Boolean) AS Boolean
  IF len(a) <> len(b) THEN
    RETURN FALSE
  END IF
  FOR i = 0 TO len(a) - 1
    IF collections::get(a, i) <> collections::get(b, i) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC

' The scripted slice for step `k` of a list of length `n`: a start and a count that
' are always in range.
FUNC sliceStart(k AS Integer, n AS Integer) AS Integer
  LET kind AS Integer = k MOD 5
  IF kind = 0 THEN
    RETURN 0
  ELSEIF kind = 1 THEN
    RETURN n / 3
  ELSEIF kind = 2 THEN
    RETURN n
  ELSEIF kind = 3 THEN
    RETURN n - 1
  END IF
  RETURN (k * 7) MOD n
END FUNC

FUNC sliceCount(k AS Integer, n AS Integer, start AS Integer) AS Integer
  LET room AS Integer = n - start
  IF k MOD 4 = 0 THEN
    RETURN room
  ELSEIF k MOD 4 = 1 THEN
    RETURN 0
  END IF
  RETURN (k * 13) MOD (room + 1)
END FUNC

FUNC bytes() AS Boolean
  MUT xs AS List OF Byte = [toByte(1), toByte(2), toByte(3)]
  MUT ys AS List OF Byte = xs
  FOR k = 0 TO 40
    LET start AS Integer = sliceStart(k, len(xs))
    LET count AS Integer = sliceCount(k, len(xs), start)
    xs = collections::append(xs, collections::mid(xs, start, count))
    LET snap AS List OF Byte = ys
    ys = collections::append(ys, collections::mid(snap, start, count))
  NEXT
  ' A back-reference shorter than its length: copy the period, then twice it.
  LET from AS Integer = len(xs) - 3
  MUT left AS Integer = 50
  WHILE left > 0
    MUT chunk AS Integer = len(xs) - from
    IF chunk > left THEN
      chunk = left
    END IF
    xs = collections::append(xs, collections::mid(xs, from, chunk))
    left = left - chunk
  END WHILE
  FOR k = 0 TO 49
    ys = collections::append(ys, collections::get(ys, len(ys) - 3))
  NEXT
  io::print("bytes " & toString(len(xs)))
  RETURN sameBytes(xs, ys)
END FUNC

FUNC ints() AS Boolean
  MUT xs AS List OF Integer = [-5, 9223372036854775807, 0, 42]
  MUT ys AS List OF Integer = xs
  FOR k = 0 TO 30
    LET start AS Integer = sliceStart(k, len(xs))
    LET count AS Integer = sliceCount(k, len(xs), start)
    xs = collections::append(xs, collections::mid(xs, start, count))
    LET snap AS List OF Integer = ys
    ys = collections::append(ys, collections::mid(snap, start, count))
  NEXT
  io::print("ints " & toString(len(xs)))
  RETURN sameInts(xs, ys)
END FUNC

FUNC floats() AS Boolean
  MUT xs AS List OF Float = [1.5, -0.25, 3.0e100]
  MUT ys AS List OF Float = xs
  FOR k = 0 TO 30
    LET start AS Integer = sliceStart(k, len(xs))
    LET count AS Integer = sliceCount(k, len(xs), start)
    xs = collections::append(xs, collections::mid(xs, start, count))
    LET snap AS List OF Float = ys
    ys = collections::append(ys, collections::mid(snap, start, count))
  NEXT
  io::print("floats " & toString(len(xs)))
  RETURN sameFloats(xs, ys)
END FUNC

FUNC bools() AS Boolean
  MUT xs AS List OF Boolean = [TRUE, FALSE, FALSE, TRUE, TRUE]
  MUT ys AS List OF Boolean = xs
  FOR k = 0 TO 30
    LET start AS Integer = sliceStart(k, len(xs))
    LET count AS Integer = sliceCount(k, len(xs), start)
    xs = collections::append(xs, collections::mid(xs, start, count))
    LET snap AS List OF Boolean = ys
    ys = collections::append(ys, collections::mid(snap, start, count))
  NEXT
  io::print("bools " & toString(len(xs)))
  RETURN sameBools(xs, ys)
END FUNC

FUNC grow(xs AS List OF Integer, start AS Integer, count AS Integer) AS List OF Integer
  MUT ys AS List OF Integer = xs
  ys = collections::append(ys, collections::mid(ys, start, count))
  RETURN ys
END FUNC

FUNC refuse(start AS Integer, count AS Integer) AS Nothing
  LET xs AS List OF Integer = [1, 2, 3, 4]
  LET fused AS List OF Integer = grow(xs, start, count) TRAP(e)
    io::print("fused " & toString(start) & "," & toString(count) & " raised " & toString(e.code) & " " & e.message & " len " & toString(len(xs)))
    RETURN
  END TRAP
  io::print("fused " & toString(start) & "," & toString(count) & " gave " & toString(len(fused)))
END FUNC

FUNC refuseCopying(start AS Integer, count AS Integer) AS Nothing
  LET xs AS List OF Integer = [1, 2, 3, 4]
  LET copied AS List OF Integer = collections::append(xs, collections::mid(xs, start, count)) TRAP(e)
    io::print("copied " & toString(start) & "," & toString(count) & " raised " & toString(e.code) & " " & e.message & " len " & toString(len(xs)))
    RETURN
  END TRAP
  io::print("copied " & toString(start) & "," & toString(count) & " gave " & toString(len(copied)))
END FUNC

FUNC main() AS Nothing
  io::print("same " & toString(bytes()) & " " & toString(ints()) & " " & toString(floats()) & " " & toString(bools()))
  refuse(-1, 2)
  refuseCopying(-1, 2)
  refuse(1, -2)
  refuseCopying(1, -2)
  refuse(5, 0)
  refuseCopying(5, 0)
  refuse(2, 3)
  refuseCopying(2, 3)
  refuse(1, 9223372036854775807)
  refuseCopying(1, 9223372036854775807)
  refuse(4, 0)
  refuseCopying(4, 0)
END FUNC
"#;

#[test]
fn self_slice_append_matches_the_copying_form() {
    let project = temp_project("self_slice_append", PROGRAM);
    let binary = build_project(&project);
    let (code, stdout, stderr) = run_capture_with_env(&binary, &[]);
    let ncode = build_ncode(&project, "macos-aarch64", "self_slice_append");
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("same TRUE TRUE TRUE TRUE"),
        "a fused self-slice append diverged from the copying form:\n{stdout}"
    );
    // The fused and the copying statement raise the same error for every bad range.
    let lines: Vec<&str> = stdout.lines().collect();
    for pair in ["-1,2", "1,-2", "5,0", "2,3", "1,9223372036854775807", "4,0"] {
        let fused = lines
            .iter()
            .find(|l| l.starts_with(&format!("fused {pair} ")))
            .unwrap_or_else(|| panic!("no fused line for {pair}:\n{stdout}"));
        let copied = lines
            .iter()
            .find(|l| l.starts_with(&format!("copied {pair} ")))
            .unwrap_or_else(|| panic!("no copying line for {pair}:\n{stdout}"));
        assert_eq!(
            fused.strip_prefix("fused "),
            copied.strip_prefix("copied "),
            "{pair}"
        );
    }
    assert!(
        stdout.contains("fused 4,0 gave 4"),
        "an empty slice at the end is legal:\n{stdout}"
    );
    assert!(
        stdout.contains("fused 2,3 raised 77050001"),
        "a slice past the end raises ErrIndexOutOfRange:\n{stdout}"
    );
    for name in ["bytes", "ints", "floats", "bools", "grow"] {
        let function = ncode["functions"]
            .as_array()
            .expect("functions")
            .iter()
            .find(|f| f["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("no `{name}` in the -ncode dump"));
        let fused = function["instructions"]
            .as_array()
            .expect("instructions")
            .iter()
            .any(|i| {
                i["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("self_slice_append_realloc"))
            });
        assert!(fused, "`{name}` did not take the fused self-slice path");
    }
}
