//! Linux GTK4 app-mode build regression tests (plan-05-linux-app.md).
//!
//! These drive the real `mfb` CLI for a `linux-aarch64` target and inspect the
//! produced artifacts. They never execute the produced ELF (the dev/CI host is
//! macOS and cannot run a Linux+GTK aarch64 binary; see plan-05 §9), so they lock
//! the cross-compilation behavior — build mode, GTK import surface, single glibc
//! output flavor — rather than runtime behavior.

#[path = "../common/mod.rs"]
mod common;
use common::temp_project;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "linux-aarch64";

fn run_mfb(project: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .args(args)
        .arg(project)
        .output()
        .expect("run mfb build");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
    )
}

const APP_SOURCE: &str = "IMPORT io\n\nSUB main()\n  io::print(\"App mode started\")\n  LET name AS String = io::readLine()\n  io::print(\"Hello, \" & name)\nEND SUB\n";

#[test]
fn linux_app_mode_nir_records_build_mode() {
    let project = temp_project("linux_app_nir", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET, "-nir"]);
    assert!(ok, "build -app -nir failed:\n{stdout}\n{stderr}");
    let nir = fs::read_to_string(project.join("linux_app_nir.nir")).expect("read nir");
    assert!(
        nir.contains("\"buildMode\": \"linux-app\""),
        "NIR should record the linux-app build mode, got:\n{nir}"
    );
}

#[test]
fn linux_app_mode_plan_declares_gtk_libraries() {
    let project = temp_project("linux_app_nplan", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET, "-nplan"]);
    assert!(ok, "build -app -nplan failed:\n{stdout}\n{stderr}");
    let nplan = fs::read_to_string(project.join("linux_app_nplan.nplan")).expect("read nplan");
    for library in [
        "libgtk-4.so.1",
        "libgobject-2.0.so.0",
        "libglib-2.0.so.0",
        "libgio-2.0.so.0",
    ] {
        assert!(
            nplan.contains(library),
            "nplan should declare {library} as a GTK app-mode dependency"
        );
    }
    for symbol in [
        "gtk_application_new",
        "g_application_run",
        "g_signal_connect_data",
    ] {
        assert!(
            nplan.contains(symbol),
            "nplan should import the GTK bootstrap symbol {symbol}"
        );
    }
    // App mode registers no console SIGINT/SIGTERM handler (plan-05 §6.1): it has
    // a window-driven finish path instead. That invariant is about the HANDLER, so
    // assert the handler symbol directly.
    //
    // This used to assert `!nplan.contains("\"signal\"")` and used the libc import
    // as a proxy for "no handler is registered". bug-467 broke the proxy without
    // touching the invariant: the program entry now calls `signal(SIGPIPE, SIG_IGN)`
    // on every POSIX target, app mode included, so that a socket peer cannot kill
    // the process — an app-mode program owns sockets just the same. So `signal` IS
    // imported now and is genuinely referenced, while the console handler still is
    // not. The proxy was disproved; the invariant it stood for is unchanged and is
    // pinned here more precisely than before.
    assert!(
        !nplan.contains("_mfb_rt_signal_handler"),
        "app mode should not register the console SIGINT/SIGTERM handler"
    );
    assert!(
        nplan.contains("\"signal\""),
        "app mode must still import `signal` for the bug-467 SIGPIPE disposition"
    );
}

