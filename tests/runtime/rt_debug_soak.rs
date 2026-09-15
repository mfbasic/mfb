//! plan-133-A: app-sized soak tests on the `--debug` report's `live_bytes`.
//!
//! Each case builds one workload at N and 2N iterations and compares the main arena's
//! `live_bytes` at exit: a loop that frees what it allocates reports the same number at both
//! counts. `live_bytes` is page-size independent (RSS is 4× `mapped_bytes` on Apple Silicon),
//! so the same bound holds on every host.
//!
//! The flat cases guard what already works (plan-134's recursive-value frees at an app-sized
//! input). Every stage of the browser example's page load that leaks has a case marked
//! `#[ignore = "bug-NNN: …"]`; the fix for that bug removes the marker. Run them with
//! `cargo test --release --test rt_debug_soak -- --include-ignored`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug_project, counter, run_ok};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

/// Allowed growth between N and 2N. Far below every measured leak (the smallest,
/// `http::read`, is 62 KB per call × 20 extra calls) and above allocator noise.
const FLAT_BOUND: u64 = 1024 * 1024;

/// Allowed growth for a loop whose leak is one small block per iteration (bug-623's 96 B
/// socket record, bug-625's 48 B list). `live_bytes` counts exactly, so a loop that frees
/// what it allocates reports the same number at N and 2N; this is only headroom for a
/// one-off block, far below N x 48 B for every case that uses it.
const BLOCK_BOUND: u64 = 4096;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Copy `from` into `to` recursively, skipping build output and installed packages.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create package copy");
    for entry in fs::read_dir(from).expect("read package dir") {
        let entry = entry.expect("dir entry");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "build" || name == "packages" || name.ends_with(".mfp") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_tree(&path, &to.join(&*name));
        } else {
            fs::copy(&path, to.join(&*name)).expect("copy package file");
        }
    }
}

/// Install `examples/browser/<name>` under `project/packages/<name>` as a source package,
/// with its own `dom` dependency (display) rewritten from the `.mfp` form to the source form.
fn install_browser_package(project: &Path, name: &str) {
    let dest = project.join("packages").join(name);
    copy_tree(&repo_root().join("examples/browser").join(name), &dest);
    if name != "dom" {
        copy_tree(
            &repo_root().join("examples/browser/dom"),
            &dest.join("packages/dom"),
        );
        let manifest = dest.join("project.json");
        let text = fs::read_to_string(&manifest).expect("read package manifest");
        fs::write(
            &manifest,
            text.replace("\"file:packages/dom.mfp\"", "\"file:packages/dom\""),
        )
        .expect("write package manifest");
    }
}

/// A scratch executable project for `source` that depends on the browser packages in
/// `packages` as source packages.
fn browser_project(name: &str, source: &str, packages: &[&str]) -> PathBuf {
    let project = common::temp_project(name, source);
    let mut deps = Vec::new();
    for package in packages {
        install_browser_package(&project, package);
        deps.push(format!(
            "{{\"name\":\"{package}\",\"version\":\"=0.1.0\",\"source\":\"file:packages/{package}\"}}"
        ));
    }
    fs::write(
        project.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"packages\":[{}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n",
            deps.join(",")
        ),
    )
    .expect("write project.json");
    project
}

/// Build and run `project`, returning the main arena's `live_bytes` at exit.
fn main_live_bytes(name: &str, project: &Path) -> u64 {
    let exe = build_debug_project(name, project);
    let (_, stderr) = run_ok(name, &exe);
    let lines = arena_lines(name, &stderr);
    counter(name, &lines, 0, "live_bytes")
}

/// `live_bytes` at `small` and `large` iterations of `make(n)`, asserting the growth is
/// under [`FLAT_BOUND`]; `owner` names what the failure means.
fn assert_live_bytes_flat(
    case: &str,
    small: u64,
    large: u64,
    owner: &str,
    make: impl Fn(u64) -> PathBuf,
) {
    assert_live_bytes_within(case, small, large, FLAT_BOUND, owner, make);
}

/// [`assert_live_bytes_flat`] with an explicit growth `bound`.
fn assert_live_bytes_within(
    case: &str,
    small: u64,
    large: u64,
    bound: u64,
    owner: &str,
    make: impl Fn(u64) -> PathBuf,
) {
    let at_small = main_live_bytes(&format!("{case}_{small}"), &make(small));
    let at_large = main_live_bytes(&format!("{case}_{large}"), &make(large));
    let grew = at_large.saturating_sub(at_small);
    assert!(
        grew < bound,
        "{case}: main-arena live_bytes grew {grew} B between {small} and {large} iterations \
         ({at_small} -> {at_large}); {owner}"
    );
}

/// A generated ~1 MiB comma-separated text, written into the project as `input.txt`.
fn write_split_input(project: &Path) -> PathBuf {
    let mut text = String::with_capacity(1 << 20);
    let mut i = 0u64;
    while text.len() < 1 << 20 {
        text.push_str(&format!("field{i},"));
        i += 1;
    }
    let path = project.join("input.txt");
    fs::write(&path, text).expect("write split input");
    path
}

