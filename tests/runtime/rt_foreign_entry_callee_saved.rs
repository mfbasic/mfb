//! bug-982 / bug-993: every function a foreign caller enters returns with the
//! caller's callee-saved registers intact.
//!
//! The OS calls back into code this compiler emits: GTK signal handlers, the
//! Objective-C IMPs AppKit dispatches to, the Win32 window procedure, thread start
//! routines, the OpenSSL verify callback. Each is an ordinary C-ABI callee, so it
//! must hand back AAPCS64 `x19`-`x28`/`d8`-`d15`, SysV `rbx rbp r12`-`r15`, Win64
//! `rbx rbp rdi rsi r12`-`r15` exactly as it found them. The foreign caller keeps
//! its own state there across the call: GLib's closure marshaller kept a pointer in
//! `x19` and faulted on the mouse handler's mode word (bug-982), and
//! `-[NSWindow sendEvent:]` released whatever the mouse IMP left in `x19` (bug-993).
//!
//! These bodies are hand-written and sit below the register allocator, so their
//! frames declare no save set and nothing else checks them. This test re-derives
//! the property from the `-ncode` dump of each target, by dataflow:
//!
//! - every location (register or stack slot) carries either "the entry value of
//!   callee-saved register R" or "unknown". A store to `sp` copies the value into a
//!   slot, a load from `sp` copies it back, a `mov` copies it, any other write
//!   makes the destination unknown. Paths meet by agreement.
//! - a `bl` to a function in the dump clobbers what THAT function fails to restore
//!   (its escape set, computed by the same analysis to a fixpoint), so a callback
//!   that calls MFB code inherits whatever that code leaves changed — `rbp`, which
//!   an internal eight-argument call stages on x86-64 (`.ai/arch-abi.md`), and the
//!   x86 scratch homes `rbx`/`r13` the hand-written runtime helpers use. A `bl` to
//!   an imported C function follows the platform ABI and clobbers none.
//! - at every reachable `ret`, each callee-saved register must hold its entry value.
//!   A routine that never returns (`_main`, a finish helper that parks) has no
//!   reachable `ret` and passes.
//!
//! What counts as foreign-entered: every function whose address the program takes
//! (`adrp`/`add_pageoff` of a text symbol — that address is handed to the OS), minus
//! the MFB function values MFB code itself calls through a pointer, plus every
//! hand-written app-backend body (`_mfb_macapp_*`, `_mfb_gtkapp_*`, `_mfb_winapp_*`).

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, build_ncode_app, unique_nonce};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The canvas + term mouse surfaces with a mode switch: every macOS mouse IMP
/// (both views), the macOS canvas blit, the canvas graphics thread, and the
/// Win32 window procedure's mouse arms.
const MOUSE_SURFACES: &str = "\
IMPORT app\n\
IMPORT canvas\n\
IMPORT io\n\
IMPORT term\n\
IMPORT vector\n\
FUNC pixels() AS Nothing\n\
  app::setMode(app::Mode.Canvas)\n\
  canvas::enableMouse(TRUE)\n\
  LET e AS canvas::MouseEvent = canvas::pollMouse()\n\
  IF e.kind <> canvas::MouseKind.None THEN\n\
    io::print(\"canvas \" & toString(e.position.x) & \",\" & toString(e.position.y))\n\
  END IF\n\
  canvas::enableMouse(FALSE)\n\
  RETURN NOTHING\n\
  TRAP(err)\n\
    RETURN NOTHING\n\
  END TRAP\n\
END FUNC\n\
FUNC cells() AS Nothing\n\
  app::setMode(app::Mode.Console)\n\
  term::enableMouse(TRUE)\n\
  LET e AS term::MouseEvent = term::pollMouse()\n\
  IF e.kind <> term::MouseKind.None THEN\n\
    io::print(\"term \" & toString(e.row) & \",\" & toString(e.column))\n\
  END IF\n\
  term::enableMouse(FALSE)\n\
  RETURN NOTHING\n\
  TRAP(err)\n\
    RETURN NOTHING\n\
  END TRAP\n\
END FUNC\n\
FUNC main() AS Nothing\n\
  cells()\n\
  pixels()\n\
  app::setTitle(\"Window \" & app::getTitle())\n\
  app::setFullscreen(NOT app::getFullscreen())\n\
END FUNC\n";