#[test]
fn linux_app_mode_emits_a_single_sealed_appimage() {
    // plan-51-C: `--app` emits one artifact — `build/<name>.AppImage` — matching
    // macOS `--app`'s single `.app`, and the intermediate AppDir is gone.
    let project = temp_project("linux_app_exe", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET]);
    assert!(ok, "build -app failed:\n{stdout}\n{stderr}");
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    // plan-56-B: one AppImage per libc world, mirroring the console build's two
    // flavored `.out` files.
    assert_eq!(
        written.len(),
        2,
        "app mode emits one artifact per libc flavor, got: {written:?}"
    );
    let names: Vec<&str> = written
        .iter()
        .map(|p| Path::new(p).file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"linux_app_exe-glibc.AppImage"), "{names:?}");
    assert!(names.contains(&"linux_app_exe-musl.AppImage"), "{names:?}");
    let path = PathBuf::from(written[0]);
    for flavor in ["glibc", "musl"] {
        assert!(
            !project
                .join(format!("build/linux_app_exe-{flavor}.AppDir"))
                .exists(),
            "a plain --app build leaves no AppDir behind (plan-51-C §3.3)"
        );
    }
    assert!(
        !project.join("build/linux_app_exe.out").exists(),
        "the pre-plan-51 bare <name>.out must be gone"
    );
    assert!(
        !project.join("build/linux_app_exe.AppImage").exists(),
        "the unflavored pre-plan-56 name must be gone"
    );

    let bytes = fs::read(&path).expect("read AppImage");
    assert_eq!(&bytes[0..4], b"\x7fELF", "the runtime is an ELF image");
    // The magic external tools key off: hex 0x414902 at offset 8.
    assert_eq!(&bytes[8..11], b"AI\x02", "AppImage type-2 magic");

    // A valid squashfs superblock begins at the runtime's exact length, with no
    // padding — padding would be read as the superblock and fail the mount.
    let offset = squashfs_offset(&bytes);
    assert_eq!(&bytes[offset..offset + 4], b"hsqs", "squashfs magic");
    // And the inner ELF's GTK dependencies are inside the (uncompressed) image.
    for library in [b"libgtk-4.so.1".as_slice(), b"libgio-2.0.so.0".as_slice()] {
        assert!(
            bytes[offset..]
                .windows(library.len())
                .any(|window| window == library),
            "the sealed payload should record {} as DT_NEEDED",
            String::from_utf8_lossy(library)
        );
    }
}

/// The offset the AppImage runtime looks for its squashfs at: the end of its own
/// ELF, which for every published runtime equals the blob's length. Recomputed
/// here from the file rather than hardcoded so a blob bump does not silently
/// invalidate the test.
fn squashfs_offset(bytes: &[u8]) -> usize {
    let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) as usize;
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
    let shoff = u64_at(0x28);
    let shentsize = u16_at(0x3A);
    let shnum = u16_at(0x3C);
    let mut end = shoff + shentsize * shnum;
    for index in 0..shnum {
        let header = shoff + index * shentsize;
        let sh_type = u32::from_le_bytes(bytes[header + 4..header + 8].try_into().unwrap());
        if sh_type == 8 {
            continue; // SHT_NOBITS occupies no file space
        }
        end = end.max(u64_at(header + 0x18) + u64_at(header + 0x20));
    }
    end
}