/// A generated ~1.1 MiB JSON array of 9,000 objects (the plan-133-A probe's shape).
fn write_json_input(project: &Path) -> PathBuf {
    let mut items = Vec::with_capacity(9000);
    for i in 0..9000u64 {
        let s = "\\u00e9".repeat((i % 5) as usize);
        items.push(format!(
            "{{\"id\": {i}, \"name\": \"item-{i}\", \"tags\": [\"a\", \"bb\", \"ccc\"], \"nested\": {{\"x\": {}.{}, \"ok\": {}, \"s\": \"{s}\"}}}}",
            i * 3 / 2,
            if i % 2 == 1 { 5 } else { 0 },
            i % 2 == 0
        ));
    }
    let path = project.join("input.json");
    fs::write(&path, format!("[{}]", items.join(", "))).expect("write json input");
    path
}

/// The flat control: splitting a 1 MiB string N times frees every list it builds.
#[test]
fn a_flat_split_loop_keeps_live_bytes_constant() {
    assert_live_bytes_flat(
        "soak_split",
        20,
        40,
        "strings::split must free its list",
        |n| {
            let project = common::temp_project("soak_split", "");
            let input = write_split_input(&project);
            fs::write(
            project.join("src/main.mfb"),
            format!(
                "IMPORT fs\nIMPORT io\nIMPORT strings\n\nSUB main()\n  LET text AS String = fs::readText(\"{}\")\n  MUT parts AS Integer = 0\n  FOR i = 1 TO {n}\n    LET l AS List OF String = strings::split(text, \",\")\n    parts = len(l)\n  NEXT\n  io::print(toString(parts))\nEND SUB\n",
                common::mfb_path_literal(&input)
            ),
        )
        .expect("write program");
            project
        },
    );
}

/// plan-134's fix at an app-sized input: parsing a 1.1 MiB JSON array N times frees each
/// recursive tree (+1,059,198,720 B between 20 and 40 parses before plan-134).
#[test]
fn a_json_parse_loop_keeps_live_bytes_constant() {
    assert_live_bytes_flat(
        "soak_json",
        20,
        40,
        "json::parse must free each tree (plan-134)",
        |n| {
            let project = common::temp_project("soak_json", "");
            let input = write_json_input(&project);
            fs::write(
            project.join("src/main.mfb"),
            format!(
                "IMPORT fs\nIMPORT io\nIMPORT json\n\nSUB main()\n  LET text AS String = fs::readText(\"{}\")\n  FOR i = 1 TO {n}\n    LET doc AS json::Json = json::parse(text)\n  NEXT\n  io::print(\"done\")\nEND SUB\n",
                common::mfb_path_literal(&input)
            ),
        )
        .expect("write program");
            project
        },
    );
}

/// The browser's parse stage on the saved 603,614-byte Wikipedia `BASIC` page
/// (8,318,848 B left per parse, plan-133-A § 2). N=1 vs 2: one parse takes ~40 s.
#[test]
fn a_dom_parse_loop_keeps_live_bytes_constant() {
    let html = repo_root().join("tests/runtime/data/rt_debug_soak_basic.html");
    assert_live_bytes_flat(
        "soak_dom_parse",
        1,
        2,
        "dom::parse leaks (bug-620, bug-621)",
        |n| {
            browser_project(
            "soak_dom_parse",
            &format!(
                "IMPORT dom\nIMPORT fs\nIMPORT io\n\nSUB main()\n  LET html AS String = fs::readText(\"{}\")\n  FOR i = 1 TO {n}\n    LET d AS dom::Node = dom::parse(html)\n  NEXT\n  io::print(\"done\")\nEND SUB\n",
                common::mfb_path_literal(&html)
            ),
            &["dom"],
        )
        },
    );
}

/// The browser's resolve-styles stage on a generated page: 400 elements × 60 class and
/// descendant rules (718,525,504 B per call on the `BASIC` page, plan-133-A § 2).
#[test]
fn a_resolve_styles_loop_keeps_live_bytes_constant() {
    let mut html = String::from("<html><head><style>");
    for r in 0..60 {
        html.push_str(&format!(
            ".c{r} {{ margin: 1px 2px; }} div p.c{r} {{ display: block; }} "
        ));
    }
    html.push_str("</style></head><body>");
    for e in 0..400 {
        html.push_str(&format!(
            "<div class='a c{}'><p class='c{}'>text {e}</p></div>",
            e % 70,
            e % 65
        ));
    }
    html.push_str("</body></html>");
    assert_live_bytes_flat(
        "soak_resolve",
        4,
        8,
        "dom::resolveStyles leaks (bug-620, bug-621)",
        |n| {
            let project = browser_project("soak_resolve", "", &["dom"]);
            let input = project.join("page.html");
            fs::write(&input, &html).expect("write page");
            fs::write(
            project.join("src/main.mfb"),
            format!(
                "IMPORT dom\nIMPORT fs\nIMPORT io\n\nSUB main()\n  LET d AS dom::Node = dom::parse(fs::readText(\"{}\"))\n  FOR i = 1 TO {n}\n    LET r AS dom::Node = dom::resolveStyles(d)\n  NEXT\n  io::print(\"done\")\nEND SUB\n",
                common::mfb_path_literal(&input)
            ),
        )
        .expect("write program");
            project
        },
    );
}

