//! bug-454: `os::resourcePath` must lower on `windows-x86_64`, with the right
//! separator arithmetic and without addressing outside the acquisition's frame.
//!
//! Three independent things are checked here, because a green cross-build proves
//! only that codegen emitted *something*:
//!
//! 1. **The gap itself.** `os.resourcePath` was the one call `macos-aarch64` and
//!    `linux-*` both advertise that `win_x86_64`'s `RUNTIME_CALLS` did not, so
//!    `validate_capabilities` rejected every Windows build of a project using it
//!    with `native backend does not support runtime call 'os.resourcePath'`.
//!
//! 2. **The frame hazard the Windows acquisition brings.** It is
//!    `CodegenPlatform::emit_os_wide_string`, which brackets its body with
//!    `subtract_stack(0x60)` … `add_stack(0x60)`. Every x86-64 spill slot is
//!    addressed `[rsp + offset]` (`X86_64RegisterModel::emit_spill`), and
//!    `adjust_stack_instruction_offsets` deliberately leaves accesses inside such
//!    a window unshifted, so a spill written before the `sub_sp` and reloaded
//!    inside it would be read `0x60` bytes away from where it was stored — a
//!    silent miscompile that no golden and no smoke test would localize.
//!    `lower_resource_path` keeps its `String` argument live across the
//!    acquisition (it is copied into the result afterwards), which is the shape
//!    that could trip it.
//!
//! 3. **The separator arithmetic**, which is what the reported symptom actually
//!    came from once the gate was opened.
//!
//! None of these needs a Windows host — `-ncode` is the post-register-allocation
//! code plan, emitted identically on every build host. What they cannot see is
//! the linker/PE side; that is `scripts/test-winapp.sh` on box 2230.

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, temp_project};

use serde_json::Value;
use std::process::Command;

/// The Failing Reproduction from `bugs/bug-454-win64-os-resourcepath-unsupported.md`.
const SOURCE: &str = "\
IMPORT os\n\
IMPORT io\n\
\n\
FUNC main AS Integer\n\
  io::print(os::resourcePath(\"data.txt\"))\n\
  RETURN 0\n\
END FUNC\n";

fn function<'a>(ncode: &'a Value, name: &str) -> &'a Value {
    ncode["functions"]
        .as_array()
        .expect("ncode has a functions array")
        .iter()
        .find(|f| f["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("ncode has no function {name}"))
}

fn instructions(function: &Value) -> &[Value] {
    function["instructions"]
        .as_array()
        .expect("function has instructions")
}

fn imm(inst: &Value, key: &str) -> Option<usize> {
    inst[key].as_str().and_then(|s| s.parse::<usize>().ok())
}

/// The regression itself: the cross-build the capabilities gate used to reject.
#[test]
fn a_resource_path_project_cross_builds_for_windows() {
    let project = temp_project("bug454_win_resource_path", SOURCE);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-q")
        .arg("-target")
        .arg("windows-x86_64")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = project.join("build").join("bug454_win_resource_path.exe");
    let built = exe.is_file();
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        output.status.success(),
        "os::resourcePath must lower on windows-x86_64 (bug-454):\n{log}"
    );
    assert!(
        built,
        "the windows-x86_64 build must produce an .exe:\n{log}"
    );
}

