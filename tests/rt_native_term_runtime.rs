mod common;
use common::*;

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

FUNC main AS Integer
  term::on()
  term::setForeground(0, 255, 0)
  term::setBackground(0, 0, 0)
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

FUNC main AS Integer
  term::setForeground(1, 2, 3)
  term::setBackground(4, 5, 6)
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::moveTo(5, 5)
  term::clear()
  term::showCursor()
  term::hideCursor()
  LET on AS Boolean = term::isOn()
  LET fg AS term::TermColor = term::getForeground()
  LET bg AS term::TermColor = term::getBackground()
  LET bold AS Boolean = term::getBold()
  LET ul AS Boolean = term::getUnderline()
  io::print("ON:" & toString(on))
  io::print("FG:" & toString(fg.r) & "," & toString(fg.g) & "," & toString(fg.b))
  io::print("BG:" & toString(bg.r) & "," & toString(bg.g) & "," & toString(bg.b))
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

FUNC main AS Integer
  term::on()
  term::setForeground(10, 20, 30)
  term::setBackground(40, 50, 60)
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::off()
  term::on()
  LET fg AS term::TermColor = term::getForeground()
  LET bg AS term::TermColor = term::getBackground()
  LET bold AS Boolean = term::getBold()
  LET ul AS Boolean = term::getUnderline()
  LET on AS Boolean = term::isOn()
  term::off()
  io::print("FG:" & toString(fg.r) & "," & toString(fg.g) & "," & toString(fg.b))
  io::print("BG:" & toString(bg.r) & "," & toString(bg.g) & "," & toString(bg.b))
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
