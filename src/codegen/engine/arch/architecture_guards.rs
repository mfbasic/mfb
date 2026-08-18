//! Whole-subtree architecture guards.
//!
//! Not a MIR/serialization test: a filesystem lint that walks every
//! `src/target/shared/**.rs` and enforces the plan-34-D physical-register-name
//! invariant which makes the hand-picked-register bug class (`bug-56`)
//! unrepresentable. It lived inside `mir.rs`'s round-trip `mod tests` — the
//! most load-bearing guard in the subtree, findable only by reading a MIR
//! serialization module. Extracted here (bug-334 D2) so its filename says what
//! it is.
//!
//! Self-exemption: the scan truncates each file at its first `#[cfg(test)]` (or
//! `mod tests`) marker, so this file's own test body — which necessarily spells
//! every forbidden register to build the needle list — is never scanned.

// --- codegen tier imports (migration) ---
#[cfg(test)]
mod tests {
    /// plan-34-D (superseding plan-34-C Phase 5) — the invariant that makes the
    /// hand-picked-register bug class (`bug-56`) *unrepresentable*: no shared
    /// source under `src/target/shared/` may spell a physical register of ANY
    /// class or ISA. Allocator-reachable scratch is a virtual register
    /// (`%vN`/`%fN`); machine-floor and kernel scratch is a neutral token pool
    /// (`abi::SCRATCH`/`FP_SCRATCH`/`VEC_SCRATCH`); the call boundary is role
    /// tokens (plan-34-B); pinned/invariant registers are tokens (plan-34-A,
    /// `%thread`/`%closure_env`/`%mathpool`, guarded in `mir.rs`). Both quoted
    /// literals (`"x13"`, `"d3"`, `"v0"`, `"rsi"`, `"a0"`, …) and
    /// `format!`-constructed names (`format!("x{n}")`) are forbidden — there is
    /// no allowlist. The only files exempt are the two that DEFINE the physical
    /// namespace: `abi.rs` (the token realization tables) and
    /// `regalloc/analysis.rs` (the occupancy parsers' per-ISA name tables), plus
    /// the pure test-fixture files `tests.rs`/`test_support.rs`.
    /// `#[cfg(test)]` fixtures and full-line comments are skipped (tests pin
    /// realization behavior; prose may cite spellings). The companion runtime
    /// guard is `regalloc::find_physical_operand`, asserted on every stream
    /// entering selection/allocation and on the machine-floor builders.
    #[test]
    fn shared_lowering_names_no_physical_register() {
        use std::path::Path;
        // Every physical spelling the three backends know, per class:
        // AArch64 GPR (x/w), scalar-and-vector FP (d/s/v/q), x86-64 GPR + xmm,
        // riscv64 int + fp ABI names. `sp`, `lr`, and `xzr` are the neutral
        // spellings (plan-34-A) and are NOT forbidden.
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
        // A constructed register name evades a literal scan; forbid the
        // constructors outright. (`format!("%v{…` — the vreg minter — is legal:
        // the `%` sentinel cannot collide with a physical name.)
        let constructed: Vec<String> = ["x", "w", "d", "s", "v", "q", "r", "xmm"]
            .iter()
            .map(|p| format!("format!(\"{p}{{"))
            .collect();

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/target/shared");
        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read shared dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let name = path.file_name().unwrap().to_str().unwrap().to_string();
                // Exact allowlist (bug-334 D2): the pure test-fixture files carry
                // register-literal fixtures, and `abi.rs` / `regalloc/analysis.rs`
                // DEFINE the physical namespace. This replaces a former
                // `name.contains("test")` substring match, which would silently
                // and permanently exempt any future file whose name merely
                // contains "test" (`latest.rs`, `fastest.rs`, a
                // `builder_test_helpers.rs`, a `tests/` subdir file) from the
                // invariant, with no diagnostic.
                let is_regalloc_analysis =
                    name == "analysis.rs" && path.parent().is_some_and(|p| p.ends_with("regalloc"));
                if matches!(name.as_str(), "tests.rs" | "test_support.rs" | "abi.rs")
                    || is_regalloc_analysis
                {
                    continue;
                }
                let src = std::fs::read_to_string(&path).expect("read source");
                // Scan only above the test module (register-literal test fixtures).
                let code = match src.find("#[cfg(test)]").or_else(|| src.find("mod tests")) {
                    Some(i) => &src[..i],
                    None => &src,
                };
                for (line_no, line) in code.lines().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    for needle in forbidden.iter().chain(constructed.iter()) {
                        if line.contains(needle.as_str()) {
                            offenders.push(format!(
                                "{}:{} names {needle}",
                                path.strip_prefix(&root).unwrap().display(),
                                line_no + 1
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "shared lowering must name no physical register (plan-34-D); offenders \
             (vreg them, or spell them through a neutral abi token pool):\n{}",
            offenders.join("\n")
        );
    }
}
