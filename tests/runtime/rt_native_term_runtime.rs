#[path = "../common/mod.rs"]
mod common;
use common::*;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "rt_native_term_runtime/console.rs"]
mod console;

#[test]
fn native_term_mouse_is_opt_in_and_silent_until_asked() {
    // The opt-in promise, which is the whole reason the mouse surface exists in
    // this shape: a program that imports the mouse members but never calls
    // `enableMouse(TRUE)` must leave the terminal exactly as it found it. No
    // tracking sequence, and every poll reports `MouseKind.None`.
    //
    // plan-94-A pinned this by asserting `enableMouse(TRUE)` ITSELF emitted
    // nothing, because in A the member was an inert stub. plan-94-B gives it its
    // real body — it now writes `\x1b[?1000h\x1b[?1002h\x1b[?1006h` by design
    // (plan-94-B §1) — so the assertion moves to where the promise actually lives:
    // the program that never asks. That is the property worth protecting; "the
    // stub does nothing" was only ever true of the stub.
    let project = temp_project(
        "native_term_mouse_opt_in",
        r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  LET event AS term::MouseEvent = term::pollMouse()
  IF event.kind = term::MouseKind.None THEN
    io::print("KIND:None")
  ELSE
    io::print("KIND:other")
  END IF
  IF event.button = term::MouseButton.None THEN
    io::print("BUTTON:None")
  ELSE
    io::print("BUTTON:other")
  END IF
  io::print("AT:" & toString(event.row) & "," & toString(event.column))
  io::print("MODS:" & toString(event.shift) & toString(event.ctrl) & toString(event.alt))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);

    let direct = run_with_stdin(&executable, b"");
    assert!(
        direct.contains("KIND:None"),
        "an idle pollMouse must report MouseKind.None, got {direct:?}"
    );
    assert!(
        direct.contains("BUTTON:None"),
        "an idle pollMouse must report MouseButton.None, got {direct:?}"
    );
    assert!(
        direct.contains("AT:0,0"),
        "the no-event record's coordinates must be zero, got {direct:?}"
    );
    assert!(
        direct.contains("MODS:FALSEFALSEFALSE"),
        "the no-event record's modifier flags must all be FALSE, got {direct:?}"
    );
    assert!(
        !direct.contains('\x1b'),
        "a program that never calls enableMouse must emit no escape bytes at all, \
         got {direct:?}"
    );

    // Under a real terminal too: tracking sequences would only ever be written to
    // a tty, so a piped run alone could not prove the program silent.
    #[cfg(unix)]
    {
        let pty = run_under_pty(&executable).replace("\r\n", "\n");
        assert!(
            pty.contains("KIND:None"),
            "an idle pollMouse must report None under a pty too, got {pty:?}"
        );
        assert!(
            !pty.contains('\x1b'),
            "a program that never asks for mouse must be silent on a tty, got {pty:?}"
        );
    }
}