#[test]
fn linux_app_debug_keeps_the_appdir_beside_the_appimage() {
    // plan-51-C §4.7: `--app-debug` implies `--app` and retains the payload the
    // seal consumed, so the AppDir can be inspected. plan-51-A §4.1's full layout
    // must be there.
    let project = temp_project("linux_app_dbg", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["--app-debug", "-target", TARGET]);
    assert!(ok, "build --app-debug failed:\n{stdout}\n{stderr}");

    let appimage = project.join("build/linux_app_dbg-glibc.AppImage");
    let appdir = project.join("build/linux_app_dbg-glibc.AppDir");
    assert!(appimage.is_file(), "--app-debug still emits the AppImage");
    assert!(appdir.is_dir(), "--app-debug keeps the AppDir");
    assert!(
        project.join("build/linux_app_dbg-musl.AppImage").is_file(),
        "--app-debug emits the musl AppImage too"
    );
    assert!(
        project.join("build/linux_app_dbg-musl.AppDir").is_dir(),
        "--app-debug keeps BOTH flavors' AppDirs (plan-56-B)"
    );

    // plan-56-A: each flavor's inner ELF names only its own libc world. This is
    // the ONLY observable difference — musl's loader absorbs the glibc compat
    // sonames, so a wrongly-linked musl binary runs identically.
    let musl_elf =
        std::fs::read(project.join("build/linux_app_dbg-musl.AppDir/usr/bin/linux_app_dbg"))
            .expect("musl ELF");
    let contains = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
    assert!(
        contains(&musl_elf, b"libc.musl-aarch64.so.1"),
        "the musl build must name the musl libc"
    );
    assert!(
        !contains(&musl_elf, b"libc.so.6"),
        "the musl build must NOT name libc.so.6 (plan-56-A)"
    );
    assert!(
        !contains(&musl_elf, b"libpthread.so.0"),
        "on musl, pthread lives in libc"
    );
    let glibc_elf =
        std::fs::read(project.join("build/linux_app_dbg-glibc.AppDir/usr/bin/linux_app_dbg"))
            .expect("glibc ELF");
    assert!(contains(&glibc_elf, b"libc.so.6"));
    assert!(!contains(&glibc_elf, b"libc.musl-"));

    // Every path plan-51-A §4.1 promises.
    assert!(appdir.join("usr/bin/linux_app_dbg").is_file());
    assert!(appdir.join("linux_app_dbg.desktop").is_file());
    assert!(appdir
        .join("usr/share/applications/linux_app_dbg.desktop")
        .is_file());
    assert!(appdir.join("linux_app_dbg.png").is_file());
    for size in [16, 32, 48, 64, 128, 256, 512] {
        assert!(
            appdir
                .join(format!(
                    "usr/share/icons/hicolor/{size}x{size}/apps/linux_app_dbg.png"
                ))
                .is_file(),
            "missing the {size}x{size} hicolor icon"
        );
    }
    assert_eq!(
        fs::read_link(appdir.join("AppRun")).expect("AppRun is a symlink"),
        Path::new("usr/bin/linux_app_dbg"),
        "AppRun must be a symlink to the real ELF, not a second copy of it"
    );
    assert_eq!(
        fs::read_link(appdir.join(".DirIcon")).expect(".DirIcon is a symlink"),
        Path::new("linux_app_dbg.png")
    );
    // A non-vendoring build carries no empty usr/lib/.
    assert!(!appdir.join("usr/lib").exists());

    let desktop = fs::read_to_string(appdir.join("linux_app_dbg.desktop")).expect("desktop");
    assert!(desktop.contains("\nType=Application\n"), "{desktop}");
    assert!(
        desktop.contains("\nIcon=linux_app_dbg\n"),
        "Icon= must be extension-less: appimagetool appends `.png` itself\n{desktop}"
    );
    assert!(
        desktop.contains("\nStartupWMClass=dev.mfbasic.linux_app_dbg\n"),
        "StartupWMClass must equal the GTK app id\n{desktop}"
    );
    assert!(
        desktop.contains("\nX-AppImage-Version=0.1.0\n"),
        "the manifest version reaches the .desktop\n{desktop}"
    );
    assert!(!desktop.contains("Terminal="), "{desktop}");

    // The inner ELF is what got sealed.
    let elf = fs::read(appdir.join("usr/bin/linux_app_dbg")).expect("read ELF");
    assert_eq!(&elf[0..4], b"\x7fELF");
    let sealed = fs::read(&appimage).expect("read AppImage");
    let offset = squashfs_offset(&sealed);
    assert!(
        sealed[offset..]
            .windows(elf.len())
            .any(|window| window == elf),
        "the AppDir's ELF must appear verbatim inside the uncompressed image"
    );
}