/// The browser's paint stage (layout + canvas) on a small styled page, N=2000 vs 4000
/// (960 B per paint: 720 B layout, 240 B canvas, plan-133-A § 2).
#[test]
fn a_paint_loop_keeps_live_bytes_constant() {
    assert_live_bytes_flat(
        "soak_paint",
        2000,
        4000,
        "display::paint leaks (bug-620, bug-621, bug-625)",
        |n| {
            browser_project(
            "soak_paint",
            &format!(
                "IMPORT display\nIMPORT dom\nIMPORT io\n\nSUB main()\n  LET d AS dom::Node = dom::indexFields(dom::resolveStyles(dom::parse(\"<div><p>hello <b>world</b> and <a href='/x'>a link</a></p><ul><li>one</li><li>two</li></ul></div>\")))\n  MUT rows AS Integer = 0\n  FOR i = 1 TO {n}\n    LET pr AS display::PaintResult = display::paint(d, 120, 8, 16)\n    rows = len(pr.rows)\n  NEXT\n  io::print(toString(rows))\nEND SUB\n"
            ),
            &["dom", "display"],
        )
        },
    );
}

/// The browser's copy-back stage: a worker returning a record that holds a recursive tree,
/// started and waited for N times (5,268,592 B per page load in the browser, plan-133-A § 2).
#[test]
fn a_thread_copy_back_loop_keeps_live_bytes_constant() {
    let source = |n: u64| {
        format!(
            "IMPORT io\nIMPORT thread\n\nTYPE SoakEl\n  tag AS String\n  kids AS List OF SoakNode\nEND TYPE\n\nTYPE SoakText\n  text AS String\nEND TYPE\n\nUNION SoakNode\n  SoakEl\n  SoakText\nEND UNION\n\nTYPE SoakResult\n  ok AS Boolean\n  document AS SoakNode\n  title AS String\nEND TYPE\n\nISOLATED FUNC work(w AS ThreadWorker OF String TO SoakResult, seed AS String) AS SoakResult\n  LET leaf AS SoakNode = SoakText[seed & \"!\"]\n  LET root AS SoakNode = SoakEl[\"div\", [leaf, leaf]]\n  RETURN SoakResult[TRUE, root, seed & \"?\"]\nEND FUNC\n\nSUB main()\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    LET t AS Thread OF String TO SoakResult = thread::start(work, \"abc\")\n    LET r AS SoakResult = thread::waitFor(t)\n    total = total + len(r.title)\n  NEXT\n  io::print(toString(total))\nEND SUB\n"
        )
    };
    // 7,424 B per thread measured, so 400 extra threads grow ~3 MB, past FLAT_BOUND.
    assert_live_bytes_flat(
        "soak_thread",
        400,
        800,
        "thread copy-back leaks (bug-622)",
        |n| common::temp_project("soak_thread", &source(n)),
    );
}

/// Serve `count` HTTP/1.1 responses with a fixed 6,839-byte body on a loopback port.
fn serve_http(count: u64) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        let body = "x".repeat(6839);
        for _ in 0..count {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

/// The browser's fetch stage, over loopback plain HTTP (62,435 B per call, plan-133-A § 2).
#[test]
fn an_http_read_loop_keeps_live_bytes_constant() {
    let run = |n: u64| {
        let port = serve_http(n);
        let project = common::temp_project(
            "soak_http",
            &format!(
                "IMPORT http\nIMPORT io\nIMPORT net\n\nSUB main()\n  MUT bytes AS Integer = 0\n  FOR i = 1 TO {n}\n    LET resp AS http::Response = http::read(net::toUrl(\"http://127.0.0.1:{port}/sheet.css\"))\n    bytes = len(resp.body)\n  NEXT\n  io::print(toString(bytes))\nEND SUB\n"
            ),
        );
        main_live_bytes(&format!("soak_http_{n}"), &project)
    };
    let at_small = run(20);
    let at_large = run(40);
    let grew = at_large.saturating_sub(at_small);
    assert!(
        grew < FLAT_BOUND,
        "soak_http: main-arena live_bytes grew {grew} B between 20 and 40 reads \
         ({at_small} -> {at_large}); http::read leaks (bug-623)"
    );
}

/// Accept `count` connections on a loopback port; write `reply` to each, then close it.
fn serve_tcp(count: u64, reply: &'static [u8]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for _ in 0..count {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.write_all(reply);
        }
    });
    port
}