/// The program every decoder case below drives: read `count` bytes, echo them,
/// then drain and report every mouse event.
///
/// Reading a fixed count rather than to EOF keeps the case deterministic, and
/// echoing what `io::readRaw` returned is what makes the passthrough claims
/// checkable — the point is not only that the events arrive, but that the
/// keystrokes around them are untouched. `readRaw` because it is byte-exact: the
/// only thing between the program and the bytes is the mouse decoder under test
/// (bug-699 replaced `io::readChar`).
fn mouse_decoder_source(count: usize) -> String {
    format!(
        r#"
IMPORT collections
IMPORT encoding
IMPORT io
IMPORT term

FUNC readChars AS String
  MUT bytes AS List OF Byte = []
  MUT n AS Integer = 0
  WHILE n < {count}
    bytes = collections::append(bytes, io::readRaw())
    n = n + 1
  END WHILE
  RETURN encoding::utf8Decode(bytes)
  TRAP(err)
    RETURN encoding::utf8Decode(bytes)
  END TRAP
END FUNC

FUNC kindName(k AS term::MouseKind) AS String
  IF k = term::MouseKind.None THEN
    RETURN "None"
  END IF
  IF k = term::MouseKind.Down THEN
    RETURN "Down"
  END IF
  IF k = term::MouseKind.Up THEN
    RETURN "Up"
  END IF
  IF k = term::MouseKind.Move THEN
    RETURN "Move"
  END IF
  IF k = term::MouseKind.Drag THEN
    RETURN "Drag"
  END IF
  IF k = term::MouseKind.ScrollUp THEN
    RETURN "ScrollUp"
  END IF
  RETURN "ScrollDown"
END FUNC

FUNC buttonName(b AS term::MouseButton) AS String
  IF b = term::MouseButton.None THEN
    RETURN "None"
  END IF
  IF b = term::MouseButton.Left THEN
    RETURN "Left"
  END IF
  IF b = term::MouseButton.Middle THEN
    RETURN "Middle"
  END IF
  RETURN "Right"
END FUNC

FUNC main AS Integer
  term::enableMouse(TRUE)
  io::print("CHARS:" & readChars())
  MUT going AS Boolean = TRUE
  WHILE going
    LET e AS term::MouseEvent = term::pollMouse()
    IF e.kind = term::MouseKind.None THEN
      going = FALSE
    ELSE
      io::print("EVENT " & kindName(e.kind) & " " & buttonName(e.button) & " " & toString(e.row) & "," & toString(e.column) & " " & toString(e.shift) & toString(e.ctrl) & toString(e.alt))
    END IF
  END WHILE
  io::print("END")
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#
    )
}

#[test]
fn native_term_mouse_decodes_every_kind_and_modifier() {
    // Every `MouseKind` and `MouseButton` the surface publishes, decoded from the
    // SGR bytes a terminal really sends, with the coordinate convention checked on
    // each one: the wire carries `b;x;y` 1-based with x the COLUMN, and the record
    // reports 0-based `row`/`column`. Getting that transposition wrong is the most
    // likely decoder bug and the hardest to notice by eye, so every case pins both
    // numbers rather than just the kind.
    // Driven through `MFB_MOUSE_INJECT` rather than stdin, because a mouse report
    // yields no character: with nothing else to read, a read loop would have
    // to block or hit EOF before the pump had seen the bytes. Injection feeds the
    // same decoder the read path does, at a point where the count of reports is
    // known, which is what makes the case deterministic.
    let project = temp_project("native_term_mouse_kinds", &mouse_decoder_source(0));
    let executable = build_project(&project);

    let feed = concat!(
        "\x1b[<0;1;1M",    // press left at the origin
        "\x1b[<2;5;3M",    // press right
        "\x1b[<1;9;9m",    // release middle
        "\x1b[<32;20;10M", // motion with left held -> Drag
        "\x1b[<35;7;7M",   // motion with no button -> Move
        "\x1b[<64;3;4M",   // wheel away -> ScrollUp
        "\x1b[<65;3;4M",   // wheel toward -> ScrollDown
        "\x1b[<20;2;2M",   // shift(4) + ctrl(16) + left(0)
    );
    let out = run_with_env(&executable, &[("MFB_MOUSE_INJECT", feed)]);

    for expected in [
        "EVENT Down Left 0,0 FALSEFALSEFALSE",
        "EVENT Down Right 2,4 FALSEFALSEFALSE",
        "EVENT Up Middle 8,8 FALSEFALSEFALSE",
        "EVENT Drag Left 9,19 FALSEFALSEFALSE",
        "EVENT Move None 6,6 FALSEFALSEFALSE",
        "EVENT ScrollUp None 3,2 FALSEFALSEFALSE",
        "EVENT ScrollDown None 3,2 FALSEFALSEFALSE",
        "EVENT Down Left 1,1 TRUETRUEFALSE",
    ] {
        assert!(
            out.contains(expected),
            "missing {expected:?} in mouse decode output:\n{out}"
        );
    }
}

