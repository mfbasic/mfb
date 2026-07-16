//! Regression tests for the program entry's `argc`/`argv` handling.
//!
//! bug-240: every arg-accepting program (`FUNC main(args AS List OF String)`)
//! SIGSEGV'd on linux-x86_64 — glibc and musl, console and `-app` — while
//! aarch64/riscv64/macOS were fine.
//!
//! The entry materializes argc/argv into `ARG[0]`/`ARG[1]`, then zeroes the
//! arena state with a loop whose end pointer is `SCRATCH[1]`. On AArch64
//! `SCRATCH[1]` is `x10`, distinct from `ARG[1]` = `x1`, so the args survived.
//! On x86-64 BOTH neutral tokens realize to **rsi** (`map_scratch_register(10)`
//! → `(10-9) % 11` = 1 → rsi; `CALL_ARGS[1]` → rsi), so the loop destroyed argv
//! two instructions after it was loaded; the entry then walked the arena address
//! as a `char**`. Same token-aliasing family as bug-85.
//!
//! The fix parks argc/argv into the callee-saved `SCRATCH[17]`/`SCRATCH[18]`
//! (x27/x28 → r12/r13) immediately, before anything can clobber them.
//!
//! Why this needed new tests: the only two arg-accepting-`main` fixtures
//! (`tests/rt-error/project/project-entry-{func,sub}-args-*`) run `-ast -ir`
//! only and `FAIL` on their first statement, so nothing exercised an
//! arg-accepting entry's native codegen — moving two instructions in every such
//! entry churned zero goldens.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Echoes its argument vector one line at a time.
const ARGS_SOURCE: &str = "IMPORT io\n\n\
     FUNC main(args AS List OF String) AS Integer\n\
    \x20 io::print(\"argc=\" & toString(len(args)))\n\
    \x20 FOR EACH a IN args\n\
    \x20   io::print(\"arg: \" & a)\n\
    \x20 NEXT\n\
    \x20 RETURN 0\n\
     END FUNC\n";

fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

/// Build for `target` with `-ncode` and return the parsed code plan.
fn build_ncode(name: &str, source: &str, target: &str, app: bool) -> serde_json::Value {
    let project = temp_project(name, source);
    let mut command = Command::new(env!("CARGO_BIN_EXE_mfb"));
    command.arg("build").args(["-target", target]).arg("-ncode");
    if app {
        command.arg("-app");
    }
    let output = command.arg(&project).output().expect("run mfb build -ncode");
    assert!(
        output.status.success(),
        "build failed for {target}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let plan = fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode");
    let value = serde_json::from_str(&plan).expect("parse ncode json");
    let _ = fs::remove_dir_all(&project);
    value
}

fn function<'a>(plan: &'a serde_json::Value, symbol: &str) -> &'a Vec<serde_json::Value> {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"] == symbol)
        .unwrap_or_else(|| panic!("no {symbol} in code plan"))["instructions"]
        .as_array()
        .expect("instructions array")
}

fn field<'a>(instruction: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    instruction.get(key).and_then(|v| v.as_str())
}

/// The register an instruction writes, if any.
fn destination(instruction: &serde_json::Value) -> Option<&str> {
    field(instruction, "dst")
}

/// argv must still be intact when it is parked into a callee-saved register:
/// nothing may redefine argv's register between the point it becomes live and
/// the park. This is the bug stated as an invariant, and it holds on any ISA —
/// including one where the arg and scratch tokens share a physical register.
///
/// argv becomes live in one of two ways. A raw ELF entry derives it from the
/// initial stack (`add argv, sp, 8`), so it is live just after that instruction.
/// An entry that is CALLED (app mode) receives it as an incoming register
/// argument, so it is live from instruction 0 with no defining instruction —
/// which is why the scan cannot simply look for the first write to the register.
fn assert_argv_survives_to_park(
    instructions: &[serde_json::Value],
    argv_reg: &str,
    stack_pointer: &str,
    what: &str,
) {
    // The raw-ELF materialization, if this entry has one.
    let live_from = instructions
        .iter()
        .position(|i| {
            i["op"] == "add_imm"
                && destination(i) == Some(argv_reg)
                && field(i, "src") == Some(stack_pointer)
                && field(i, "imm") == Some("8")
        })
        .map_or(0, |index| index + 1);
    let park = instructions
        .iter()
        .enumerate()
        .skip(live_from)
        .find(|(_, i)| i["op"] == "mov" && field(i, "src") == Some(argv_reg))
        .map(|(index, _)| index)
        .unwrap_or_else(|| panic!("{what}: argv ({argv_reg}) is never parked into a register"));

    for (index, instruction) in instructions.iter().enumerate().take(park).skip(live_from) {
        assert_ne!(
            destination(instruction),
            Some(argv_reg),
            "{what}: argv ({argv_reg}) is clobbered at instruction {index} \
             ({instruction}) before it is parked at {park} — this is bug-240, \
             where the arena-state zero loop's end pointer aliased argv on x86-64"
        );
    }
}

