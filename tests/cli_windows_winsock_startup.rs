//! Windows Winsock initialization for the `tcp`/`udp` packages (bug-460).
//!
//! Winsock refuses every call until `WSAStartup` has run in the process. The
//! compiler emits it once, in the `_start` entry, gated on whether the program
//! reaches a socket helper at all — a socket-free program must stay
//! byte-identical, so the gate cannot simply always fire.
//!
//! That gate matched the runtime-symbol prefixes `_mfb_rt_net_` and
//! `_mfb_rt_tls_`. plan-110-B/C moved the transports into `tcp` and `udp`, which
//! carry their own runtime families (`_mfb_rt_tcp_*`, `_mfb_rt_udp_*`), so a
//! program using only those got NO `WSAStartup` and every socket call failed with
//! `WSANOTINITIALISED` — surfacing as `7-707-0001`, "Network host, address, or
//! port is invalid", which names neither Winsock nor initialization.
//!
//! It hid behind `net::lookup`: a program that resolved a host first pulled in a
//! `_mfb_rt_net_` symbol, the gate fired, and every later `tcp::` call worked.
//! Reproduced on box 2230 (Windows 11, 10.0.26100.9168): the same
//! `tcp::listen("127.0.0.1", 0)` succeeds after a `net::lookup` and raises
//! without one.
//!
//! The gate is checked here rather than on the box because it is a
//! cross-compiled, host-independent property of the emitted plan, and because
//! the byte-identity half — a socket-free program gaining nothing — cannot be
//! observed by running anything.

mod common;
use common::mfb_exe;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_project(name: &str, source: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mfb_bug460_{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("create temp project");
    std::fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    std::fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

/// Whether the Windows ENTRY of `source` calls `WSAStartup`.
///
/// The whole `.ncode` text is the wrong thing to search: `WSAStartup` appears in
/// the import table of any program that reaches a socket helper, whether or not
/// the entry initializes Winsock. The gate lives in the entry, so the entry's own
/// instruction stream is what has to be read -- located by `entrySymbol` rather
/// than by name, since the entry is `_main` on Windows and `_start` elsewhere.
fn windows_entry_starts_winsock(name: &str, source: &str) -> bool {
    let project = temp_project(name, source);
    let output = Command::new(mfb_exe())
        .arg("build")
        .args(["-target", "windows-x86_64", "-ncode"])
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "windows -ncode build failed for {name}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    entry_body(&project, name).contains("WSAStartup")
}

/// The entry function's own object in the `.ncode` dump, extracted by brace
/// matching.
///
/// Slicing "from the entry's symbol to the next `"symbol":`" does not work: a
/// relocation inside the instruction stream carries a `symbol` field too, so the
/// slice ends a few lines in and reports every entry as empty. The object is
/// found by walking back to its opening brace and forward to the matching close,
/// skipping over string literals so a brace inside one cannot unbalance it.
fn entry_body(project: &Path, name: &str) -> String {
    let dump =
        std::fs::read_to_string(project.join(format!("{name}.ncode"))).expect("read ncode dump");
    let symbol = dump
        .split("\"entrySymbol\": \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("ncode dump names an entrySymbol")
        .to_string();
    let marker = format!("\"symbol\": \"{symbol}\"");
    let at = dump.find(&marker).expect("entry function in ncode dump");
    let bytes = dump.as_bytes();
    let open = dump[..at].rfind('{').expect("entry object opening brace");

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in bytes[open..].iter().enumerate() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return dump[open..open + offset + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("entry function object is unbalanced in the ncode dump");
}

const TCP_ONLY: &str = "IMPORT tcp\n\nFUNC main AS Integer\n  \
                        RES s = tcp::listen(\"127.0.0.1\", 0)\n  \
                        tcp::close(s)\n  RETURN 0\nEND FUNC\n";

const UDP_ONLY: &str = "IMPORT udp\n\nFUNC main AS Integer\n  \
                        RES s = udp::bind(\"127.0.0.1\", 0)\n  \
                        udp::close(s)\n  RETURN 0\nEND FUNC\n";

const TLS_ONLY: &str = "IMPORT tls\n\nFUNC main AS Integer\n  \
                        RES s = tls::connect(\"example.com\", 443)\n  \
                        tls::close(s)\n  RETURN 0\nEND FUNC\n";

const NO_SOCKETS: &str = "IMPORT io\n\nFUNC main AS Integer\n  \
                          io::print(\"hi\")\n  RETURN 0\nEND FUNC\n";

#[test]
fn a_tcp_only_windows_program_initializes_winsock() {
    let starts = windows_entry_starts_winsock("tcp_only", TCP_ONLY);
    assert!(
        starts,
        "a tcp-only program must initialize Winsock; without it every socket call \
         fails with WSANOTINITIALISED (bug-460)"
    );
}

#[test]
fn a_udp_only_windows_program_initializes_winsock() {
    let starts = windows_entry_starts_winsock("udp_only", UDP_ONLY);
    assert!(
        starts,
        "a udp-only program must initialize Winsock (bug-460)"
    );
}

#[test]
fn a_tls_only_windows_program_initializes_winsock() {
    // The Schannel client opens its own raw Winsock socket, so this arm was
    // already covered — it guards against a fix that swaps one prefix for another.
    let starts = windows_entry_starts_winsock("tls_only", TLS_ONLY);
    assert!(starts, "a tls-only program must initialize Winsock");
}

#[test]
fn a_socket_free_windows_program_gains_nothing() {
    // The other half of the gate: it must stay a gate. A program that touches no
    // socket helper must not acquire the call, or every Windows program's entry
    // would change and the byte-identity goldens with it.
    let starts = windows_entry_starts_winsock("no_sockets", NO_SOCKETS);
    assert!(!starts, "a socket-free program must not initialize Winsock");
}