#[test]
fn native_term_mouse_leaves_other_input_untouched() {
    // The negative property, and the one that matters most: the decoder sits on
    // the path EVERY keystroke takes, so a byte that is not part of a recognised
    // mouse report has to reach the program unchanged and in order.
    //
    // Three shapes, because they fail differently:
    //   * ordinary characters interleaved with reports — the reports must vanish
    //     and the characters must not,
    //   * an unrecognised CSI (`ESC [ Z`) — buffered as a possible report, then
    //     replayed in full once `Z` rules it out,
    //   * a bare `ESC` — buffered on its own, then replayed when the next byte is
    //     not `[`.
    //
    // The last two are what the drain cursor exists for. Without it the replayed
    // bytes would re-enter the decoder as if freshly read and the escape would be
    // eaten.
    let project = temp_project("native_term_mouse_passthrough", &mouse_decoder_source(6));
    let executable = build_project(&project);

    let mixed = run_with_stdin(&executable, b"ab\x1b[<0;4;2Mcd\x1b[<0;4;2mef");
    assert!(
        mixed.contains("CHARS:abcdef"),
        "keystrokes around mouse reports must arrive intact, got {mixed:?}"
    );
    assert!(
        mixed.contains("EVENT Down Left 1,3"),
        "the interleaved press must still decode, got {mixed:?}"
    );
    assert!(
        mixed.contains("EVENT Up Left 1,3"),
        "the interleaved release must still decode, got {mixed:?}"
    );

    // Five characters: `a`, ESC, `[`, `Z`, `b` — the whole replayed prefix plus
    // the bytes either side of it.
    let project = temp_project("native_term_mouse_csi", &mouse_decoder_source(5));
    let executable = build_project(&project);
    let csi = run_with_stdin(&executable, b"a\x1b[Zb");
    assert!(
        csi.contains("CHARS:a\x1b[Zb"),
        "an unrecognised CSI must pass through byte for byte, got {csi:?}"
    );

    let project = temp_project("native_term_mouse_bare_esc", &mouse_decoder_source(3));
    let executable = build_project(&project);
    let esc = run_with_stdin(&executable, b"a\x1bb");
    assert!(
        esc.contains("CHARS:a\x1bb"),
        "a bare ESC must pass through, got {esc:?}"
    );
}

/// A program that reads with a timeout, the shape every TUI draw loop has: wait
/// up to a deadline for input, and stop when none comes. Each byte is echoed
/// with ESC spelled out so the output stays printable.
fn mouse_poll_then_read_source() -> &'static str {
    r#"
IMPORT encoding
IMPORT io
IMPORT term

FUNC name(b AS Byte) AS String
  IF b = 27 THEN RETURN "<ESC>"
  RETURN encoding::utf8Decode([b])
END FUNC

FUNC main AS Integer
  term::enableMouse(TRUE)
  MUT chars AS String = ""
  DO WHILE TRUE
    LET b AS Byte = io::readRaw(2000) TRAP(e)
      EXIT DO
    END TRAP
    chars = chars & name(b)
  LOOP
  io::print("CHARS:" & chars)
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#
}

/// Spawn `executable`, write `input` to its stdin and then HOLD the pipe open —
/// no EOF — for up to `hold`, closing it early only if the program exits first.
///
/// A lone ESC is only ambiguous while more input might still arrive; closing
/// stdin right after it would hand the program an EOF, which is a different case.
/// Returns the program's stdout and whether it exited while stdin was still open.
fn run_with_held_stdin(executable: &Path, input: &[u8], hold: Duration) -> (String, bool) {
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn executable");
    let mut stdin = child.stdin.take().expect("stdin pipe");
    stdin.write_all(input).expect("write stdin");
    stdin.flush().expect("flush stdin");
    let deadline = Instant::now() + hold;
    let mut exited_while_open = false;
    while Instant::now() < deadline {
        if child.try_wait().expect("poll child").is_some() {
            exited_while_open = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stdin);
    let output = child.wait_with_output().expect("wait for executable");
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        exited_while_open,
    )
}