#[test]
fn linux_app_mode_carries_the_per_project_gtk_identity() {
    // plan-51-A §4.5: the GApplication id and the window title were compile-time
    // constants shared by every MFBASIC app; they are now derived from the
    // project name, which is what lets a `.desktop` file find the window.
    let project = temp_project("linux_app_id", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET, "-ncode"]);
    assert!(ok, "build -app -ncode failed:\n{stdout}\n{stderr}");
    let ncode = fs::read_to_string(project.join("linux_app_id.ncode")).expect("read ncode");

    let hex = |text: &str| -> String {
        let mut out = String::new();
        for byte in text.bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
        out.push_str("00");
        out
    };
    assert!(
        ncode.contains(&hex("dev.mfbasic.linux_app_id")),
        "the app id must be namespaced under the project name"
    );
    assert!(
        !ncode.contains(&hex("dev.mfbasic.app")),
        "the shared pre-plan-51 app id must be gone"
    );
    assert!(
        !ncode.contains(&hex("MFBASIC App")),
        "the shared pre-plan-51 window title must be gone"
    );
}

#[test]
fn linux_console_mode_still_emits_both_flavors() {
    let project = temp_project("linux_console", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-target", TARGET]);
    assert!(ok, "console build failed:\n{stdout}\n{stderr}");
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    assert_eq!(
        written.len(),
        2,
        "console mode emits glibc + musl flavors, got: {written:?}"
    );
    assert!(
        written
            .iter()
            .any(|p| p.ends_with("linux_console-glibc.out"))
            && written
                .iter()
                .any(|p| p.ends_with("linux_console-musl.out")),
        "console mode should emit -glibc.out and -musl.out, got: {written:?}"
    );
}

// --- bug-539: the positioned `term::` drawing members in GTK app mode ---------

/// A GTK app-mode program that exercises every positioned `term::` member, with a
/// dash style, a dot style and `Double` so a wrong ordinal→glyph mapping cannot hide
/// behind `Light` (the whole style table is selected at emit time).
const TERM_DRAW_SOURCE: &str = "IMPORT io\nIMPORT term\nIMPORT color\n\nSUB main()\n  \
     term::on()\n  term::setForeground(color::rgb(0, 255, 0))\n  \
     term::moveTo(2, 4)\n  io::write(\"oracle\")\n  \
     term::drawHLine(term::LineStyle.LightDash, 5, 1, 12)\n  \
     term::drawVLine(term::LineStyle.HeavyDot, 6, 3, 10)\n  \
     term::drawBox(term::LineStyle.Double, 7, 2, 11, 20)\n  \
     term::fillRect(term::FillStyle.Medium, 8, 4, 10, 16)\n  \
     term::drawText(9, 6, \"app\")\n  term::drawGlyph(10, 7, 9731)\n  \
     term::sync()\n  term::off()\nEND SUB\n";

/// Read the `-ncode` plan of a `--app` build as JSON, keyed by function symbol.
fn app_ncode_functions(name: &str, source: &str) -> serde_json::Map<String, serde_json::Value> {
    let project = temp_project(name, source);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET, "-ncode"]);
    assert!(ok, "build -app -ncode failed:\n{stdout}\n{stderr}");
    let text = fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode");
    let plan: serde_json::Value = serde_json::from_str(&text).expect("ncode is JSON");
    let mut map = serde_json::Map::new();
    for function in plan["functions"].as_array().expect("functions array") {
        let symbol = function["symbol"].as_str().expect("symbol").to_string();
        map.insert(symbol, function["instructions"].clone());
    }
    map
}