/// bug-623: `tcp::read` frees its capped read buffer, and `tcp::close` the socket record.
/// Measured before the fix: 65,536 B buffer + 96 B record per iteration.
#[test]
fn a_tcp_read_loop_keeps_live_bytes_constant() {
    static REPLY: [u8; 6839] = [b'x'; 6839];
    assert_live_bytes_within(
        "soak_tcp_read",
        200,
        400,
        BLOCK_BOUND,
        "tcp::read leaks its read buffer or tcp::close its socket record (bug-623)",
        |n| {
            let port = serve_tcp(n, &REPLY);
            common::temp_project(
                "soak_tcp_read",
                &format!(
                    "IMPORT io\nIMPORT tcp\n\nSUB main()\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS tcp::Socket = tcp::connect(\"127.0.0.1\", {port})\n    LET got AS List OF Byte = tcp::read(c, 65536)\n    total = total + len(got)\n    tcp::close(c)\n  NEXT\n  io::print(toString(total))\nEND SUB\n"
                ),
            )
        },
    );
}

/// bug-623: a `tcp::connect` / `tcp::close` pair frees the socket record (96 B each before
/// the fix).
#[test]
fn a_tcp_connect_close_loop_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_tcp_close",
        300,
        600,
        BLOCK_BOUND,
        "tcp::close leaks the socket record (bug-623)",
        |n| {
            let port = serve_tcp(n, b"");
            common::temp_project(
                "soak_tcp_close",
                &format!(
                    "IMPORT io\nIMPORT tcp\n\nSUB main()\n  FOR i = 1 TO {n}\n    RES c AS tcp::Socket = tcp::connect(\"127.0.0.1\", {port})\n    tcp::close(c)\n  NEXT\n  io::print(\"done\")\nEND SUB\n"
                ),
            )
        },
    );
}

/// bug-623's udp audit: a `udp::bind` / `udp::close` pair frees the socket record (96 B each
/// before the fix).
#[test]
fn a_udp_bind_close_loop_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_udp_close",
        300,
        600,
        BLOCK_BOUND,
        "udp::close leaks the socket record (bug-623)",
        |n| {
            common::temp_project(
                "soak_udp_close",
                &format!(
                    "IMPORT io\nIMPORT udp\n\nSUB main()\n  FOR i = 1 TO {n}\n    RES s AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    udp::close(s)\n  NEXT\n  io::print(\"done\")\nEND SUB\n"
                ),
            )
        },
    );
}

/// An OpenSSL (not LibreSSL) `openssl` CLI to serve TLS, as `rt_tls_connect_allow_self_signed`
/// requires of its peer.
fn have_openssl_peer() -> bool {
    std::process::Command::new("openssl")
        .arg("version")
        .output()
        .map(|out| {
            out.status.success() && String::from_utf8_lossy(&out.stdout).starts_with("OpenSSL")
        })
        .unwrap_or(false)
}