#[test]
fn native_term_mouse_poll_input_keeps_the_byte_after_an_escape() {
    // bug-669 (2). `ESC x` arriving together is a buffered prefix the decoder
    // flushes when `x` rules out a report: ESC goes back now and `x` is owed. A
    // timed read must not drop that owed byte, nor wait on the source for it.
    let project = temp_project(
        "native_term_mouse_poll_escape",
        mouse_poll_then_read_source(),
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"a\x1bxb\x1b[Zc");
    assert!(
        out.contains("CHARS:a<ESC>xb<ESC>[Zc"),
        "keys after an ESC must survive a timed read, got {out:?}"
    );
}

#[test]
fn native_term_mouse_escape_at_end_of_input_is_delivered() {
    // bug-669 (3). A lone ESC as the last byte before EOF is still the program's:
    // end of input settles that no report is coming, so the held prefix must be
    // handed back before the EOF is.
    let project = temp_project("native_term_mouse_escape_eof", &mouse_decoder_source(2));
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"a\x1b");
    assert!(
        out.contains("CHARS:a\x1b\n"),
        "an ESC right before EOF must reach io::readRaw, got {out:?}"
    );
}

#[test]
fn native_term_mouse_lone_escape_is_reported_without_a_following_key() {
    // bug-669 (1). The Esc key sends one byte and then nothing. With mouse
    // reporting on, the decoder buffers that ESC as the possible start of a report;
    // once the terminal has gone quiet it is plainly a keypress and must be
    // delivered — to a timed read and to a blocking one alike — without
    // waiting for the user to press something else.
    let project = temp_project(
        "native_term_mouse_lone_esc_poll",
        mouse_poll_then_read_source(),
    );
    let executable = build_project(&project);
    // The program polls for up to 2 s at a time; hold stdin open well past the
    // first poll so a FALSE there cannot be explained by EOF.
    let (out, _) = run_with_held_stdin(&executable, b"\x1b", Duration::from_millis(3000));
    assert!(
        out.contains("CHARS:<ESC>"),
        "a timed read must return a lone ESC once input goes quiet, got {out:?}"
    );
    // A whole key sequence as the last input: a read takes the ESC and leaves
    // `[Z` owed inside the decoder, where neither the stdin log nor the OS can see
    // it. A timed read must still return those bytes rather than time out.
    let (out, _) = run_with_held_stdin(&executable, b"\x1b[Z", Duration::from_millis(3000));
    assert!(
        out.contains("CHARS:<ESC>[Z"),
        "a timed read must return bytes the decoder still owes, got {out:?}"
    );

    let source = r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  LET b AS Byte = io::readRaw()
  IF b = 27 THEN
    io::print("GOT:ESC")
  ELSE
    io::print("GOT:" & toString(b))
  END IF
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#;
    let project = temp_project("native_term_mouse_lone_esc_read", source);
    let executable = build_project(&project);
    let (out, exited_while_open) =
        run_with_held_stdin(&executable, b"\x1b", Duration::from_millis(3000));
    assert!(
        exited_while_open && out.contains("GOT:ESC"),
        "a blocking io::readRaw must return a lone ESC without a following key \
         (exited while stdin open: {exited_while_open}), got {out:?}"
    );
}

