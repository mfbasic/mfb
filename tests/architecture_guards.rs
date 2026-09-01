//! Whole-tree architecture guards — filesystem lints, not MIR/serialization
//! tests.
//!
//! Two invariants over the codegen tiers:
//!
//!  1. `shared_lowering_names_no_physical_register` — no hand-picked *physical*
//!     register (`"x9"`, `"rax"`, `"d3"`, …) in an instruction-emission context,
//!     across `src/codegen` AND all of `src/target` (the per-target backends
//!     included — not just `src/target/shared`). Makes the `bug-56`
//!     hand-picked-physical-register class a source lint on top of the
//!     authoritative runtime guard `regalloc::find_physical_operand`.
//!
//!  2. `builtins_no_hand_picked_vreg` — bans hand-*numbered* virtual registers
//!     (`"%v9"`, `"%f3"`) in `src/codegen/builtins`. These are legal (they pass
//!     the physical guard) but they are hand-allocated register identities, the
//!     exact style the minting migration (`temporary_vreg()` /
//!     `allocate_register()`) replaced. The migration is complete, so the count
//!     is a hard floor of 0: any new hand-numbered `"%vN"` fails the test.
//!
//! Both live in `tests/` (an integration-test crate) rather than inside the
//! compiler's module tree, so neither the scan roots nor the self-exemption need
//! to reason about this file.

use std::path::{Path, PathBuf};

/// Files exempt from the physical-register scan: the pure test-fixture files and
/// the two that DEFINE the physical namespace (`abi.rs` = token realization
/// tables, `regalloc/analysis.rs` = the per-ISA occupancy name tables).
fn is_physical_scan_exempt(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let is_regalloc_analysis =
        name == "analysis.rs" && path.parent().is_some_and(|p| p.ends_with("regalloc"));
    matches!(name, "tests.rs" | "test_support.rs" | "abi.rs") || is_regalloc_analysis
}

/// The scannable slice of a source file: everything above the first
/// `#[cfg(test)]` / `mod tests` marker. Register-literal test fixtures (needle
/// lists, round-trip corpora) live below it and are not real emission code.
fn code_above_tests(src: &str) -> &str {
    match src.find("#[cfg(test)]").or_else(|| src.find("mod tests")) {
        Some(i) => &src[..i],
        None => src,
    }
}

/// Recursively collect every `.rs` file under `roots`.
fn rs_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read source dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

/// A quoted physical-register spelling is only a hazard when it sits in an
/// instruction-emission context. Data strings that merely collide with a
/// register name — registry parameter names (`name: "x1"`), vector-constant
/// match arms (`("zero", 2, false)`), doc-example bytes (`toBytes("v1")`) — are
/// NOT emission and must not be flagged. In this codebase every real register
/// operand is threaded through an `abi::` builder or a `.field(...)` operand, so
/// gating on those two markers cleanly separates registers from lookalike data.
/// (Constructed names like `format!("x{n}")` are matched unconditionally — no
/// data string spells one.)
fn is_emission_context(line: &str) -> bool {
    line.contains("abi::") || line.contains(".field(")
}