/// Serve a fresh self-signed `localhost` identity with `openssl s_server` on a loopback port,
/// and return the child once a handshake completes against it.
fn serve_tls(root: &Path) -> (std::process::Child, u16) {
    use std::process::{Command, Stdio};
    fs::create_dir_all(root).expect("create tls scratch");
    let cert = root.join("cert.pem");
    let key = root.join("key.pem");
    let made = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost"])
        .args(["-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"])
        .args(["-addext", "extendedKeyUsage=serverAuth", "-days", "397"])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .output()
        .expect("run openssl req");
    assert!(
        made.status.success(),
        "openssl req: {}",
        String::from_utf8_lossy(&made.stderr)
    );
    for _ in 0..10 {
        let guard = common::PortGate::acquire();
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind an ephemeral port")
            .local_addr()
            .expect("local addr")
            .port();
        let mut child = Command::new("openssl")
            .args(["s_server", "-quiet", "-accept", &port.to_string()])
            .arg("-cert")
            .arg(&cert)
            .arg("-key")
            .arg(&key)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn openssl s_server");
        for _ in 0..200 {
            if child.try_wait().expect("poll s_server").is_some() {
                break;
            }
            let probe = Command::new("openssl")
                .args(["s_client", "-connect", &format!("127.0.0.1:{port}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if probe.map(|status| status.success()).unwrap_or(false) {
                drop(guard);
                return (child, port);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    panic!("openssl s_server never began accepting");
}

/// bug-623: a `tls::connect` / `tls::close` pair frees the TLS context and socket records
/// (384 B per HTTPS read before the fix, plan-133-A).
#[test]
fn a_tls_connect_close_loop_keeps_live_bytes_constant() {
    if !have_openssl_peer() {
        eprintln!("skipping: no OpenSSL `openssl` CLI to serve TLS");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_soak_tls_{}", common::unique_nonce()));
    let (mut server, port) = serve_tls(&root);
    let run = |n: u64| {
        let project = common::temp_project(
            "soak_tls_close",
            &format!(
                "IMPORT io\nIMPORT tls\n\nSUB main()\n  FOR i = 1 TO {n}\n    RES conn = tls::connect(\"127.0.0.1\", {port}, 5000, \"localhost\", allowSelfSigned := TRUE)\n    tls::close(conn)\n  NEXT\n  io::print(\"done\")\nEND SUB\n"
            ),
        );
        main_live_bytes(&format!("soak_tls_close_{n}"), &project)
    };
    let at_small = run(100);
    let at_large = run(200);
    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_dir_all(&root);
    let grew = at_large.saturating_sub(at_small);
    assert!(
        grew < BLOCK_BOUND,
        "soak_tls_close: main-arena live_bytes grew {grew} B between 100 and 200 connections \
         ({at_small} -> {at_large}); tls::close leaks its connection records (bug-623)"
    );
}

/// A loop of `n` iterations running `body` (which may use `i` and `total`), with `decls`
/// above `main`.
fn astrings_project(case: &str, n: u64, decls: &str, body: &str) -> PathBuf {
    common::temp_project(
        case,
        &format!(
            "IMPORT io\nIMPORT astrings\nIMPORT collections\n\n{decls}\nSUB main()\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n{body}\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND SUB\n"
        ),
    )
}

/// bug-625: `astrings::fromString` frees the empty `spans` list it byte-copies into the
/// record (48 B per value before the fix).
#[test]
fn a_bound_attributed_string_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_as_single",
        1000,
        2000,
        BLOCK_BOUND,
        "astrings::fromString leaks its spans list (bug-625)",
        |n| {
            astrings_project(
                "soak_as_single",
                n,
                "",
                "    LET a AS AttributedString = astrings::fromString(\"ab\" & toString(i))\n    total = total + 1",
            )
        },
    );
}

/// bug-625: the same leak once per list element.
#[test]
fn a_list_of_attributed_strings_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_as_list",
        1000,
        2000,
        BLOCK_BOUND,
        "astrings::fromString leaks its spans list (bug-625)",
        |n| {
            astrings_project(
                "soak_as_list",
                n,
                "",
                "    LET l AS List OF AttributedString = [astrings::fromString(\"a\" & toString(i)), astrings::fromString(\"c\" & toString(i))]\n    total = total + len(l)",
            )
        },
    );
}

/// bug-625: the same leak through a record field built by a helper.
#[test]
fn a_record_of_attributed_strings_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_as_record",
        1000,
        2000,
        BLOCK_BOUND,
        "astrings::fromString leaks its spans list (bug-625)",
        |n| {
            astrings_project(
                "soak_as_record",
                n,
                "TYPE AsResult\n  rows AS List OF AttributedString\n  count AS Integer\nEND TYPE\n\nFUNC build(texts AS List OF String) AS AsResult\n  MUT out AS List OF AttributedString = []\n  FOR EACH t IN texts\n    out = collections::append(out, astrings::fromString(t))\n  NEXT\n  RETURN AsResult[out, len(texts)]\nEND FUNC\n",
                "    LET r AS AsResult = build([\"one\" & \"!\", \"two\" & \"!\", \"three\" & \"!\", \"four\" & \"!\"])\n    total = total + len(r.rows)",
            )
        },
    );
}

/// bug-625: an attributed value (a non-empty `spans` list) is freed completely too.
#[test]
fn an_attributed_string_with_an_attribute_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_as_attr",
        1000,
        2000,
        BLOCK_BOUND,
        "astrings::fromString / addAttribute leak (bug-625)",
        |n| {
            astrings_project(
                "soak_as_attr",
                n,
                "",
                "    LET a AS AttributedString = astrings::fromString(\"hello \" & toString(i))\n    LET styled AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())\n    total = total + 1",
            )
        },
    );
}

/// bug-625 B: a defaulted record frees the default field values it byte-copies inline
/// (48 B per record before the fix — the `fromString` hazard in `lower_default_value`'s
/// record arm, `builder_value_semantics.rs`).
#[test]
fn a_defaulted_record_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_default_record",
        1000,
        2000,
        BLOCK_BOUND,
        "a defaulted record leaks its default field values (bug-625)",
        |n| {
            common::temp_project(
                "soak_default_record",
                &format!(
                    "IMPORT io\n\nTYPE Rec\n  name AS String\n  items AS List OF Integer\nEND TYPE\n\nSUB main()\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    MUT r AS Rec\n    total = total + len(r.items) + len(r.name) + 1\n  NEXT\n  io::print(\"total=\" & toString(total))\nEND SUB\n"
                ),
            )
        },
    );
}

/// A loop of `n` `crypto::generate` calls cycling through `variants`.
fn crypto_generate_project(case: &str, n: u64, variants: &[&str]) -> PathBuf {
    let calls: String = variants
        .iter()
        .map(|variant| {
            format!(
                "    LET kp{variant} AS crypto::KeyPair = crypto::generate(crypto::Certificate.{variant})\n    total = total + len(kp{variant}.publicKey) + len(kp{variant}.privateKey)\n"
            )
        })
        .collect();
    common::temp_project(
        case,
        &format!(
            "IMPORT io\nIMPORT crypto\n\nSUB main()\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n{calls}  NEXT\n  io::print(toString(total))\nEND SUB\n"
        ),
    )
}

