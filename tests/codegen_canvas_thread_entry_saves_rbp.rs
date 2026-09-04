//! The graphics thread's entry must hand `rbp` back to `pthread`'s `start_thread`.
//!
//! MFBASIC's internal call convention needs eight argument registers and SysV x86-64
//! has six, so `CALL_ARGS` extends with `rax` and `rbp` (bug-296). That is agreed
//! between an MFB caller and an MFB callee, and it makes `rbp` — **callee-saved under
//! SysV** — a register that ordinary MFB code writes. `__canvas_geoDistance` takes 22
//! parameters and `__canvas_drawGeometry` stages `rbp` for it six times.
//!
//! So every boundary a non-MFB caller enters through has to save it.
//! `_mfb_rt_canvas_graphics_entry` is one: `pthread_create` calls it, and glibc's
//! `start_thread` keeps its own frame pointer in `rbp` across that call. It did not
//! save it, and returned with `rbp = 0x404e000000000000` — not a pointer at all, but
//! the double `60.0`, the `radius` of a circle in the scene being drawn. Then
//! `start_thread` ran `mov -0x98(%rbp),%rax` and died with SIGBUS. That single
//! register was 68 of ~90 canvas tests on CI's x86_64 Linux rows, 55 SIGBUS and 13
//! SIGSEGV, the two alternating as one wild address lands in different places.
//!
//! ## Why this is a codegen-inspection test rather than a behavioural one
//!
//! The behavioural proof only exists on one platform, and not the developer's.
//! AArch64 passes eight arguments in registers (`x7`, caller-saved), so macOS and the
//! aarch64 rows are clean however this is emitted; and on x86-64 whether a clobbered
//! `rbp` is ever *dereferenced* depends on the libc's own path after the start
//! routine returns — it faults on ubuntu-24.04 runners and stays silent on Debian 13,
//! on Alpine, and in a container carrying the same GTK, across dozens of runs. A test
//! that can only fail on one CI row is a test that gets diagnosed as a flake. This
//! reads the emitted plan instead, so it fails on every host.
//!
//! It also cannot be established by reading the Rust source: the source names a
//! neutral token and the physical register is chosen elsewhere. An earlier attempt at
//! this fix saved `abi::SCRATCH[10]` believing it was `rbp`; it is `x20`, which is
//! `rbx`, and the fault came back bit-for-bit identical. The emitted plan is the only
//! artifact where the register carries the name the machine will use.

mod common;

use serde_json::Value;
use std::process::Command;

/// The scene needs one shape whose geometry reaches `__canvas_geoDistance`, which is
/// what makes the caller stage an eighth argument.
const SOURCE: &str = r#"IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET dot AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 400.0, radius := 60.0, paint := canvas::fill(canvas::rgb(40, 200, 120))]
  canvas::present([dot])
END SUB
"#;

/// A `--app --ncode` build for an explicit target. `common::build_ncode` has no
/// `-app`, and `canvas` is importable only in app mode.
fn app_ncode(name: &str, target: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg("-ncode")
        .arg("-target")
        .arg(target)
        .arg(&project)
        .output()
        .expect("run mfb build -app -ncode");
    assert!(
        output.status.success(),
        "mfb build -app -ncode -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let text =
        std::fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode dump");
    let plan: Value = serde_json::from_str(&text).expect("parse ncode json");
    let _ = std::fs::remove_dir_all(&project);
    plan
}

fn instructions<'a>(plan: &'a Value, symbol: &str) -> &'a Vec<Value> {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"] == symbol)
        .unwrap_or_else(|| panic!("no {symbol} in the code plan"))["instructions"]
        .as_array()
        .expect("instructions array")
}

#[test]
fn the_graphics_thread_entry_saves_and_restores_rbp_on_x86_64() {
    let plan = app_ncode("canvas_entry_rbp", "linux-x86_64", SOURCE);
    let body = instructions(&plan, "_mfb_rt_canvas_graphics_entry");

    let saved = body
        .iter()
        .any(|i| i["op"] == "str_u64" && i["src"] == "rbp");
    let restored = body
        .iter()
        .any(|i| i["op"] == "ldr_u64" && i["dst"] == "rbp");

    assert!(
        saved && restored,
        "the graphics thread entry must save AND restore rbp (saved={saved}, \
         restored={restored}): pthread's start_thread keeps its frame pointer there \
         across the call, and MFB stages an 8th argument into rbp. Without both, the \
         thread returns with a geometry double in rbp and start_thread takes SIGBUS. \
         Emitted body:\n{}",
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    );
}

#[test]
fn the_hazard_this_guards_is_still_real() {
    // The guard above is only worth having while MFB actually stages an argument into
    // rbp. If the internal convention ever stops doing that, this fails and says so —
    // rather than leaving a save nobody can explain and a test that passes vacuously.
    let plan = app_ncode("canvas_entry_rbp_hazard", "linux-x86_64", SOURCE);
    let staged = plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter(|f| {
            f["instructions"]
                .as_array()
                .is_some_and(|body| body.iter().any(|i| i["op"] == "mov" && i["dst"] == "rbp"))
        })
        .count();

    assert!(
        staged > 0,
        "no emitted function writes rbp, so the 8th-argument staging this test guards \
         against no longer happens. Either the internal call convention changed — in \
         which case the save in `emit_graphics_trampoline` and the `.ai/arch-abi.md` \
         section on it should be revisited — or this scene stopped reaching a call \
         wide enough to stage one.",
    );
}