/// bug-539: `term::drawHLine`/`drawVLine`/`drawBox`/`fillRect`/`drawGlyph`/`drawText`
/// had no GTK arm, so the app dispatcher returned `None` and each member fell through
/// to the CONSOLE emitter. That emitter opens by loading the console shadow-grid
/// header out of the arena term-state (slot 48 off the pinned arena register) and
/// no-ops when it is null — and the only writer of that slot is the *console*
/// `term::on`, which a GTK app build never runs. All six therefore returned `OK`
/// having drawn nothing, silently, for the life of the program.
///
/// The fix is asserted two ways, because either alone is satisfiable by accident:
/// each member must CALL its GTK worker helper, and no member may read the arena
/// register at all — reading it is exactly the console fall-through's first act.
#[test]
fn linux_app_mode_positioned_term_members_reach_the_gtk_backend() {
    let functions = app_ncode_functions("linux_app_term_draw", TERM_DRAW_SOURCE);
    for helper in [
        "_mfb_gtkapp_term_stamp",
        "_mfb_gtkapp_term_run",
        "_mfb_gtkapp_term_hline",
        "_mfb_gtkapp_term_vline",
        "_mfb_gtkapp_term_box",
        "_mfb_gtkapp_term_fill",
        "_mfb_gtkapp_term_glyph",
        "_mfb_gtkapp_term_draw_text",
    ] {
        assert!(
            functions.contains_key(helper),
            "a term:: app build must emit the worker-side helper {helper}"
        );
    }
    for (member, helper) in [
        ("drawHLine", "_mfb_gtkapp_term_hline"),
        ("drawVLine", "_mfb_gtkapp_term_vline"),
        ("drawBox", "_mfb_gtkapp_term_box"),
        ("fillRect", "_mfb_gtkapp_term_fill"),
        ("drawGlyph", "_mfb_gtkapp_term_glyph"),
        ("drawText", "_mfb_gtkapp_term_draw_text"),
    ] {
        let symbol = format!("_mfb_rt_term_term_{member}");
        let body = functions
            .get(&symbol)
            .unwrap_or_else(|| panic!("{symbol} is emitted"))
            .as_array()
            .expect("instructions");
        assert!(
            body.iter().any(|instruction| {
                instruction["op"] == "bl" && instruction["target"] == serde_json::json!(helper)
            }),
            "term::{member} must call the GTK helper {helper}, not fall through to the \
             console emitter; body was:\n{body:#?}"
        );
        assert!(
            !body
                .iter()
                .any(|instruction| instruction["base"] == serde_json::json!("x19")),
            "term::{member} must not read the arena term-state: loading the console \
             shadow-grid header there is the fall-through that silently no-ops in an \
             app build (bug-539)"
        );
    }
}

/// The positive half of the same change: `term::drawText` is the `io::write` grid
/// walk under a different edge contract, emitted as a second specialization of ONE
/// emitter rather than a second hand-written walk. Pin what makes the two different
/// so a later edit cannot quietly collapse them — and pin that the write path still
/// does everything it did before, which is the property the specialization risked.
#[test]
fn linux_app_mode_draw_text_specializes_the_write_path_without_taking_its_cursor() {
    let functions = app_ncode_functions("linux_app_term_walk", TERM_DRAW_SOURCE);
    let write = functions["_mfb_gtkapp_term_write"]
        .as_array()
        .expect("write instructions");
    let draw_text = functions["_mfb_gtkapp_term_draw_text"]
        .as_array()
        .expect("draw_text instructions");
    let calls_scroll = |body: &[serde_json::Value]| {
        body.iter().any(|instruction| {
            instruction["op"] == "bl"
                && instruction["target"] == serde_json::json!("_mfb_gtkapp_term_scroll")
        })
    };
    // `io::write` wraps at the right edge and scrolls at the bottom, and commits the
    // shadow cursor on the way out. All three are still there.
    assert!(
        calls_scroll(write),
        "the write path must still scroll at the bottom of the grid"
    );
    assert!(
        write
            .iter()
            .any(|instruction| instruction["name"] == serde_json::json!("tw_store")),
        "the write path must still commit the shadow cursor"
    );
    // `drawText` places a run at an absolute cell on ONE row: it never scrolls and
    // never moves the cursor.
    assert!(
        !calls_scroll(draw_text),
        "drawText must not scroll: it is clipped to its row, not wrapped"
    );
    assert!(
        !draw_text
            .iter()
            .any(|instruction| instruction["name"] == serde_json::json!("tw_store")),
        "drawText must leave the shadow cursor where the program left it"
    );
    // Both are the same walk: the combining-mark fold and the wide-cell sentinel are
    // emitted from the one emitter, so neither can drift from the other.
    for (label, body) in [("write", write), ("drawText", draw_text)] {
        assert!(
            body.iter()
                .any(|instruction| { instruction["value"] == serde_json::json!("4294967295") }),
            "{label} must stamp the GTK_WIDE_TRAIL sentinel for a wide cluster"
        );
    }
}