/// bug-982's reproduction: a console-mode program that turns the mouse on, which
/// is what attaches the GTK mouse controllers at activation.
const MOUSE_CONSOLE: &str = "\
IMPORT term\n\
IMPORT io\n\
IMPORT os\n\
FUNC main() AS Nothing\n\
  term::on()\n\
  term::enableMouse(TRUE)\n\
  MUT i AS Integer = 0\n\
  WHILE i < 40\n\
    LET ev AS term::MouseEvent = term::pollMouse()\n\
    IF ev.kind <> term::MouseKind.None THEN\n\
      io::print(\"event row=\" & toString(ev.row))\n\
    END IF\n\
    os::sleep(100)\n\
    i = i + 1\n\
  END WHILE\n\
  term::off()\n\
END FUNC\n";

const ALL_TARGETS: &[&str] = &[
    "macos-aarch64",
    "linux-aarch64",
    "linux-x86_64",
    "windows-x86_64",
    "linux-riscv64",
];

/// Routines that call MFB code through a register: the worker a thread runs is
/// a function value, so the trampoline's `blr` reaches compiled MFB code. Every
/// other `blr` in an audited body calls a C function pointer (OpenSSL through
/// `dlsym`, an Objective-C block) and follows the platform ABI.
const MFB_DISPATCHERS: &[&str] = &["_mfb_rt_thread_trampoline"];

/// The callee-saved registers every compiled MFB function preserves: the ones
/// the register allocator may hand out, which a frame saves when it does
/// (mirrors `INT_ALLOCATABLE`/`FP_ALLOCATABLE` in `src/arch/*/regmodel.rs`).
///
/// The REST of the callee-saved bank is outside that guarantee. MFB code pins
/// the arena, current-thread and closure-environment registers there, stages
/// internal call arguments there on x86-64 (`rbp`; Win64 also `rdi`/`rsi`), and
/// its hand-written runtime helpers use the x86 scratch homes `rbx`/`r13`. So a
/// foreign entry that runs MFB code must save every one of those itself, whatever
/// the program in hand happens to clobber.
fn allocator_preserved(target: &str) -> BTreeSet<String> {
    let names: Vec<String> = match target {
        "macos-aarch64" | "linux-aarch64" => (21..=27)
            .map(|i| format!("x{i}"))
            .chain((8..=15).map(|i| format!("d{i}")))
            .collect(),
        "linux-riscv64" => ["s1", "s3", "s4", "s5", "s6", "s7", "s8", "s9"]
            .iter()
            .map(|s| s.to_string())
            .chain((0..=11).map(|i| format!("fs{i}")))
            .collect(),
        "linux-x86_64" => vec!["r12".to_string(), "r14".to_string()],
        "windows-x86_64" => ["r12".to_string(), "r14".to_string()]
            .into_iter()
            .chain((6..=14).map(|i| format!("xmm{i}")))
            .collect(),
        other => panic!("no allocator table for {other}"),
    };
    names.into_iter().collect()
}

/// The callee-saved registers of a target's C ABI.
fn callee_saved(target: &str) -> BTreeSet<String> {
    let names: Vec<String> = match target {
        "macos-aarch64" | "linux-aarch64" => (19..=28)
            .map(|i| format!("x{i}"))
            .chain((8..=15).map(|i| format!("d{i}")))
            .collect(),
        "linux-riscv64" => (0..=11)
            .map(|i| format!("s{i}"))
            .chain((0..=11).map(|i| format!("fs{i}")))
            .collect(),
        "linux-x86_64" => ["rbx", "rbp", "r12", "r13", "r14", "r15"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        // `xmm15` is left out: it is the encoder's private SSE scratch, written
        // inside instruction encodings and so invisible in the dump (bug-994).
        "windows-x86_64" => ["rbx", "rbp", "rdi", "rsi", "r12", "r13", "r14", "r15"]
            .iter()
            .map(|s| s.to_string())
            .chain((6..=14).map(|i| format!("xmm{i}")))
            .collect(),
        other => panic!("no ABI table for {other}"),
    };
    names.into_iter().collect()
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Loc {
    Reg(String),
    Slot(i64),
}

/// The abstract state before an instruction: the `sp` depth below the entry `sp`,
/// and which locations still hold which callee-saved register's entry value.
#[derive(Clone, PartialEq, Debug)]
struct State {
    depth: i64,
    values: BTreeMap<Loc, String>,
}

fn field<'a>(instruction: &'a Value, key: &str) -> Option<&'a str> {
    instruction.get(key).and_then(Value::as_str)
}

