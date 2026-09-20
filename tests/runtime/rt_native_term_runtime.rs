#[path = "../common/mod.rs"]
mod common;
use common::*;

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

/// The program every decoder case below drives: read `count` characters, echo
/// them, then drain and report every mouse event.
///
/// Reading a fixed count rather than to EOF keeps the case deterministic, and
/// echoing what `io::readChar` returned is what makes the passthrough claims
/// checkable — the point is not only that the events arrive, but that the
/// keystrokes around them are untouched.
fn mouse_decoder_source(count: usize) -> String {
    format!(
        r#"
IMPORT io
IMPORT term

FUNC readChars AS String
  MUT chars AS String = ""
  MUT n AS Integer = 0
  WHILE n < {count}
    chars = chars & io::readChar()
    n = n + 1
  END WHILE
  RETURN chars
  TRAP(err)
    RETURN chars
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
    // yields no character: with nothing else to read, a `readChar` loop would have
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
fn native_term_off_withdraws_mouse_tracking() {
    // A program that enables mouse reporting and exits without disabling it would
    // otherwise leave the user's terminal reporting every movement to whatever
    // runs next — which is not a small annoyance: the shell prompt fills with
    // escape garbage on the first mouse move.
    //
    // `term::off` therefore withdraws tracking unconditionally, and does so even
    // when TUI mode was never entered, because mouse mode is independent of
    // `term::on`. The order is asserted as well as the presence: the reset must
    // come after the enable, or it resets nothing.
    let project = temp_project(
        "native_term_mouse_off",
        r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  io::print("MARK")
  term::off()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");

    let enable = out
        .find("\x1b[?1000h")
        .unwrap_or_else(|| panic!("enableMouse must set mode 1000, got {out:?}"));
    assert!(
        out.contains("\x1b[?1002h"),
        "enableMouse must set mode 1002 so drags report motion, got {out:?}"
    );
    assert!(
        out.contains("\x1b[?1006h"),
        "enableMouse must set mode 1006: without SGR coordinates there is no way \
         to report a column past 223, let alone a pixel, got {out:?}"
    );
    let reset = out
        .find("\x1b[?1000l")
        .unwrap_or_else(|| panic!("term::off must reset mode 1000, got {out:?}"));
    assert!(
        reset > enable,
        "the reset must follow the enable, or it resets nothing: {out:?}"
    );
}

#[test]
fn native_term_size_reports_unsupported_off_and_size_when_active() {
    // `term::terminalSize` errors with ERR_UNSUPPORTED_OPERATION while TUI mode
    // is off (plan-01-term.md §4.7) and returns the live window size once active.
    let project = temp_project(
        "native_term_size",
        r#"
IMPORT io
IMPORT term

FUNC sizeWhileOff AS Nothing
  LET size AS term::TermSize = term::terminalSize()
  io::print("OFF-COLS:" & toString(size.columns))
  RETURN NOTHING
  TRAP(err)
    io::print("OFF-ERR:" & toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC printSize AS Nothing
  LET size AS term::TermSize = term::terminalSize()
  io::print("SIZE:" & toString(size.columns) & "x" & toString(size.rows))
  RETURN NOTHING
  TRAP(err)
    io::print("SIZE-ERR:" & toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC main AS Integer
  sizeWhileOff()
  term::on()
  printSize()
  term::off()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);

    // Piped (no tty): the off-path errors, and the active path also reports
    // unsupported because the ioctl fails on a pipe (program still exits 0
    // because both reads are trapped).
    let direct = run_with_stdin(&executable, b"");
    assert!(
        direct.contains("OFF-ERR:77050007"),
        "expected off-path unsupported error, got {direct:?}"
    );
    assert!(
        direct.contains("SIZE-ERR:77050007"),
        "expected non-tty active terminalSize to report unsupported, got {direct:?}"
    );

    // Under a pty with a known window size, the active path reports it. Only the
    // Unix harness can supply a terminal on the other end of the child's streams
    // (see `run_under_pty`), so the tty half is Unix-only; the piped half above
    // runs everywhere.
    #[cfg(unix)]
    {
        let pty = run_under_pty(&executable).replace("\r\n", "\n");
        assert!(
            pty.contains("OFF-ERR:77050007"),
            "expected off-path unsupported error under pty, got {pty:?}"
        );
        assert!(
            pty.contains("SIZE:100x40"),
            "expected live window size under pty, got {pty:?}"
        );
    }
}

#[test]
fn native_term_console_emits_expected_escape_sequences() {
    // The console backend is a shadow grid (plan-35): drawing calls (setColor/
    // setAttr/cursor/moveTo/clear) mutate the in-memory back buffer and emit no
    // ANSI; only `term::sync`/`term::off` present the frame, diffing the back
    // buffer against the last-presented front buffer and writing the changed cells
    // as one batched escape run. So the pen (foreground/background/bold/underline)
    // only surfaces on cells a glyph is actually written to — here the "HELLO" run
    // positioned by `moveTo`. Driven into a pipe (no tty needed).
    let project = temp_project(
        "native_term_escapes",
        r#"
IMPORT io
IMPORT term
IMPORT color

FUNC main AS Integer
  term::on()
  term::setForeground(color::rgb(0, 255, 0))
  term::setBackground(color::rgb(0, 0, 0))
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::moveTo(2, 4)
  io::print("HELLO")
  term::hideCursor()
  term::sync()
  term::off()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    for needle in [
        "\x1b[?1049h",        // on(): enter the alternate screen
        "\x1b[2J\x1b[H",      // on()'s first present clears the alternate screen
        "\x1b[38;2;0;255;0m", // setForeground(0,255,0), presented on the drawn run
        "\x1b[48;2;0;0;0m",   // setBackground(0,0,0)
        "\x1b[1m",            // setBold(TRUE)
        "\x1b[4m",            // setUnderline(TRUE)
        "HELLO",              // the glyph run, drawn with the pen above
        "\x1b[?25l",          // hideCursor(), presented as the frame's cursor state
        "\x1b[?1049l",        // off(): leave the alternate screen
    ] {
        assert!(
            out.contains(needle),
            "missing escape {:?} in output {:?}",
            needle,
            hex(out.as_bytes())
        );
    }
}

#[test]
fn native_term_gate_no_ops_while_inactive() {
    // Every term:: call except on()/isOn() is inert while TUI mode is off
    // (plan-01-term.md §4.2.1): setters/surface calls emit nothing, getters return
    // the inert default, isOn() is FALSE.
    let project = temp_project(
        "native_term_gate",
        r#"
IMPORT io
IMPORT term
IMPORT color

FUNC main AS Integer
  term::setForeground(color::rgb(1, 2, 3))
  term::setBackground(color::rgb(4, 5, 6))
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::moveTo(5, 5)
  term::clear()
  term::showCursor()
  term::hideCursor()
  LET on AS Boolean = term::isOn()
  LET fg AS color::Color = term::getForeground()
  LET bg AS color::Color = term::getBackground()
  LET bold AS Boolean = term::getBold()
  LET ul AS Boolean = term::getUnderline()
  io::print("ON:" & toString(on))
  io::print("FG:" & toString(fg.red) & "," & toString(fg.green) & "," & toString(fg.blue))
  io::print("BG:" & toString(bg.red) & "," & toString(bg.green) & "," & toString(bg.blue))
  io::print("BOLD:" & toString(bold))
  io::print("UL:" & toString(ul))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    assert!(
        !out.contains('\x1b'),
        "inactive term:: leaked escape bytes: {:?}",
        hex(out.as_bytes())
    );
    assert!(
        out.contains("ON:FALSE"),
        "isOn should be FALSE while off: {out:?}"
    );
    assert!(
        out.contains("FG:255,255,255"),
        "inert fg should be white: {out:?}"
    );
    assert!(
        out.contains("BG:0,0,0"),
        "inert bg should be black: {out:?}"
    );
    assert!(
        out.contains("BOLD:FALSE"),
        "inert bold should be FALSE: {out:?}"
    );
    assert!(
        out.contains("UL:FALSE"),
        "inert underline should be FALSE: {out:?}"
    );
}

#[test]
fn native_term_did_resize_reports_false_without_a_resize() {
    // `term::didResize` (planning/term.md #11) is a cached read-and-clear Boolean:
    // FALSE while off, and FALSE after `on()`+`sync()` when the terminal size has
    // not changed. Two consecutive reads both stay FALSE (no spurious latch). The
    // resize→TRUE path is a live size change, exercised by the app backends and the
    // CLI reflow, not reproducible from this fixed-size harness.
    let project = temp_project(
        "native_term_did_resize",
        r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  LET offResize AS Boolean = term::didResize()
  term::on()
  term::sync()
  LET a AS Boolean = term::didResize()
  LET b AS Boolean = term::didResize()
  term::off()
  io::print("OFF:" & toString(offResize))
  io::print("A:" & toString(a))
  io::print("B:" & toString(b))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);

    // The pty run is Unix-only (see `run_under_pty`); the piped run covers every
    // platform.
    #[cfg(unix)]
    let runs = vec![
        run_with_stdin(&executable, b""),
        run_under_pty(&executable).replace("\r\n", "\n"),
    ];
    #[cfg(not(unix))]
    let runs = vec![run_with_stdin(&executable, b"")];

    for out in runs {
        assert!(
            out.contains("OFF:FALSE"),
            "didResize should be FALSE while off: {out:?}"
        );
        assert!(
            out.contains("A:FALSE"),
            "didResize should be FALSE with no resize: {out:?}"
        );
        assert!(
            out.contains("B:FALSE"),
            "second didResize should stay FALSE (read-and-clear, no latch): {out:?}"
        );
    }
}

#[test]
fn native_term_on_resets_state_to_defaults() {
    // on() resets all state to defaults every time it is called (plan-01-term.md
    // §4.2): set non-defaults, off(), on() again, read the defaults back.
    let project = temp_project(
        "native_term_reset",
        r#"
IMPORT io
IMPORT term
IMPORT color

FUNC main AS Integer
  term::on()
  term::setForeground(color::rgb(10, 20, 30))
  term::setBackground(color::rgb(40, 50, 60))
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::off()
  term::on()
  LET fg AS color::Color = term::getForeground()
  LET bg AS color::Color = term::getBackground()
  LET bold AS Boolean = term::getBold()
  LET ul AS Boolean = term::getUnderline()
  LET on AS Boolean = term::isOn()
  term::off()
  io::print("FG:" & toString(fg.red) & "," & toString(fg.green) & "," & toString(fg.blue))
  io::print("BG:" & toString(bg.red) & "," & toString(bg.green) & "," & toString(bg.blue))
  io::print("BOLD:" & toString(bold))
  io::print("UL:" & toString(ul))
  io::print("ON:" & toString(on))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    assert!(
        out.contains("FG:255,255,255"),
        "on() should reset fg to white: {out:?}"
    );
    assert!(
        out.contains("BG:0,0,0"),
        "on() should reset bg to black: {out:?}"
    );
    assert!(
        out.contains("BOLD:FALSE"),
        "on() should reset bold: {out:?}"
    );
    assert!(
        out.contains("UL:FALSE"),
        "on() should reset underline: {out:?}"
    );
    assert!(
        out.contains("ON:TRUE"),
        "isOn should be TRUE while active: {out:?}"
    );
}

#[test]
fn native_term_draw_text_attributed_applies_bold_underline() {
    // `term::drawText(x, y, AttributedString)` stamps the visible text honouring
    // the per-scalar bold/underline attributes, drawing maximal same-style runs and
    // ignoring attributes the terminal cannot represent (italic/font here). The
    // default pen is white-on-black, so each presented run carries its own SGR:
    // "Hi " plain, "Bold" bold, " " plain, "Und" underlined. Driven into a pipe.
    let project = temp_project(
        "native_term_draw_attr",
        r#"
IMPORT term
IMPORT astrings

FUNC main AS Integer
  term::on()
  MUT a AS AttributedString = astrings::fromString("Hi Bold Und")
  a = astrings::addAttribute(a, 3, 6, astrings::bold())
  a = astrings::addAttribute(a, 8, 10, astrings::underline())
  a = astrings::addAttribute(a, 0, 1, astrings::italic())
  a = astrings::addAttribute(a, 0, 10, astrings::font("Serif"))
  term::drawText(0, 0, a)
  term::sync()
  term::off()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    // "Hi " is plain despite the italic/font spans covering it (both ignored): the
    // reset pen (`\x1b[0m`) with default colours and no bold/underline.
    assert!(
        out.contains("\x1b[0m\x1b[38;2;255;255;255m\x1b[48;2;0;0;0mHi "),
        "prefix run should be plain (ignored attrs), got {:?}",
        hex(out.as_bytes())
    );
    // "Bold" carries the bold SGR (and no underline).
    assert!(
        out.contains("\x1b[1m\x1b[38;2;255;255;255m\x1b[48;2;0;0;0mBold"),
        "bold run should carry \\x1b[1m, got {:?}",
        hex(out.as_bytes())
    );
    // "Und" carries the underline SGR (and no bold).
    assert!(
        out.contains("\x1b[4m\x1b[38;2;255;255;255m\x1b[48;2;0;0;0mUnd"),
        "underline run should carry \\x1b[4m, got {:?}",
        hex(out.as_bytes())
    );
}

#[test]
fn native_term_draw_text_attributed_applies_colors() {
    // `term::drawText(x, y, AttributedString)` also honours the per-scalar
    // foreground/background color attributes (packed 0xRRGGBB), drawing maximal
    // same-color runs. "Red" carries a red foreground over the default black
    // background; "Blue" carries a blue background over the default white
    // foreground — each unset channel falls back to the saved pen. Driven into a
    // pipe so `sync()` emits truecolor SGR escapes.
    let project = temp_project(
        "native_term_draw_attr_color",
        r#"
IMPORT term
IMPORT astrings
IMPORT color

FUNC main AS Integer
  term::on()
  MUT a AS AttributedString = astrings::fromString("RedBlue")
  a = astrings::addAttribute(a, 0, 2, astrings::foreground(color::rgb(255, 0, 0)))
  a = astrings::addAttribute(a, 3, 6, astrings::background(color::rgb(0, 0, 255)))
  term::drawText(0, 0, a)
  term::sync()
  term::off()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    // "Red": red foreground (38;2;255;0;0), default black background.
    assert!(
        out.contains("\x1b[38;2;255;0;0m\x1b[48;2;0;0;0mRed"),
        "red run should carry a red foreground, got {:?}",
        hex(out.as_bytes())
    );
    // "Blue": default white foreground, blue background (48;2;0;0;255).
    assert!(
        out.contains("\x1b[38;2;255;255;255m\x1b[48;2;0;0;255mBlue"),
        "blue run should carry a blue background, got {:?}",
        hex(out.as_bytes())
    );
}

/// A half-transparent foreground draws exactly the cells an opaque one draws.
///
/// The partner to `color-payload-rt`'s alpha round-trip, and the reason both are
/// needed: plan-122-E widened the attribute payload so a colour keeps its alpha
/// through storage, and that round-trip is asserted there. What must NOT change is
/// what reaches the terminal — it has no alpha, so `__term_applyFg` reads only
/// `.red`/`.green`/`.blue` from `color::fromPacked` and the emitted SGR escape is
/// byte-identical whatever the alpha was.
///
/// Without this, "the bridge ignores alpha" would be a claim on a man page with
/// nothing holding it: a future change that blended against the cell background,
/// or that leaked the alpha byte into the escape, would break no test.
#[test]
fn native_term_draw_text_attributed_ignores_alpha() {
    let program = |alpha: u16| {
        format!(
            r#"
IMPORT term
IMPORT astrings
IMPORT color

FUNC main AS Integer
  term::on()
  MUT a AS AttributedString = astrings::fromString("RedBlue")
  a = astrings::addAttribute(a, 0, 2, astrings::foreground(color::rgba(255, 0, 0, {alpha})))
  a = astrings::addAttribute(a, 3, 6, astrings::background(color::rgba(0, 0, 255, {alpha})))
  term::drawText(0, 0, a)
  term::sync()
  term::off()
  RETURN 0
END FUNC
"#
        )
    };

    // Opaque, half-transparent, and fully transparent must all emit the same bytes.
    let opaque = run_with_stdin(
        &build_project(&temp_project("native_term_alpha_255", &program(255))),
        b"",
    );
    let half = run_with_stdin(
        &build_project(&temp_project("native_term_alpha_128", &program(128))),
        b"",
    );
    let clear = run_with_stdin(
        &build_project(&temp_project("native_term_alpha_0", &program(0))),
        b"",
    );

    // The colours reach the terminal at full strength regardless of alpha — the
    // same escapes `native_term_draw_text_attributed_applies_colors` pins.
    assert!(
        half.contains("\x1b[38;2;255;0;0m\x1b[48;2;0;0;0mRed"),
        "a half-transparent red must still emit a full-strength red, got {:?}",
        hex(half.as_bytes())
    );
    assert!(
        half.contains("\x1b[38;2;255;255;255m\x1b[48;2;0;0;255mBlue"),
        "a half-transparent blue background must still emit full-strength blue, got {:?}",
        hex(half.as_bytes())
    );

    // And alpha changes nothing at all about the output.
    assert_eq!(
        opaque, half,
        "alpha 128 changed the terminal output; the bridge must ignore alpha"
    );
    assert_eq!(
        opaque, clear,
        "alpha 0 changed the terminal output; the bridge must ignore alpha"
    );
}

#[test]
fn native_term_mouse_keeps_poll_input_honest() {
    // `io::pollInput` promises that a following read will not block. With mouse
    // reporting on, "bytes are ready" stops implying "a character is ready" — the
    // pending bytes may be a report the pump swallows whole — so an unverified
    // TRUE would hand the program a `readChar` that blocks on a terminal that has
    // gone quiet.
    //
    // The fix has to hold BOTH ways round, and this checks both: every character
    // typed around the reports still arrives (the verification does not eat the
    // byte it inspected — it pushes it back), and every report still decodes (the
    // verification consumed them into the ring rather than discarding them).
    let source = r#"
IMPORT io
IMPORT term

FUNC pump AS String
  MUT chars AS String = ""
  MUT polls AS Integer = 0
  WHILE polls < 40
    IF io::pollInput(0) THEN
      chars = chars & io::readChar()
    END IF
    polls = polls + 1
  END WHILE
  RETURN chars
  TRAP(err)
    RETURN chars
  END TRAP
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
        "a pollInput-guarded read loop must still see every character — the \
         verification reads a byte to classify it and must push back the ones that \
         belong to the program, got {out:?}"
    );
    assert!(
        out.contains("EVENTS:2"),
        "the reports the verification consumed must land in the ring, not be \
         discarded, got {out:?}"
    );
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
  LET c AS String = io::readChar()
  io::print("WORKER-READ:" & c)
  RETURN 0
  TRAP(err)
    io::print("WORKER-TRAP:" & toString(err.code))
    RETURN 1
  END TRAP
END FUNC

FUNC main AS Integer
  term::enableMouse(TRUE)
  LET t AS Thread OF String TO Integer = thread::start(worker, "x")
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