#[test]
fn native_term_mouse_ring_overwrites_oldest_and_expires_stale() {
    // The ring's two backpressure rules, both of which say the same thing in
    // different directions: what the program gets is what the user is doing NOW.
    //
    //   * overwrite-on-full — a burst longer than the ring keeps the NEWEST events
    //     and drops the oldest, rather than refusing the new ones,
    //   * the 100 ms TTL — an event the program did not collect in time is skipped
    //     rather than replayed late.
    //
    // The burst is driven through the real decoder (72 reports against a 64-slot
    // ring), so this exercises the enqueue path a terminal would, not a synthetic
    // shortcut.
    let source = r#"
IMPORT io
IMPORT os
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  os::sleep(toInt(os::getEnvOr("P94_DELAY_MS", "0")))
  MUT n AS Integer = 0
  MUT first AS Integer = -1
  MUT last AS Integer = -1
  MUT going AS Boolean = TRUE
  WHILE going
    LET e AS term::MouseEvent = term::pollMouse()
    IF e.kind = term::MouseKind.None THEN
      going = FALSE
    ELSE
      IF n = 0 THEN
        first = e.column
      END IF
      last = e.column
      n = n + 1
    END IF
  END WHILE
  io::print("COUNT:" & toString(n) & " FIRST:" & toString(first) & " LAST:" & toString(last))
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#;
    let project = temp_project("native_term_mouse_ring", source);
    let executable = build_project(&project);

    // Column `i` on the wire is 0-based column `i - 1` in the record, so a burst of
    // 1..=n reports carries columns 0..=n-1 and the surviving window is readable
    // straight off the output.
    let burst = |n: usize| -> String { (1..=n).map(|i| format!("\x1b[<0;{i};1M")).collect() };

    let under = run_with_env(&executable, &[("MFB_MOUSE_INJECT", &burst(10))]);
    assert!(
        under.contains("COUNT:10 FIRST:0 LAST:9"),
        "a burst under capacity must arrive whole, got {under:?}"
    );

    let exact = run_with_env(&executable, &[("MFB_MOUSE_INJECT", &burst(64))]);
    assert!(
        exact.contains("COUNT:64 FIRST:0 LAST:63"),
        "a burst of exactly the ring capacity must arrive whole, got {exact:?}"
    );

    // 72 reports into 64 slots: the last 64 survive, so the first surviving column
    // is 8 and the last is 71. Asserting BOTH ends is what distinguishes
    // "overwrote the oldest" from "dropped the newest" — a ring that refused the
    // overflow would report FIRST:0 LAST:63 here and look just as plausible.
    let over = run_with_env(&executable, &[("MFB_MOUSE_INJECT", &burst(72))]);
    assert!(
        over.contains("COUNT:64 FIRST:8 LAST:71"),
        "an overflowing burst must keep the NEWEST events, got {over:?}"
    );

    // The TTL. A short wait leaves the events pollable; a wait past 100 ms does
    // not. Both directions are asserted: only checking the expiry would pass for a
    // ring that dropped everything always.
    let click = "\x1b[<0;3;3M\x1b[<0;3;3m";
    let fresh = run_with_env(
        &executable,
        &[("MFB_MOUSE_INJECT", click), ("P94_DELAY_MS", "20")],
    );
    assert!(
        fresh.contains("COUNT:2"),
        "events polled well within the TTL must survive, got {fresh:?}"
    );
    let stale = run_with_env(
        &executable,
        &[("MFB_MOUSE_INJECT", click), ("P94_DELAY_MS", "400")],
    );
    assert!(
        stale.contains("COUNT:0"),
        "events older than the 100ms TTL must be skipped, got {stale:?}"
    );
}

#[test]
fn native_term_mouse_keeps_immediate_reads_honest() {
    // An immediate read (`timeoutMs = 0`) returns a byte only when one is there
    // (bug-699 replaced `io::pollInput` + `io::readChar` with it). With mouse
    // reporting on, "bytes are ready" stops implying "a byte for the program is
    // ready" — the pending bytes may be a report the pump swallows whole.
    //
    // This has to hold BOTH ways round, and this checks both: every character
    // typed around the reports still arrives, and every report still decodes
    // (consumed into the ring rather than discarded).
    let source = r#"
IMPORT collections
IMPORT encoding
IMPORT errorCode
IMPORT io
IMPORT term

FUNC pump AS String
  MUT bytes AS List OF Byte = []
  MUT polls AS Integer = 0
  WHILE polls < 40
    LET b AS Byte = io::readRaw(0) TRAP(e)
      IF e.code <> errorCode::ErrTimeout THEN
        RETURN encoding::utf8Decode(bytes)
      END IF
      RECOVER 0
    END TRAP
    IF b <> 0 THEN
      bytes = collections::append(bytes, b)
    END IF
    polls = polls + 1
  END WHILE
  RETURN encoding::utf8Decode(bytes)
END FUNC

FUNC main AS Integer
  term::enableMouse(TRUE)
  io::print("CHARS:" & pump())
  MUT events AS Integer = 0
  MUT going AS Boolean = TRUE
  WHILE going
    LET e AS term::MouseEvent = term::pollMouse()
    IF e.kind = term::MouseKind.None THEN
      going = FALSE
    ELSE
      events = events + 1
    END IF
  END WHILE
  io::print("EVENTS:" & toString(events))
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#;
    let project = temp_project("native_term_mouse_poll_input", source);
    let executable = build_project(&project);

    let out = run_with_stdin(&executable, b"ab\x1b[<0;4;2Mcd\x1b[<0;4;2mef");
    assert!(
        out.contains("CHARS:abcdef"),
        "an immediate-read loop must still see every character around the \
         reports, got {out:?}"
    );
    assert!(
        out.contains("EVENTS:2"),
        "the reports the reads consumed must land in the ring, not be \
         discarded, got {out:?}"
    );
}