/// bug-625 C: the native EC `crypto::generate` paths free the two key byte lists they build
/// and byte-copy into the inlined `KeyPair` (2 blocks per call before the fix: 256 / 336 /
/// 416 B for P-256 / P-384 / P-521 — the `fromString` hazard in `crypto/func_generate.rs`).
#[test]
fn a_native_ec_generate_loop_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_generate_ec",
        50,
        100,
        BLOCK_BOUND,
        "crypto::generate (P256/P384/P521) leaks its key lists (bug-625)",
        |n| crypto_generate_project("soak_generate_ec", n, &["P256", "P384", "P521"]),
    );
}

/// bug-625 D: the software-curve `crypto::generate` paths leave nothing live (one block per
/// call before the fix: 32 B for Ed25519 / X25519, 64 B for X448 / Ed448).
#[test]
fn a_software_curve_generate_loop_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_generate_soft",
        50,
        100,
        BLOCK_BOUND,
        "crypto::generate (Ed25519/X25519/X448/Ed448) leaks a key-sized block (bug-625)",
        |n| {
            crypto_generate_project(
                "soak_generate_soft",
                n,
                &["Ed25519", "X25519", "X448", "Ed448"],
            )
        },
    );
}

/// bug-623 residual: `http::read` over loopback leaves nothing live per call. The FLAT_BOUND
/// case above passes with a small leak left; measured after the read-buffer and record fixes:
/// 160 B per call (the response resource union's 96 B variant record plus 64 B in two blocks).
#[test]
fn an_http_read_loop_leaves_no_block_behind() {
    let run = |n: u64| {
        let port = serve_http(n);
        let project = common::temp_project(
            "soak_http_exact",
            &format!(
                "IMPORT http\nIMPORT io\nIMPORT net\n\nSUB main()\n  MUT bytes AS Integer = 0\n  FOR i = 1 TO {n}\n    LET resp AS http::Response = http::read(net::toUrl(\"http://127.0.0.1:{port}/sheet.css\"))\n    bytes = len(resp.body)\n  NEXT\n  io::print(toString(bytes))\nEND SUB\n"
            ),
        );
        main_live_bytes(&format!("soak_http_exact_{n}"), &project)
    };
    let at_small = run(50);
    let at_large = run(100);
    let grew = at_large.saturating_sub(at_small);
    assert!(
        grew < BLOCK_BOUND,
        "soak_http_exact: main-arena live_bytes grew {grew} B between 50 and 100 reads \
         ({at_small} -> {at_large}); http::read leaves blocks behind (bug-623)"
    );
}

/// bug-623 residual: a resource union bound straight from a producer frees the variant
/// record it owns (96 B per bind after the box fix: only the 16 B box was freed).
#[test]
fn a_resource_union_bound_from_a_producer_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_union_direct",
        300,
        600,
        BLOCK_BOUND,
        "a resource union bound from a producer leaks its variant record (bug-623)",
        |n| {
            common::temp_project(
                "soak_union_direct",
                &format!(
                    "IMPORT io\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nSUB main()\n  FOR i = 1 TO {n}\n    RES c AS Chan = udp::bind(\"127.0.0.1\", 0)\n  NEXT\n  io::print(\"done\")\nEND SUB\n"
                ),
            )
        },
    );
}

/// bug-623 guard: a resource union aliasing a live concrete binding frees nothing the
/// concrete binding still owns — the record is freed once, by `u`'s drop.
#[test]
fn a_resource_union_aliasing_a_binding_keeps_live_bytes_constant() {
    assert_live_bytes_within(
        "soak_union_alias",
        300,
        600,
        BLOCK_BOUND,
        "a resource union alias leaks or double-frees (bug-623)",
        |n| {
            common::temp_project(
                "soak_union_alias",
                &format!(
                    "IMPORT io\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nSUB main()\n  FOR i = 1 TO {n}\n    RES u AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    RES c AS Chan = u\n  NEXT\n  io::print(\"done\")\nEND SUB\n"
                ),
            )
        },
    );
}

/// `arena.0` counters from a `--debug` report: `(alloc_calls, free_calls, live_bytes,
/// double_free_skips)`.
fn main_arena_calls(name: &str, stderr: &str) -> (u64, u64, u64, u64) {
    let lines = arena_lines(name, stderr);
    (
        counter(name, &lines, 0, "alloc_calls"),
        counter(name, &lines, 0, "free_calls"),
        counter(name, &lines, 0, "live_bytes"),
        counter(name, &lines, 0, "double_free_skips"),
    )
}

/// Build `source` (with `{n}` substituted) as a `--debug` project, run it, and return its
/// stdout and main-arena counters.
fn debug_run(case: &str, source: &str, n: u64) -> (String, (u64, u64, u64, u64)) {
    let name = format!("{case}_{n}");
    let project = common::temp_project(case, &source.replace("{n}", &n.to_string()));
    let exe = build_debug_project(&name, &project);
    let (stdout, stderr) = run_ok(&name, &exe);
    let counters = main_arena_calls(&name, &stderr);
    (stdout, counters)
}

