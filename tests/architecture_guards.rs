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