#[test]
fn native_term_mouse_line_read_keeps_the_bytes_after_an_escape() {
    // bug-699 (found while threading the input buffer through the read path).
    // `io::readLine` loops back to its per-byte read for every byte of the line,
    // and that loop head sat BELOW the mouse pump's owed-byte drain. After an
    // `ESC x` flush — ESC delivered, `x` owed — the next iteration skipped the
    // drain and read a fresh byte, which re-entered the decoder mid-flush: the
    // decoder appended it to the old prefix and delivered the ESC again, so the
    // rest of the line came back as a run of ESCs.
    //
    // Measured on the unfixed compiler: `a ESC x b` read back as `97 27 27 27`,
    // and `a ESC [ Z b` the same. Without `enableMouse` both are exact, which is
    // the control below.
    let source = |mouse: bool| {
        format!(
            r#"
IMPORT io
IMPORT strings
IMPORT term

FUNC main AS Integer
  term::enableMouse({mouse})
  LET l AS String = io::readLine()
  term::enableMouse(FALSE)
  MUT s AS String = ""
  FOR EACH b IN strings::toBytes(l)
    s = s & toString(b) & " "
  NEXT
  io::printError("LINE:" & s)
  RETURN 0
END FUNC
"#,
            mouse = if mouse { "TRUE" } else { "FALSE" }
        )
    };
    for mouse in [false, true] {
        let project = temp_project("native_term_mouse_line_escape", &source(mouse));
        let executable = build_project(&project);
        for (input, want) in [
            (&b"a\x1bxb\n"[..], "LINE:97 27 120 98 "),
            (b"a\x1b[Zb\n", "LINE:97 27 91 90 98 "),
            (b"\x1b\x1bq\n", "LINE:27 27 113 "),
        ] {
            let output = Command::new(&executable)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    child.stdin.take().expect("stdin").write_all(input)?;
                    child.wait_with_output()
                })
                .expect("run the line reader");
            let err = String::from_utf8_lossy(&output.stderr);
            assert!(
                err.contains(want),
                "enableMouse({mouse}): readLine of {input:?} must return every byte, \
                 want {want:?}, got {err:?}"
            );
        }
    }
}

#[test]
fn native_term_mouse_does_not_echo_during_a_line_read() {
    // `io::input`/`io::readLine` restore the saved cooked line discipline for the
    // duration of their read, which re-enables ECHO. A mouse report arriving in
    // that window would be echoed onto the user's screen as literal garbage in the
    // middle of what they are typing, and delivered a line at a time rather than a
    // byte at a time.
    //
    // So tracking is withdrawn before the restore and re-established after the raw
    // termios goes back. The assertion is on the ORDER, because presence alone
    // would pass for a bracket placed on the wrong side of the restore — which
    // would leave exactly the window it exists to close.
    let project = temp_project(
        "native_term_mouse_line_read",
        r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  io::print("BEFORE")
  LET line AS String = io::readLine()
  io::print("LINE:" & line)
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"hello\n");

    // The line itself must survive. This is not incidental: the resume write runs
    // after the read's result is staged, and an emitter that clobbers the result
    // bank there returns a wild pointer — the program faults on the first use of
    // the string while the escape bytes still look perfectly correct in the output.
    assert!(
        out.contains("LINE:hello"),
        "the line read must survive the mouse bracket, got {out:?}"
    );

    let before = out.find("BEFORE").expect("marker");
    let suspend = out[before..]
        .find("\x1b[?1000l")
        .map(|i| i + before)
        .unwrap_or_else(|| panic!("tracking must be withdrawn for the line read: {out:?}"));
    let resume = out[suspend..]
        .find("\x1b[?1000h")
        .map(|i| i + suspend)
        .unwrap_or_else(|| panic!("tracking must be re-established after it: {out:?}"));
    let line = out.find("LINE:hello").expect("line");
    assert!(
        suspend < resume && resume < line,
        "the suspend must precede the resume and both must precede the line's \
         delivery, got suspend={suspend} resume={resume} line={line} in {out:?}"
    );
}