fn is_stack_base(instruction: &Value) -> bool {
    matches!(field(instruction, "base"), Some("sp" | "rsp"))
}

fn offset(instruction: &Value) -> i64 {
    field(instruction, "offset")
        .map(|o| o.parse::<i64>().expect("numeric sp offset"))
        .unwrap_or(0)
}

fn immediate(instruction: &Value) -> i64 {
    field(instruction, "imm")
        .map(|o| o.parse::<i64>().expect("numeric sp adjustment"))
        .expect("sp adjustment immediate")
}

/// The callee-saved registers `function` may return with changed, given the
/// escape sets of the functions it calls.
fn escapes(
    function: &Value,
    callee_saved: &BTreeSet<String>,
    escape: &HashMap<String, BTreeSet<String>>,
    dispatch_clobber: Option<&BTreeSet<String>>,
) -> BTreeSet<String> {
    let instructions = function["instructions"].as_array().expect("instructions");
    let labels: HashMap<&str, usize> = instructions
        .iter()
        .enumerate()
        .filter(|(_, i)| field(i, "op") == Some("label"))
        .filter_map(|(k, i)| Some((field(i, "name")?, k)))
        .collect();
    let entry = State {
        depth: 0,
        values: callee_saved
            .iter()
            .map(|r| (Loc::Reg(r.clone()), r.clone()))
            .collect(),
    };
    let mut states: HashMap<usize, State> = HashMap::from([(0, entry)]);
    let mut work = vec![0usize];
    let mut changed = BTreeSet::new();
    while let Some(k) = work.pop() {
        let mut state = states[&k].clone();
        let instruction = &instructions[k];
        let op = field(instruction, "op").expect("op");
        let clobber = |state: &mut State, register: &str| {
            state.values.remove(&Loc::Reg(register.to_string()));
        };
        match op {
            "sub_sp" => state.depth += immediate(instruction),
            "add_sp" => state.depth -= immediate(instruction),
            // plan-171-A: an AArch64 pair is its two accesses, `width` apart; a
            // pre-indexed one moves `sp` first and a post-indexed one after.
            _ if (op.starts_with("st") || op.starts_with("ld")) && is_stack_base(instruction) => {
                let store = op.starts_with("st");
                let pair = op == "stp" || op == "ldp";
                let mode = field(instruction, "addressing").unwrap_or("offset");
                let mut at = offset(instruction);
                if mode == "pre" {
                    state.depth -= at;
                }
                if mode != "offset" {
                    at = 0;
                }
                let width = field(instruction, "width").map_or(8, |w| w.parse().expect("width"));
                let roles: &[&str] = match (store, pair) {
                    (true, true) => &["src", "src2"],
                    (true, false) => &["src"],
                    (false, true) => &["dst", "dst2"],
                    (false, false) => &["dst"],
                };
                for (half, role) in roles.iter().enumerate() {
                    let slot = Loc::Slot(at + half as i64 * width - state.depth);
                    let register = field(instruction, role).expect("pair register");
                    if store {
                        match state.values.get(&Loc::Reg(register.to_string())).cloned() {
                            Some(v) => state.values.insert(slot, v),
                            None => state.values.remove(&slot),
                        };
                    } else {
                        match state.values.get(&slot).cloned() {
                            Some(v) => state.values.insert(Loc::Reg(register.to_string()), v),
                            None => state.values.remove(&Loc::Reg(register.to_string())),
                        };
                    }
                }
                if mode == "post" {
                    state.depth -= offset(instruction);
                }
            }
            "mov" | "fmov_d_from_d" => {
                let dst = field(instruction, "dst").expect("mov dst");
                let src = field(instruction, "src").expect("mov src");
                match state.values.get(&Loc::Reg(src.to_string())).cloned() {
                    Some(v) => state.values.insert(Loc::Reg(dst.to_string()), v),
                    None => state.values.remove(&Loc::Reg(dst.to_string())),
                };
            }
            "bl" => {
                let target = field(instruction, "target").expect("bl target");
                for register in escape.get(target).into_iter().flatten() {
                    clobber(&mut state, register);
                }
            }
            "blr" => {
                for register in dispatch_clobber.into_iter().flatten() {
                    clobber(&mut state, register);
                }
            }
            _ => {
                for key in ["dst", "dst2", "carry_out"] {
                    if let Some(register) = field(instruction, key) {
                        clobber(&mut state, register);
                    }
                }
            }
        }
        if op == "ret" {
            for register in callee_saved {
                if state.values.get(&Loc::Reg(register.clone())) != Some(register) {
                    changed.insert(register.clone());
                }
            }
            continue;
        }
        let mut successors = Vec::new();
        match op {
            "branch_self" | "br" => {}
            "b" => successors.extend(field(instruction, "target").and_then(|t| labels.get(t))),
            _ => {
                if k + 1 < instructions.len() {
                    successors.push(k + 1);
                }
                if op != "bl" {
                    successors.extend(field(instruction, "target").and_then(|t| labels.get(t)));
                }
            }
        }
        for next in successors {
            let merged = match states.get(&next) {
                None => state.clone(),
                Some(old) => State {
                    depth: old.depth,
                    values: old
                        .values
                        .iter()
                        .filter(|(loc, v)| state.values.get(*loc) == Some(*v))
                        .map(|(l, v)| (l.clone(), v.clone()))
                        .collect(),
                },
            };
            if states.get(&next) != Some(&merged) {
                states.insert(next, merged);
                work.push(next);
            }
        }
    }
    changed
}

