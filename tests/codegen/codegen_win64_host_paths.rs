//! plan-157: the `os::` host-path members that resolve through Windows'
//! `SHGetKnownFolderPath` (`appDataPath`, `appCachePath`, `userHomePath`,
//! `userDocumentsPath`) must lower on `windows-x86_64` without addressing outside
//! the acquisition's temporary frame, and must hand the system's buffer back with
//! `CoTaskMemFree` on the success AND the failure path.
//!
//! The frame hazard is the one `codegen_win64_app_resource_path.rs` pins for
//! `GetModuleFileNameW`: `emit_os_wide_string` brackets its body with
//! `subtract_stack(0x60)` … `add_stack(0x60)`, x86-64 spills are `[rsp + k]`, and
//! accesses inside such a window are left unshifted, so a spill placed inside it
//! would be read `0x60` bytes from where it was written. These members keep their
//! `relative` argument live across the query (it is joined on afterwards) — the
//! shape that could trip it.
//!
//! `-ncode` is the post-register-allocation plan, emitted identically on every
//! host; the runtime behaviour is proved on box 2230 (plan-157-B B4).

#[path = "../common/mod.rs"]
mod common;
use common::{build_ncode, temp_project};

use serde_json::Value;

/// The members served by the known-folder query, as `os::<name>`.
const MEMBERS: &[&str] = &[
    "appDataPath",
    "appCachePath",
    "userHomePath",
    "userDocumentsPath",
];

fn source(member: &str) -> String {
    format!(
        "IMPORT os\nIMPORT io\n\nFUNC main AS Integer\n  io::print(os::{member}(\"a/b.txt\"))\n  RETURN 0\nEND FUNC\n"
    )
}

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

fn is_call_to(inst: &Value, target: &str) -> bool {
    inst["op"].as_str() == Some("bl") && inst["target"].as_str() == Some(target)
}

#[test]
fn known_folder_members_stay_inside_the_acquisition_frame_and_free_the_path() {
    for member in MEMBERS {
        let tag = format!("plan156_win_{member}");
        let project = temp_project(&tag, &source(member));
        let ncode = build_ncode(&project, "windows-x86_64", &tag);
        let _ = std::fs::remove_dir_all(&project);

        let body = instructions(function(&ncode, &format!("runtime.os.{member}")));
        let call = body
            .iter()
            .position(|i| is_call_to(i, "SHGetKnownFolderPath"))
            .unwrap_or_else(|| panic!("os::{member} must query SHGetKnownFolderPath"));
        let open = body[..call]
            .iter()
            .rposition(|i| i["op"].as_str() == Some("sub_sp"))
            .expect("the query carves a temporary frame");
        let size = imm(&body[open], "imm").expect("sub_sp imm");
        let close = open
            + 1
            + body[open + 1..]
                .iter()
                .position(|i| i["op"].as_str() == Some("add_sp") && imm(i, "imm") == Some(size))
                .expect("the query tears its temporary frame down");
        let window = &body[open + 1..close];

        let offenders: Vec<String> = window
            .iter()
            .filter(|i| i["base"].as_str() == Some("rsp"))
            .filter(|i| imm(i, "offset").is_some_and(|offset| offset >= size))
            .map(|i| format!("{i}"))
            .collect();
        assert!(
            offenders.is_empty(),
            "os::{member}: while `sp` is moved down {size} bytes these accesses address \
             past the temporary frame:\n{}",
            offenders.join("\n")
        );
        // Both exits free the system's buffer: the success path after the UTF-8
        // marshal, and the failed-HRESULT path.
        let frees = window
            .iter()
            .filter(|i| is_call_to(i, "CoTaskMemFree"))
            .count();
        assert_eq!(
            frees, 2,
            "os::{member} must CoTaskMemFree the known-folder path on both its success \
             and its failure path"
        );
        assert!(
            window.iter().any(|i| is_call_to(i, "WideCharToMultiByte")),
            "os::{member} must marshal the UTF-16 path to UTF-8 inside the frame"
        );
    }
}