#[test]
fn shared_lowering_names_no_physical_register() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots = [manifest.join("src/codegen"), manifest.join("src/target")];

    // Every physical spelling the three backends know, per class: AArch64 GPR
    // (x/w), scalar-and-vector FP (d/s/v/q), x86-64 GPR + xmm, riscv64 int + fp
    // ABI names. `sp`, `lr`, `xzr` are neutral spellings and are NOT forbidden.
    let mut forbidden: Vec<String> = Vec::new();
    for n in 0..=30 {
        forbidden.push(format!("\"x{n}\""));
        forbidden.push(format!("\"w{n}\""));
    }
    for prefix in ["d", "s", "v", "q"] {
        for n in 0..=31 {
            forbidden.push(format!("\"{prefix}{n}\""));
        }
    }
    for gpr in ["rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp"] {
        forbidden.push(format!("\"{gpr}\""));
    }
    for n in 8..=15 {
        forbidden.push(format!("\"r{n}\""));
    }
    for n in 0..=15 {
        forbidden.push(format!("\"xmm{n}\""));
    }
    for reg in ["zero", "ra", "gp", "tp"] {
        forbidden.push(format!("\"{reg}\""));
    }
    for n in 0..=6 {
        forbidden.push(format!("\"t{n}\""));
    }
    for n in 0..=7 {
        forbidden.push(format!("\"a{n}\""));
        forbidden.push(format!("\"fa{n}\""));
    }
    for n in 0..=11 {
        forbidden.push(format!("\"ft{n}\""));
        forbidden.push(format!("\"fs{n}\""));
    }
    // A constructed register name evades a literal scan; forbid the constructors
    // outright. (`format!("%v{…` — the vreg minter — is legal; the `%` sentinel
    // cannot collide with a physical name.)
    let constructed: Vec<String> = ["x", "w", "d", "s", "v", "q", "r", "xmm"]
        .iter()
        .map(|p| format!("format!(\"{p}{{"))
        .collect();

    let mut offenders: Vec<String> = Vec::new();
    for path in rs_files(&roots) {
        if is_physical_scan_exempt(&path) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        for (line_no, line) in code_above_tests(&src).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let rel = path.strip_prefix(manifest).unwrap().display();
            if is_emission_context(line) {
                for needle in &forbidden {
                    if line.contains(needle.as_str()) {
                        offenders.push(format!("{rel}:{} names {needle}", line_no + 1));
                    }
                }
            }
            for needle in &constructed {
                if line.contains(needle.as_str()) {
                    offenders.push(format!("{rel}:{} builds {needle}", line_no + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "target-generic lowering must name no physical register in an emission \
         context (bug-56 / plan-34-D); offenders (vreg them, or spell them \
         through a neutral abi token pool):\n{}",
        offenders.join("\n")
    );
}

/// Count `"%vN"` / `"%fN"` literals (a quoted `%`, then `v`/`f`, then a digit).
fn count_hand_picked_vregs(line: &str) -> usize {
    line.match_indices("\"%")
        .filter(|(idx, _)| {
            let mut rest = line[idx + 2..].chars();
            matches!(rest.next(), Some('v') | Some('f'))
                && rest.next().is_some_and(|c| c.is_ascii_digit())
        })
        .count()
}

#[test]
fn builtins_no_hand_picked_vreg() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots = [manifest.join("src/codegen/builtins")];

    let mut count = 0usize;
    let mut sample: Vec<String> = Vec::new();
    for path in rs_files(&roots) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "test_support.rs" || name.ends_with("tests.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        for (line_no, line) in code_above_tests(&src).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let n = count_hand_picked_vregs(line);
            if n > 0 {
                count += n;
                if sample.len() < 15 {
                    let rel = path.strip_prefix(manifest).unwrap().display();
                    sample.push(format!("{rel}:{}", line_no + 1));
                }
            }
        }
    }
    // The migration is complete: every native OS-seam helper mints its scratch
    // through `Vregs::next()` (standalone helpers) or `temporary_vreg()` /
    // `allocate_register()` (CodeBuilder value lowerings), so the allocator owns
    // placement. This is a hard floor of 0 — any new hand-numbered `"%vN"`
    // reopens the class and fails here.
    assert_eq!(
        count, 0,
        "hand-picked virtual-register literals in src/codegen/builtins: {count} \
         (must be 0). New hand-numbered `\"%vN\"` are banned —\n \
         Mint via temporary_vreg()/temporary_fp_vreg()/allocate_register()/allocate_fp_register() so the \
         allocator owns placement. See builder_registers.rs. First offenders:\n{}",
        sample.join("\n")
    );
}

/// Every `abi_function` builtin member registers its OWN per-member lowering in its
/// `func_*.rs` (migrate.md §0: "**one** `lower_<name>` … registered as
/// `Body::abi_function(lower_<name>)`. No `lower_<name>` + `emit_<name>_body`
/// split."). The crypto/io reference migrations do this: `func_flush.rs` →
/// `Body::abi_function(lower_flush)` with the body right there; `func_hash.rs` →
/// `lower_hash`.
///
/// The banned shape — the "shared-dispatcher to fake a migration" deviation — is a
/// package-wide `lower_<pkg>_os_seam` body reached by EVERY member (directly, or via
/// a per-member `lower_<name>` shell that just calls
/// `lower_<pkg>_os_seam(builder, ctx, "pkg.member")`) that switches on `AbiCtx::call`
/// to pick the member. That leaves each `func_*.rs` a shell and the real lowering in
/// one cross-member `match ctx.call` in `gen_*`. `AbiCtx::call` is sanctioned ONLY
/// for one member's own overload aliases (audio's `openInput`/`openInputDevice`),
/// never to dispatch between different members — and the correct references
/// (crypto/io) carry no `_os_seam` construct at all. So: ban the `_os_seam`
/// identifier everywhere in `src/codegen/builtins` (and a `func_*.rs` `body:` that
/// registers a `*_native(` package shell wrapping one). Hard floor of 0.
#[test]
fn no_cross_member_os_seam_dispatcher() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("src/codegen/builtins");
    let mut offenders: Vec<String> = Vec::new();
    for path in rs_files(&[root]) {
        let src = std::fs::read_to_string(&path).expect("read source");
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        for (line_no, line) in code_above_tests(&src).lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            let rel = path.strip_prefix(manifest).unwrap().display();
            // The cross-member dispatcher construct itself — its definition, and every
            // per-member shell that reaches it — all name `_os_seam`.
            if line.contains("_os_seam") {
                offenders.push(format!("{rel}:{}", line_no + 1));
            } else if name.starts_with("func_")
                && line.contains("body:")
                && line.contains("_native(")
            {
                // A `func_*.rs` registering a `<pkg>_native(...)` shell (net's idiom)
                // that returns the shared `_os_seam` body.
                offenders.push(format!(
                    "{rel}:{} (registers a *_native shell body)",
                    line_no + 1
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "cross-member `*_os_seam` shared-dispatcher deviation (migrate.md §0). Each \
         `abi_function` member must carry its OWN `lower_<name>` in its `func_*.rs` \
         (fold a single-use emitter in; keep genuinely-shared code as a `gen_*` helper \
         the per-member bodies CALL), and the `lower_<pkg>_os_seam` / `lower_<pkg>_helper` \
         cross-member `match ctx.call` must be deleted. See fs/io/crypto for the shape. \
         Offenders:\n{}",
        offenders.join("\n")
    );
}

// --- 3. Golden tiers: only tests/syntax/ may pin a compiler diagnostic --------
//
// bug-466. `tests/rt-behavior/tls/tls-timeout-convention-rt` spent months dead:
// plan-110-E Phase 2 migrated it from `net::` to `tcp::` and dropped `IMPORT
// net` while the body still read `bound.port` off the returned `net::Address`,
// so the build failed -- with bug-466's unlocated `native plan has no storage
// class for type 'Unknown'` -- and the golden was regenerated ONTO that
// failure. From then on the harness compared a build failure against a build
// failure and reported PASS, while none of the plan-73-D assertions the fixture
// exists to prove had run since. Its `.run` marker says it was meant to execute.
//
// The tiers already encode the intent, so the invariant is just never checked:
// a `tests/syntax/` fixture exists to pin a DIAGNOSTIC, and everything else --
// `rt-behavior` (build + run), `rt-error` (build, then fail at RUNTIME),
// `byte-identity` (build, compare bytes) -- must COMPILE. A golden build.log
// outside `tests/syntax/` that carries a compiler error is therefore a dead
// fixture, whatever the harness says about it.
//
// Both diagnostic spellings are matched: the bare `error:` of an unlocated
// internal failure (what killed this one) and the located
// `path:line error[CODE NAME]` form. Runtime failures are unaffected -- those
// print `Error: 7-703-0001` from the program, not a compiler diagnostic, and
// several fixtures assert them deliberately.
//
// LIMIT, stated because a green result here is easy to over-read: this checks
// the diagnostic TEXT, so a build that fails without emitting one (a watchdog
// kill, a link failure) is not caught. It is the class that actually occurred.

/// Every `golden/build.log` under `tests/`, with its repo-relative path.
fn golden_build_logs(tests_root: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![tests_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("build.log")
                && path.parent().is_some_and(|p| p.ends_with("golden"))
            {
                let rel = path
                    .strip_prefix(tests_root)
                    .expect("under tests/")
                    .to_string_lossy()
                    .into_owned();
                out.push((path, rel));
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// A line that is a COMPILER diagnostic: the unlocated `error: …` form, or the
/// located `…:N error[2-203-0043 NAME]: …` form. Deliberately not `Error:`,
/// which is a runtime failure several fixtures assert on purpose.
fn is_compiler_diagnostic(line: &str) -> bool {
    line.starts_with("error:") || line.contains(" error[")
}

#[test]
fn only_syntax_goldens_may_pin_a_compiler_diagnostic() {
    let tests_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut offenders: Vec<String> = Vec::new();

    for (path, rel) in golden_build_logs(&tests_root) {
        // `tests/syntax/**` is the diagnostic tier: pinning an error IS its job.
        if rel.starts_with("syntax/") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read golden build.log");
        for (n, line) in src.lines().enumerate() {
            if is_compiler_diagnostic(line) {
                offenders.push(format!("  tests/{rel}:{} — {line}", n + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "bug-466: a golden build.log outside tests/syntax/ pins a COMPILER \
         diagnostic, which means the fixture no longer builds and none of its \
         assertions run -- the harness compares failure against failure and \
         reports PASS. Fix the fixture (or move it under tests/syntax/ if \
         pinning the diagnostic is genuinely the point); do NOT rebaseline the \
         golden onto the failure. Offenders:\n{}",
        offenders.join("\n")
    );
}

// The runtime twin of the guard above (bug-483).
//
// `only_syntax_goldens_may_pin_a_compiler_diagnostic` catches a fixture that
// stopped BUILDING. This catches one that stopped RUNNING: a golden whose
// recorded exit status is a fatal signal. The failure mode is identical — the
// harness compares one crash against another and reports PASS, so every
// assertion below the crash silently stops being checked.
//
// bug-483 is why this exists. A record-layout regression made `net::Address`'s
// String fields unreadable, and the goldens for 34 fixtures were regenerated
// while it was live. Each recorded a crash, and each then passed. Between them
// they covered `net::lookup`, `net::ping`, and nearly all of `tcp`/`udp`'s
// surface — `func_net_ping_valid` alone had 13 assertions below its crash line.
//
// Worse than merely dead: they are flaky-by-crash. The same bad pointer lands as
// SIGSEGV or SIGBUS depending on where it points, so a fixture pinning one
// signal fails intermittently when it hits the other, and a run may die earlier
// than the golden did and lose trailing output. A single acceptance run reported
// 9 of the 34; this check finds all of them, deterministically, without running
// anything.
//
// A behaviour fixture has no legitimate reason to end in a signal: a raise is an
// `Error:` line with a controlled non-zero exit, which this deliberately allows.

/// The shell-recorded exit statuses that mean the process died on a fatal
/// signal: 128 + signum, for the signals a miscompile actually produces.
const CRASH_EXITS: &[(&str, &str)] = &[
    ("[exit 132]", "SIGILL"),
    ("[exit 134]", "SIGABRT"),
    ("[exit 136]", "SIGFPE"),
    ("[exit 138]", "SIGBUS"),
    ("[exit 139]", "SIGSEGV"),
];

#[test]
fn no_golden_pins_a_fatal_signal() {
    let tests_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut offenders: Vec<String> = Vec::new();

    for (path, rel) in golden_build_logs(&tests_root) {
        let src = std::fs::read_to_string(&path).expect("read golden build.log");
        for (n, line) in src.lines().enumerate() {
            if let Some((_, signal)) = CRASH_EXITS.iter().find(|(exit, _)| line.trim() == *exit) {
                offenders.push(format!("  tests/{rel}:{} — {line} ({signal})", n + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "bug-483: a golden build.log records a fatal-signal exit. The fixture \
         crashes, so every assertion after that point is unchecked and the \
         harness still reports PASS -- and because the signal depends on where \
         the bad pointer lands, it fails intermittently rather than honestly. \
         Fix the crash; do NOT rebaseline the golden onto it. Offenders:\n{}",
        offenders.join("\n")
    );
}