/// The frame hazard, asserted on the emitted plan: every `rsp`-relative access
/// inside the acquisition's `sub_sp N` … `add_sp N` window must address *within*
/// that window (`k < N`). A spill slot lives above the whole function frame, so
/// a spill or reload placed inside would show up as `k >= N`.
#[test]
fn nothing_addresses_outside_the_windows_acquisition_frame() {
    let project = temp_project("bug454_win_spill_span", SOURCE);
    let ncode = build_ncode(&project, "windows-x86_64", "bug454_win_spill_span");
    let _ = std::fs::remove_dir_all(&project);

    let function = function(&ncode, "runtime.os.resourcePath");
    let body = instructions(function);

    // Anchor on the Win32 call itself rather than on a `sub_sp` count: the Win64
    // PROLOGUE is itself two `sub_sp`s around a stack probe for a frame this
    // large (`sub_sp 4096` / `str xzr,[rsp+0]` / `sub_sp 200`), torn down by a
    // single `add_sp`, so plain depth counting misreads the whole body as nested.
    let call = body
        .iter()
        .position(|i| {
            i["op"].as_str() == Some("bl") && i["target"].as_str() == Some("GetModuleFileNameW")
        })
        .expect("windows-x86_64 os::resourcePath must acquire via GetModuleFileNameW");
    let open = body[..call]
        .iter()
        .rposition(|i| i["op"].as_str() == Some("sub_sp"))
        .expect("the acquisition carves a temporary frame");
    let size = imm(&body[open], "imm").expect("sub_sp imm");
    let close = open
        + 1
        + body[open + 1..]
            .iter()
            .position(|i| i["op"].as_str() == Some("add_sp") && imm(i, "imm") == Some(size))
            .expect("the acquisition tears its temporary frame down");
    assert!(
        close > call,
        "the Win32 call sits inside the temporary frame"
    );

    let offenders: Vec<String> = body[open + 1..close]
        .iter()
        .filter(|i| i["base"].as_str() == Some("rsp"))
        .filter(|i| imm(i, "offset").is_some_and(|offset| offset >= size))
        .map(|i| format!("{i}"))
        .collect();
    assert!(
        offenders.is_empty(),
        "while `sp` is moved down {size} bytes by the executable-path acquisition, \
         these accesses address past the temporary frame — a spill written before \
         the `sub_sp` would be read {size} bytes away from where it was stored:\n{}",
        offenders.join("\n")
    );
    // Not vacuous: the window really does carry rsp-relative traffic of its own.
    assert!(
        body[open + 1..close]
            .iter()
            .any(|i| i["base"].as_str() == Some("rsp")),
        "the acquisition window must contain rsp-relative traffic"
    );
}

/// The separator arithmetic. The backward scan runs over the OS-produced
/// executable path, which `GetModuleFileNameW` returns backslash-delimited
/// (`C:\dir\app.exe`), so on Windows it must look for `\` (92). Scanning for `/`
/// (47) there finds nothing, runs the cursor to 0 and raises `ErrUnsupported` —
/// the failure mode a green cross-build alone would hide. The POSIX lowering
/// must be untouched by that.
#[test]
fn the_windows_lowering_scans_for_a_backslash_and_posix_does_not() {
    let project = temp_project("bug454_win_separator", SOURCE);
    let win = build_ncode(&project, "windows-x86_64", "bug454_win_separator");
    let mac = build_ncode(&project, "macos-aarch64", "bug454_win_separator");
    let linux = build_ncode(&project, "linux-x86_64", "bug454_win_separator");
    let _ = std::fs::remove_dir_all(&project);

    let compares = |ncode: &Value, rhs: &str| -> usize {
        instructions(function(ncode, "runtime.os.resourcePath"))
            .iter()
            .filter(|i| i["op"].as_str() == Some("cmp_imm") && i["rhs"].as_str() == Some(rhs))
            .count()
    };
    // Two sites on Windows: the `.`/`..` component validation of the caller's
    // relative argument (where `\` is a traversal separator Win32 honours) and
    // the backward scan of the executable path (where it is the ONLY separator).
    assert_eq!(
        compares(&win, "92"),
        2,
        "windows-x86_64 os::resourcePath compares against `\\` (92) at both the \
         component-validation and backward-scan sites"
    );
    for (name, posix) in [("macos-aarch64", &mac), ("linux-x86_64", &linux)] {
        assert_eq!(
            compares(posix, "92"),
            0,
            "{name} must be untouched — a POSIX filename may contain `\\`"
        );
        assert!(compares(posix, "47") > 0, "{name} still scans for `/` (47)");
    }
}