#[test]
fn native_term_mouse_is_per_thread_and_needs_stdin() {
    // Mouse events are **per-thread**, and follow stdin: a worker decodes the
    // bytes it reads, into the ring in its own arena. So a worker that never
    // subscribes to stdin sees no events at all, and that is not a special case
    // bolted on — it falls out of where the ring lives.
    //
    // Two things are asserted because they are separate mechanisms that happen to
    // agree: the worker's `pollMouse` reports `None` (its arena's ring pointer is
    // null — nothing decoded into it), and its raw stdin read still raises
    // `ErrInvalidContext` naming `thread::openStdIn` (the pre-existing
    // subscription trap, which the pump must not have swallowed by reading ahead).
    //
    // The main thread polls too, and DOES see the event, which is what makes the
    // worker's silence meaningful rather than a program that simply decoded
    // nothing.
    let project = temp_project(
        "native_term_mouse_thread",
        r#"
IMPORT io
IMPORT term
IMPORT thread

ISOLATED FUNC worker(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  LET e AS term::MouseEvent = term::pollMouse()
  IF e.kind = term::MouseKind.None THEN
    io::print("WORKER-KIND:None")
  ELSE
    io::print("WORKER-KIND:other")
  END IF
  LET c AS Byte = io::readRaw()
  io::print("WORKER-READ:" & toString(c))
  RETURN 0
  TRAP(err)
    io::print("WORKER-TRAP:" & toString(err.code))
    RETURN 1
  END TRAP
END FUNC

FUNC main AS Integer
  term::enableMouse(TRUE)
  RES t AS Thread OF String TO Integer = thread::start(worker, "x")
  LET rc AS Integer = thread::waitFor(t)
  MUT events AS Integer = 0
  MUT going AS Boolean = TRUE
  WHILE going
    LET e AS term::MouseEvent = term::pollMouse()
    IF e.kind = term::MouseKind.None THEN
      going = FALSE
    ELSE
      events = events + 1
    END IF
  END WHILE
  io::print("MAIN-EVENTS:" & toString(events))
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_env(
        &executable,
        &[("MFB_MOUSE_INJECT", "\x1b[<0;3;3M\x1b[<0;3;3m")],
    );

    assert!(
        out.contains("WORKER-KIND:None"),
        "a worker that never subscribed to stdin must see no mouse events — the \
         ring is in its own arena and nothing decoded into it, got {out:?}"
    );
    assert!(
        out.contains("WORKER-TRAP:"),
        "an unsubscribed worker's stdin read must still raise; the pump must not \
         have read past the subscription check, got {out:?}"
    );
    assert!(
        out.contains("MAIN-EVENTS:2"),
        "the main thread must still receive the injected events, or the worker's \
         silence proves nothing, got {out:?}"
    );
}

/// bug-1015: `term::pollMouse` decodes the input that is already waiting itself,
/// so a program sees mouse events whether or not it has read standard input — and
/// the keystrokes the poll had to read past to reach the reports are kept, in
/// order, for the program's next `io` read.
///
/// The decoder used to run only on the read path, so a poll before any read found
/// an empty ring: the reports sat undecoded in front of it (`EVENTS:0`). The same
/// one drain serves the console log and every `--app` window pipe; the console is
/// the case a test can feed byte-exactly.
const POLL_BEFORE_READ_SOURCE: &str = r#"
IMPORT collections
IMPORT encoding
IMPORT errorCode
IMPORT io
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  MUT events AS String = ""
  MUT going AS Boolean = TRUE
  WHILE going
    LET e AS term::MouseEvent = term::pollMouse()
    IF e.kind = term::MouseKind.None THEN
      going = FALSE
    ELSE
      events = events & "[" & toString(e.row) & "," & toString(e.column) & "]"
    END IF
  END WHILE
  io::print("EVENTS:" & events)
  MUT bytes AS List OF Byte = []
  MUT reading AS Boolean = TRUE
  WHILE reading
    LET b AS Byte = io::readRaw(0) TRAP(err)
      RECOVER 0
    END TRAP
    IF b = 0 THEN
      reading = FALSE
    ELSE
      bytes = collections::append(bytes, b)
    END IF
  END WHILE
  io::print("CHARS:" & encoding::utf8Decode(bytes))
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#;

#[test]
fn native_term_poll_mouse_decodes_without_a_prior_read_and_keeps_the_keys() {
    let project = temp_project("native_term_poll_before_read", POLL_BEFORE_READ_SOURCE);
    let executable = build_project(&project);
    // Two reports with keystrokes before, between and after them.
    let out = run_with_stdin(&executable, b"ab\x1b[<0;4;2Mcd\x1b[<0;7;5mef");
    assert!(
        out.contains("EVENTS:[1,3][4,6]"),
        "pollMouse must decode the waiting reports itself, before any read, got {out:?}"
    );
    assert!(
        out.contains("CHARS:abcdef"),
        "the keystrokes pollMouse read past must reach the next io read intact \
         and in order, got {out:?}"
    );
}

/// bug-1099: a console program that polls the mouse and contains no `io` reader at
/// all sees the events the terminal reported. The standard-input subscription and
/// the input buffer used to be decided by the `io` readers alone, so this program's
/// main thread was never subscribed and the pending-input drain was skipped
/// (`EVENTS:0`). The mouse readers are stdin readers for that decision.
const POLL_ONLY_SOURCE: &str = r#"
IMPORT io
IMPORT os
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  MUT seen AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 20
    LET e AS term::MouseEvent = term::pollMouse()
    IF e.kind <> term::MouseKind.None THEN
      seen = seen + 1
    END IF
    os::sleep(20)
    i = i + 1
  END WHILE
  io::print("EVENTS:" & toString(seen))
  term::enableMouse(FALSE)
  RETURN 0
END FUNC
"#;

#[test]
fn native_term_poll_mouse_needs_no_io_reader_in_the_program() {
    let project = temp_project("native_term_poll_only", POLL_ONLY_SOURCE);
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"\x1b[<0;5;5M\x1b[<0;5;5m");
    assert!(
        out.contains("EVENTS:2"),
        "a program that only polls the mouse must still receive the reports, got {out:?}"
    );
}

/// bug-1015, the `--app` window half: the same program in a headless app build,
/// whose standard input is the stream the window's mouse handlers write their
/// reports into. Headless is the run a test can feed byte-exactly; the windowed
/// GTK proof is `scripts/test-appimage.sh --gui`'s pollMouse row.
#[cfg(unix)]
#[test]
fn app_term_poll_mouse_decodes_without_a_prior_read_and_keeps_the_keys() {
    let name = "app_term_poll_before_read";
    let project = temp_project(name, POLL_BEFORE_READ_SOURCE);
    let binary = build_app_debug(&project, name);
    let mut child = Command::new(&binary)
        .env("MFB_HEADLESS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the headless app");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"ab\x1b[<0;4;2Mcd\x1b[<0;7;5mef")
        .expect("feed stdin");
    let run = child.wait_with_output().expect("wait");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        out.contains("EVENTS:[1,3][4,6]"),
        "an app pollMouse must decode the waiting reports itself, got {out:?}"
    );
    assert!(
        out.contains("CHARS:abcdef"),
        "the keystrokes it read past must reach the next io read, got {out:?}"
    );
}