/// The other positive pin: a GTK app that never touches `term::` must not grow any
/// of the new bodies. The helpers are gated on `AppEntrySpec::uses_term`, so the
/// function set of an ordinary transcript app is exactly what it was.
#[test]
fn linux_app_mode_without_term_emits_no_positioned_helpers() {
    let functions = app_ncode_functions("linux_app_no_term", APP_SOURCE);
    for helper in [
        "_mfb_gtkapp_term_stamp",
        "_mfb_gtkapp_term_run",
        "_mfb_gtkapp_term_hline",
        "_mfb_gtkapp_term_vline",
        "_mfb_gtkapp_term_box",
        "_mfb_gtkapp_term_fill",
        "_mfb_gtkapp_term_glyph",
        "_mfb_gtkapp_term_draw_text",
    ] {
        assert!(
            !functions.contains_key(helper),
            "a term-free GTK app must not emit {helper}"
        );
    }
}

/// Found while proving bug-539 on 2228: the draw callback computed the snapshot
/// EGC-pool slot with `state_array(SCRATCH[0], ST_TERM_SNAP_POOL)`. For an offset
/// past the add immediate that helper stages the offset in `SCRATCH[0]` itself, so
/// the destination was overwritten with the offset and the following add DOUBLED it
/// — `2 * ST_TERM_SNAP_POOL`, an absolute low address. The GTK main thread SIGSEGV'd
/// the first time it rendered a multi-scalar grapheme cluster (verified under gdb on
/// 2228: deterministic before, clean after).
///
/// The emitter now refuses that aliasing, and this pins the whole class: in no GTK
/// app body may an `adrp` be immediately followed by a `mov_imm` into the same
/// register, which is precisely the shape that throws a symbol address away.
#[test]
fn linux_app_mode_no_symbol_address_is_overwritten_by_the_offset_it_needs() {
    let functions = app_ncode_functions("linux_app_addr", TERM_DRAW_SOURCE);
    for (symbol, body) in &functions {
        let body = body.as_array().expect("instructions");
        // The address materializes as `adrp` + `add_pageoff` on AArch64 and folds to
        // the `adrp` alone once x86 selection has run, so accept either shape and
        // look for the `mov_imm` that lands on the register they just defined.
        for window in body.windows(3) {
            let killed = match window[1]["op"].as_str() {
                Some("add_pageoff") if window[1]["dst"] == window[0]["dst"] => &window[2],
                _ => &window[1],
            };
            if window[0]["op"] == "adrp"
                && killed["op"] == "mov_imm"
                && killed["dst"] == window[0]["dst"]
            {
                panic!(
                    "{symbol}: the address `adrp {dst}` materialized is overwritten by \
                     `mov_imm {dst}` before anything reads it — the symbol is discarded and \
                     the following add doubles the immediate instead of indexing off the base",
                    dst = window[0]["dst"]
                );
            }
        }
    }
}