/// bug-623 regression (introduced by the record free, fixed by the ownership pass): a
/// function that returns its `RES` parameter hands the caller back the caller's own
/// record, and the caller's second binding must not free it again. Before the ownership
/// pass: `free_calls` 752 against `alloc_calls` 600 and `double_free_skips` 152 at N=300.
#[test]
fn a_resource_passed_through_a_function_is_freed_once() {
    const SOURCE: &str = "IMPORT io\nIMPORT udp\n\nFUNC passthru(RES u AS udp::Socket) AS RES udp::Socket\n  RETURN u\nEND FUNC\n\nSUB main()\n  FOR i = 1 TO {n}\n    RES a AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    RES b AS udp::Socket = passthru(a)\n  NEXT\n  io::print(\"done\")\nEND SUB\n";
    for n in [300u64, 600] {
        let (_, (allocs, frees, _, skips)) = debug_run("res_passthru", SOURCE, n);
        assert!(
            skips == 0 && frees <= allocs,
            "res_passthru N={n}: double_free_skips {skips}, free_calls {frees} > alloc_calls \
             {allocs} — a passed-through record was freed twice (bug-623)"
        );
    }
    let (_, (_, _, at_small, _)) = debug_run("res_passthru_flat", SOURCE, 300);
    let (_, (_, _, at_large, _)) = debug_run("res_passthru_flat", SOURCE, 600);
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "res_passthru: live_bytes grew {at_small} -> {at_large} between 300 and 600 (bug-623)"
    );
}

/// Found by bug-623 (pre-existing; a use-after-free since the record free): a function that
/// returns a resource union wrapping its OWN owned local handed the caller a closed handle
/// — `udp::localAddress` on it raised `7-703-0004` and the program exited 255 — because the
/// callee's scope drop closed (and now freed) the record the union still pointed at.
#[test]
fn a_returned_union_wrapping_an_owned_local_stays_open_in_the_caller() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nFUNC open() AS RES Chan\n  RES u AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  RES c AS Chan = u\n  RETURN c\nEND FUNC\n\nFUNC portOf(RES c AS Chan) AS Integer\n  MUT port AS Integer = -1\n  MATCH c\n    CASE udp::Socket(s)\n      LET a AS net::Address = udp::localAddress(s)\n      port = a.port\n    CASE fs::File(f)\n      port = -2\n  END MATCH\n  RETURN port\nEND FUNC\n\nSUB main()\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c AS Chan = open()\n    IF portOf(c) > 0 THEN\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND SUB\n";
    let (stdout_small, (_, _, at_small, skips_small)) = debug_run("ret_union_owned", SOURCE, 50);
    let (stdout_large, (_, _, at_large, skips_large)) = debug_run("ret_union_owned", SOURCE, 100);
    assert!(
        stdout_small.contains("ok=50") && stdout_large.contains("ok=100"),
        "every returned union must still be an open socket:\n{stdout_small}\n{stdout_large}"
    );
    assert!(
        skips_small == 0 && skips_large == 0,
        "ret_union_owned: double_free_skips {skips_small}/{skips_large}"
    );
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "ret_union_owned: live_bytes grew {at_small} -> {at_large} between 50 and 100"
    );
}

/// Found by bug-623 (pre-existing, same class): a resource union aliasing an outer binding
/// in an inner scope closed the OUTER handle when the inner scope ended — the union
/// registered its own close. `udp::localAddress(u)` afterwards raised `7-703-0004`.
#[test]
fn a_union_alias_in_an_inner_scope_leaves_the_outer_handle_open() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\nIMPORT fs\n\nUNION Chan\n  udp::Socket\n  fs::File\nEND UNION\n\nSUB main()\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES u AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n    MUT flag AS Boolean = 1 > 0\n    IF flag THEN\n      RES c AS Chan = u\n    END IF\n    LET a AS net::Address = udp::localAddress(u)\n    IF a.port > 0 THEN\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND SUB\n";
    let (stdout_small, (_, _, at_small, skips_small)) = debug_run("union_scope_alias", SOURCE, 50);
    let (stdout_large, (_, _, at_large, skips_large)) = debug_run("union_scope_alias", SOURCE, 100);
    assert!(
        stdout_small.contains("ok=50") && stdout_large.contains("ok=100"),
        "the outer handle must stay open after the inner alias's scope ends:\n{stdout_small}\n{stdout_large}"
    );
    assert!(
        skips_small == 0 && skips_large == 0,
        "union_scope_alias: double_free_skips {skips_small}/{skips_large}"
    );
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "union_scope_alias: live_bytes grew {at_small} -> {at_large} between 50 and 100"
    );
}