fn is_mfb_function_value(symbol: &str) -> bool {
    [
        "_mfb_fn_",
        "_mfb_ifn_",
        "_mfb_rt_closure_env_",
        "_mfb_rt_thread_copy_value_",
        "_mfb_rt_thread_drop_value_",
    ]
    .iter()
    .any(|prefix| symbol.starts_with(prefix))
}

fn is_app_backend_body(symbol: &str) -> bool {
    ["_mfb_macapp_", "_mfb_gtkapp_", "_mfb_winapp_"]
        .iter()
        .any(|prefix| symbol.starts_with(prefix))
}

/// Audit one `-ncode` dump. Returns the audited symbols; panics, naming every
/// offender, if any of them returns with a callee-saved register changed.
fn audit(ncode: &Value, target: &str, what: &str) -> BTreeSet<String> {
    let callee_saved = callee_saved(target);
    let functions: HashMap<String, &Value> = ncode["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .filter_map(|f| Some((f["symbol"].as_str()?.to_string(), f)))
        .collect();
    let mut taken = BTreeSet::new();
    for function in functions.values() {
        for instruction in function["instructions"].as_array().expect("instructions") {
            if matches!(field(instruction, "op"), Some("adrp" | "add_pageoff")) {
                if let Some(symbol) = field(instruction, "symbol") {
                    if functions.contains_key(symbol) {
                        taken.insert(symbol.to_string());
                    }
                }
            }
        }
    }
    let audited: BTreeSet<String> = functions
        .keys()
        .filter(|s| (taken.contains(*s) && !is_mfb_function_value(s)) || is_app_backend_body(s))
        .cloned()
        .collect();

    // Escape sets for every function, to a fixpoint (recursion makes a single
    // pass unsound).
    let mut escape: HashMap<String, BTreeSet<String>> = functions
        .keys()
        .map(|s| (s.clone(), BTreeSet::new()))
        .collect();
    loop {
        let mut grew = false;
        for (symbol, function) in &functions {
            let found = escapes(function, &callee_saved, &escape, None);
            let entry = escape.get_mut(symbol).expect("known symbol");
            if !found.is_subset(entry) {
                entry.extend(found);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    // An audited entry that runs compiled MFB code is held to the convention, not
    // to what this program's MFB code happens to clobber: a call into
    // `_mfb_fn_*`/`_mfb_ifn_*`, or a dispatcher's `blr` to a worker, may change any
    // callee-saved register outside the allocator's preserved set.
    let preserved = allocator_preserved(target);
    let outside: BTreeSet<String> = callee_saved.difference(&preserved).cloned().collect();
    let mut convention = escape.clone();
    for (symbol, set) in convention.iter_mut() {
        if symbol.starts_with("_mfb_fn_") || symbol.starts_with("_mfb_ifn_") {
            set.extend(outside.iter().cloned());
        }
    }
    let offenders: Vec<String> = audited
        .iter()
        .filter_map(|s| {
            let dispatch = MFB_DISPATCHERS.contains(&s.as_str()).then_some(&outside);
            let changed = escapes(functions[s], &callee_saved, &convention, dispatch);
            (!changed.is_empty()).then(|| format!("{s} {changed:?}"))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "{target} ({what}): {} foreign-entered function(s) return with a callee-saved \
         register changed:\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
    audited
}

fn app_project(name: &str, source: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mfb_{name}_{}", unique_nonce()));
    std::fs::create_dir_all(root.join("src")).expect("create project");
    std::fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"mode\":\"app\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\
             \"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    std::fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create copy");
    for entry in std::fs::read_dir(from).expect("read fixture") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if entry.file_name() == "golden" {
            continue;
        }
        if path.is_dir() {
            copy_dir(&path, &to.join(entry.file_name()));
        } else {
            std::fs::copy(&path, to.join(entry.file_name())).expect("copy file");
        }
    }
}

/// A byte-identity fixture copied out of the tree, so the dump lands in a
/// scratch directory.
fn fixture_project(fixture: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mfb_{fixture}_{}", unique_nonce()));
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/byte-identity")
            .join(fixture),
        &root,
    );
    root
}

fn audit_app(target: &str) -> BTreeSet<String> {
    let mut audited = BTreeSet::new();
    for (name, source) in [
        ("cb_mouse_surfaces", MOUSE_SURFACES),
        ("cb_mouse_console", MOUSE_CONSOLE),
    ] {
        let project = app_project(name, source);
        let ncode = build_ncode_app(&project, target, name);
        let _ = std::fs::remove_dir_all(&project);
        audited.extend(audit(&ncode, target, name));
    }
    audited
}

fn assert_audited(audited: &BTreeSet<String>, target: &str, expected: &[&str]) {
    for symbol in expected {
        assert!(
            audited.contains(*symbol),
            "{target}: `{symbol}` was not audited; audited: {audited:?}"
        );
    }
}

#[test]
fn macos_app_imps_preserve_callee_saved() {
    let audited = audit_app("macos-aarch64");
    assert_audited(
        &audited,
        "macos-aarch64",
        &[
            "_mfb_macapp_canvas_scrollWheel",
            "_mfb_macapp_canvas_mouseUp",
            "_mfb_macapp_term_scrollWheel",
            "_mfb_macapp_term_mouseDown",
            "_mfb_macapp_canvas_blit",
            "_mfb_macapp_key_inject",
            "_mfb_rt_canvas_graphics_entry",
        ],
    );
}

#[test]
fn gtk_aarch64_callbacks_preserve_callee_saved() {
    let audited = audit_app("linux-aarch64");
    assert_audited(
        &audited,
        "linux-aarch64",
        &[
            "_mfb_gtkapp_mouse_pressed",
            "_mfb_gtkapp_mouse_released",
            "_mfb_gtkapp_mouse_motion",
            "_mfb_gtkapp_mouse_scroll",
            "_mfb_gtkapp_key_pressed",
            "_mfb_gtkapp_term_draw",
            "_mfb_gtkapp_canvas_draw",
            "_mfb_rt_canvas_graphics_entry",
        ],
    );
}

#[test]
fn gtk_x86_64_callbacks_preserve_callee_saved() {
    let audited = audit_app("linux-x86_64");
    assert_audited(
        &audited,
        "linux-x86_64",
        &[
            "_mfb_gtkapp_mouse_pressed",
            "_mfb_gtkapp_mouse_scroll",
            "_mfb_gtkapp_key_pressed",
            "_mfb_rt_canvas_graphics_entry",
        ],
    );
}

#[test]
fn win32_window_procedure_preserves_callee_saved() {
    let audited = audit_app("windows-x86_64");
    assert_audited(
        &audited,
        "windows-x86_64",
        &["_mfb_winapp_wndproc", "_mfb_rt_canvas_graphics_entry"],
    );
}

#[test]
fn thread_start_routine_preserves_callee_saved() {
    for target in ALL_TARGETS {
        let project = fixture_project("thread");
        let ncode = build_ncode(&project, target, "thread_codegen_cover_rt");
        let _ = std::fs::remove_dir_all(&project);
        let audited = audit(&ncode, target, "thread");
        assert_audited(&audited, target, &["_mfb_rt_thread_trampoline"]);
    }
}

#[test]
fn tls_verify_callback_preserves_callee_saved() {
    for target in ["linux-aarch64", "linux-x86_64", "linux-riscv64"] {
        let project = fixture_project("tls");
        let ncode = build_ncode(&project, target, "tls_codegen_cover_rt");
        let _ = std::fs::remove_dir_all(&project);
        let audited = audit(&ncode, target, "tls");
        assert_audited(&audited, target, &["_mfb_tls_verify_cb"]);
    }
}