/// The GTK app helpers are hand-written machine code emitted below the register
/// allocator, and `linux-aarch64` GTK app mode cannot be executed from this host or
/// from CI — so the structural invariants a runtime crash would otherwise surface
/// are asserted here instead, on the AArch64 plan where the neutral `LOCAL`/`SCRATCH`
/// tokens realize to physical `x19`–`x28` with hand-tracked liveness rather than
/// being coloured by the allocator (that only happens on x86-64,
/// `finalize_x86_app_function`).
///
/// Three properties, each of which is a real way a hand-written body goes wrong:
/// a branch to a label the function does not define (a link-time or silent-fallthrough
/// bug), a frame that is not released on some return path, and any use of `x19` —
/// the pinned arena base, which a worker-thread helper must never treat as scratch
/// without saving it.
#[test]
fn linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64() {
    let functions = app_ncode_functions("linux_app_term_shape", TERM_DRAW_SOURCE);
    for symbol in [
        "_mfb_gtkapp_term_stamp",
        "_mfb_gtkapp_term_run",
        "_mfb_gtkapp_term_hline",
        "_mfb_gtkapp_term_vline",
        "_mfb_gtkapp_term_box",
        "_mfb_gtkapp_term_fill",
        "_mfb_gtkapp_term_glyph",
        "_mfb_gtkapp_term_draw_text",
    ] {
        let body = functions[symbol].as_array().expect("instructions");
        let labels: Vec<&str> = body
            .iter()
            .filter(|instruction| instruction["op"] == "label")
            .filter_map(|instruction| instruction["name"].as_str())
            .collect();
        let mut depth: i64 = 0;
        for instruction in body {
            let op = instruction["op"].as_str().unwrap_or_default();
            if let Some(target) = instruction["target"].as_str() {
                // `bl` targets a symbol, every other branch targets a local label.
                if op != "bl" {
                    assert!(
                        labels.contains(&target),
                        "{symbol}: branch to '{target}', which the function never defines"
                    );
                }
            }
            let imm = || {
                instruction["imm"]
                    .as_str()
                    .and_then(|text| text.parse::<i64>().ok())
                    .expect("stack adjust immediate")
            };
            match op {
                "sub_sp" => depth += imm(),
                "add_sp" => depth -= imm(),
                "ret" => assert_eq!(
                    depth, 0,
                    "{symbol}: returns with {depth} bytes of frame still carved"
                ),
                _ => {}
            }
        }
        // `x19` is the pinned arena base on AArch64. A worker-thread helper may only
        // borrow it as scratch if it saves and restores it through its own frame —
        // which is the convention the pre-existing write helper uses for the
        // code-point byte length, and which `drawText` inherits by being that same
        // emitter. Borrowing it WITHOUT the save is how the arena pointer disappears
        // out from under every later allocation on the worker thread.
        let touches = |predicate: &dyn Fn(&serde_json::Value) -> bool| {
            body.iter().any(|instruction| predicate(instruction))
        };
        let uses_x19 = touches(&|instruction| {
            instruction
                .as_object()
                .expect("instruction object")
                .iter()
                .any(|(_, value)| value == &serde_json::json!("x19"))
        });
        if uses_x19 {
            assert!(
                touches(&|instruction| instruction["op"] == "str_u64"
                    && instruction["src"] == serde_json::json!("x19"))
                    && touches(&|instruction| instruction["op"] == "ldr_u64"
                        && instruction["dst"] == serde_json::json!("x19")),
                "{symbol}: borrows the pinned arena register x19 without saving and \
                 restoring it through its own frame"
            );
        }
        assert_eq!(depth, 0, "{symbol}: frame is not balanced at the end");
        // Every frame this fix introduces is a multiple of 16 so the stack stays
        // 16-aligned at the calls these helpers make (AAPCS64 and SysV both require
        // it, and x86 selection brackets these bodies assuming the parity holds).
        for instruction in body {
            if instruction["op"] == "sub_sp" {
                let bytes: i64 = instruction["imm"]
                    .as_str()
                    .and_then(|text| text.parse().ok())
                    .expect("imm");
                assert_eq!(
                    bytes % 16,
                    0,
                    "{symbol}: frame {bytes} breaks 16-byte alignment"
                );
            }
        }
    }
}