/// Found while settling the cross-arena question for bug-623: a macOS `tls::Socket` handed to
/// a worker with `thread::transfer` and closed there never frees its connection ctx. The
/// close skips the free when `CTX_OWNER` differs from the closing thread's arena, but a free
/// of another arena's block is sound here — `arena_free` pushes onto the FREEING thread's
/// bins and never asks which arena carved the block, and no arena but the main one is ever
/// destroyed (`.ai/canvas-threading.md` §2; bug-498's hand-over relies on it) — so the skip
/// only leaked the 216 B ctx the connecting thread allocated, once per transferred socket.
#[test]
#[cfg(target_os = "macos")]
fn a_tls_socket_closed_on_another_thread_keeps_live_bytes_constant() {
    if !have_openssl_peer() {
        eprintln!("skipping: no OpenSSL `openssl` CLI to serve TLS");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_soak_tls_xfer_{}", common::unique_nonce()));
    let (mut server, port) = serve_tls(&root);
    let run = |n: u64| {
        let project = common::temp_project(
            "soak_tls_xfer_close",
            &format!(
                "IMPORT io\nIMPORT tls\nIMPORT thread\n\nISOLATED FUNC worker(t AS ThreadWorker OF RES tls::Socket TO Integer, n AS Integer) AS Integer\n  RES s AS tls::Socket = thread::accept(t, 20000)\n  tls::close(s)\n  RETURN 1\nEND FUNC\n\nSUB main()\n  MUT total AS Integer = 0\n  FOR i = 1 TO {n}\n    RES c = tls::connect(\"127.0.0.1\", {port}, 5000, \"localhost\", allowSelfSigned := TRUE)\n    LET a AS Thread OF RES tls::Socket TO Integer = thread::start(worker, 0)\n    thread::transfer(a, c)\n    total = total + thread::waitFor(a)\n  NEXT\n  io::print(toString(total))\nEND SUB\n"
            ),
        );
        main_live_bytes(&format!("soak_tls_xfer_close_{n}"), &project)
    };
    let at_small = run(30);
    let at_large = run(60);
    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_dir_all(&root);
    let grew = at_large.saturating_sub(at_small);
    assert!(
        grew < BLOCK_BOUND,
        "soak_tls_xfer_close: main-arena live_bytes grew {grew} B between 30 and 60 transferred \
         sockets ({at_small} -> {at_large}); a transferred tls::Socket's ctx is never freed (bug-623)"
    );
}

/// Run `exe` with `RLIMIT_NOFILE` capped at `limit`, returning (exit success, stdout+stderr).
fn run_with_fd_limit(exe: &Path, limit: u64) -> (bool, String) {
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(exe);
    unsafe {
        command.pre_exec(move || {
            let rl = libc::rlimit {
                rlim_cur: limit as libc::rlim_t,
                rlim_max: limit as libc::rlim_t,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &rl) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().expect("run the program under an fd limit");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

/// Found by bug-623's union-alias fix (reading `emit_return_exit_inner`, then measured): a
/// function with two owned sockets and two `RETURN`s — `IF give THEN RETURN a END IF` /
/// `RETURN b` — removed `a`'s close permanently while lowering the first `RETURN`, so the
/// fall-through path that returns `b` never closed `a`. One socket leaked per such call:
/// under a 128-descriptor limit `udp::bind` failed after ~250 calls (`7-707-0003`, exit 255),
/// and `live_bytes` grew 96 B per two calls.
#[test]
fn a_resource_not_returned_on_a_sibling_path_is_still_closed() {
    const SOURCE: &str = "IMPORT io\nIMPORT net\nIMPORT udp\n\nFUNC pick(give AS Boolean) AS RES udp::Socket\n  RES a AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  RES b AS udp::Socket = udp::bind(\"127.0.0.1\", 0)\n  IF give THEN\n    RETURN a\n  END IF\n  RETURN b\nEND FUNC\n\nSUB main()\n  MUT ok AS Integer = 0\n  FOR i = 1 TO {n}\n    RES s AS udp::Socket = pick((i MOD 2) = 0)\n    LET addr AS net::Address = udp::localAddress(s)\n    IF addr.port > 0 THEN\n      ok = ok + 1\n    END IF\n  NEXT\n  io::print(\"ok=\" & toString(ok))\nEND SUB\n";
    let project = common::temp_project("ret_sibling", &SOURCE.replace("{n}", "600"));
    let exe = build_debug_project("ret_sibling_600", &project);
    let (ok, output) = run_with_fd_limit(&exe, 128);
    assert!(
        ok && output.contains("ok=600"),
        "600 calls under a 128-descriptor limit must succeed — the socket a sibling RETURN \
         path does not return must be closed:\n{output}"
    );
    let (_, (_, _, at_small, skips_small)) = debug_run("ret_sibling_flat", SOURCE, 300);
    let (_, (_, _, at_large, skips_large)) = debug_run("ret_sibling_flat", SOURCE, 600);
    assert!(
        skips_small == 0 && skips_large == 0,
        "ret_sibling: double_free_skips {skips_small}/{skips_large}"
    );
    assert!(
        at_large.saturating_sub(at_small) < BLOCK_BOUND,
        "ret_sibling: live_bytes grew {at_small} -> {at_large} between 300 and 600"
    );
}
