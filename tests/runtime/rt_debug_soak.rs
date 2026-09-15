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
    let at_small = main_live_bytes(&format!("{case}_{small}"), &make(small));
    let at_large = main_live_bytes(&format!("{case}_{large}"), &make(large));
    let grew = at_large.saturating_sub(at_small);
    assert!(
        grew < FLAT_BOUND,
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
#[ignore = "bug-625: an AttributedString leaks one block on drop (bug-620/621 layout temps fixed); run with --include-ignored"]
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
#[ignore = "bug-622: thread::start and thread::waitFor leak the result copy and the thread plumbing in the parent arena; run with --include-ignored"]
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
#[ignore = "bug-623: http::read leaks the tcp::read buffer and per-connection records; run with --include-ignored"]
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