/// x86-64 is where the aliasing bites: SysV `arg1` and `SCRATCH[1]` are both rsi.
#[test]
fn linux_x86_64_console_entry_parks_argv_before_clobbering_it() {
    let plan = build_ncode("entry_args_x86", ARGS_SOURCE, "linux-x86_64", false);
    assert_argv_survives_to_park(
        function(&plan, "_main"),
        "rsi",
        "rsp",
        "linux-x86_64 console entry",
    );
}

/// App mode runs the same entry body on the worker thread under its own symbol.
#[test]
fn linux_x86_64_app_entry_parks_argv_before_clobbering_it() {
    let plan = build_ncode("entry_args_x86_app", ARGS_SOURCE, "linux-x86_64", true);
    assert_argv_survives_to_park(
        function(&plan, "_mfb_macapp_program"),
        "rsi",
        "rsp",
        "linux-x86_64 app entry",
    );
}

/// The ISA that always worked: the invariant must keep holding there too, so a
/// future re-ordering cannot regress it.
#[test]
fn linux_aarch64_console_entry_parks_argv_before_clobbering_it() {
    let plan = build_ncode("entry_args_arm", ARGS_SOURCE, "linux-aarch64", false);
    assert_argv_survives_to_park(
        function(&plan, "_main"),
        "x1",
        "sp",
        "linux-aarch64 console entry",
    );
}

/// An app-mode entry is CALLED by the worker, so it must take argc/argv from
/// registers rather than reading the raw-ELF `[sp]`/`sp+8` layout, which on a
/// worker stack is unrelated data (bug-240).
#[test]
fn linux_app_entry_reads_args_from_registers_not_the_stack() {
    let plan = build_ncode("entry_args_regs", ARGS_SOURCE, "linux-x86_64", true);
    let instructions = function(&plan, "_mfb_macapp_program");
    // The raw-ELF materialization is `ldr argc, [sp,0]` + `add argv, sp, 8`
    // before the frame is carved. Neither may appear in a called entry.
    let frame = instructions
        .iter()
        .position(|i| i["op"] == "sub_sp")
        .expect("entry carves a frame");
    for instruction in instructions.iter().take(frame) {
        assert!(
            !(instruction["op"] == "ldr_u64"
                && field(instruction, "base") == Some("rsp")
                && field(instruction, "offset") == Some("0")),
            "app-mode entry must not read argc off the worker stack: {instruction}"
        );
        assert!(
            !(instruction["op"] == "add_imm"
                && field(instruction, "src") == Some("rsp")
                && field(instruction, "imm") == Some("8")),
            "app-mode entry must not derive argv from the worker stack: {instruction}"
        );
    }
}

/// End-to-end on the host: an arg-accepting program actually receives its argv.
/// Host-only — the cross-target cases above cover the ISA where the bug lived.
#[test]
fn host_arg_accepting_program_receives_its_arguments() {
    let project = temp_project("entry_args_run", ARGS_SOURCE);
    let build = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        build.status.success(),
        "host build failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let exe = String::from_utf8_lossy(&build.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .map(PathBuf::from)
        .expect("build printed an executable path");

    let run = Command::new(&exe)
        .args(["alpha", "beta"])
        .output()
        .expect("run the built program");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "arg-accepting program crashed: {:?}\n{stdout}",
        run.status
    );
    // argv[0] is the program itself, so the vector is [exe, alpha, beta].
    assert!(
        stdout.contains("argc=3"),
        "expected argc=3, got:\n{stdout}"
    );
    assert!(stdout.contains("arg: alpha"), "missing alpha:\n{stdout}");
    assert!(stdout.contains("arg: beta"), "missing beta:\n{stdout}");

    let _ = fs::remove_dir_all(&project);
}
