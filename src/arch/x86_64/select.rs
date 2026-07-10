//! x86-64 instruction selection (plan-00-H): neutral MIR → x86-64 machine ops.
//!
//! The x86 counterpart of `arch::aarch64::select`. It consumes the shared
//! neutral [`MirInstruction`] stream (via `mir::Backend::select`) and produces
//! [`CodeInstruction`]s with x86/SysV registers, using the shared MIR primitives
//! (`fused_setter_codeop`, `rename_field_values`, …) — so all the ISA-specific
//! selection lives here, not in shared `mir.rs`.

use crate::arch::aarch64::ops::CodeOp;
use crate::arch::x86_64::regmodel::ZERO_REGISTER;
use crate::target::shared::code::mir::{
    fused_setter_codeop, MirInstruction, MirOp, ARENA_BASE, FUSED_COND_FIELD, FUSED_SHARE_FIELD,
};
use crate::target::shared::abi;
use crate::target::shared::code::CodeInstruction;

/// Map residual AArch64 scratch `xN` (N ≥ 9) to an x86 GPR (encoding-only; see
/// the call site). Avoids `r14` (zero), `r15` (arena_base), and `rsp`.
fn map_scratch_register(n: usize) -> &'static str {
    // rax and rdx are excluded: `mul`/`imul`/`div`/`idiv`/`cqo` use them
    // *implicitly* (dividend/quotient in rax, high-half/remainder in rdx), so a
    // long-lived scratch value mapped there would be silently destroyed across a
    // division or wide multiply — e.g. the digit-loop divisor `10` in
    // `emit_write_integer_to_stderr` lived across the `div` that clobbers rdx.
    //
    // Ordering matters: the hand-written helpers inherit the AArch64 convention
    // that x19–x28 are *callee-saved* — values parked there survive an
    // intervening `call`/`syscall` (e.g. the entry's error message in x20 across
    // the code-printing `write` syscall, which clobbers rcx; argc/argv in x27/x28
    // across `clock_gettime`). So the pool is arranged so those high registers
    // land on x86's callee-saved bank (rbx/rbp/r12/r13): with the `(n-9) % 11`
    // index, x20→rbx, x27→r12, x28→r13, x19→rbp. The low scratch (x8–x18, not
    // parked across calls) takes the caller-saved remainder (rcx/rsi/rdi/r8–r11).
    const POOL: &[&str] = &[
        "rbx", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "rcx", "rbp",
    ];
    POOL[(n - 9) % POOL.len()]
}

// SysV: call args rdi,rsi,rdx,rcx,r8,r9; syscall args rdi,rsi,rdx,r10,r8,r9;
// returns rax,rdx; syscall nr + result rax.
// SysV integer argument registers, extended with two INTERNAL argument
// registers for `x6`/`x7`: MFBASIC functions take up to 8 parameters and
// AArch64 has 8 argument registers, but SysV only has 6 — so internal calls
// pass the 7th in `rax` (dead at a call site: the variadic al marker is only
// emitted for external libc calls, see the `bl` encoder) and the 8th in `rbp`
// (reserved, never allocated, and no vregified builder function names it).
// Libc calls never exceed 6 integer args, so the extension is internal-only
// in practice.
const CALL_ARGS: &[&str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9", "rax", "rbp"];
const SYS_ARGS: &[&str] = &["rdi", "rsi", "rdx", "r10", "r8", "r9"];
// x0/x1 are the SysV return registers (rax/rdx). x2/x3 extend the set only for
// the runtime's 4-register error-Result convention (tag=x0, value=x1,
// message=x2, source=x3), which `make_error_result` produces and the error/TRAP
// path consumes immediately (no intervening call), so caller-saved rcx/rsi are
// safe distinct homes. Without these, x2/x3 fell back to rax and aliased,
// corrupting propagated errors.
const RETS: &[&str] = &["rax", "rdx", "rcx", "rsi"];

/// Remap the residual AArch64 register spellings a selected stream still carries
/// to their x86-64 / SysV homes. Call-boundary registers arrive as explicit role
/// tokens (`%arg`/`%ret`/`%sysarg`/`%sysnr`/`%closure_env`, plan-34-B), so their
/// SysV home is a direct table lookup — the control-flow role inference this pass
/// used to run is gone. Virtual registers (`%vN`), `arena_base` (already `r15`),
/// and the zero token (`xzr`, materialized by the encoder) pass through; residual
/// physical scratch (`x9`+) maps by pool.
fn remap_x86_abi(instructions: &mut Vec<CodeInstruction>) {
    // The link register has no x86 equivalent — `call` pushes / `ret` pops the
    // return address — so drop the frame's LR save/restore entirely. Shared code
    // spells it with the neutral `abi::LR` token (`"lr"`); the `"x30"` spelling is
    // still accepted from any non-shared producer (plan-34-A).
    instructions
        .retain(|inst| !inst.fields.iter().any(|(_, value)| value == "x30" || value == "lr"));
    for inst in instructions.iter_mut() {
        for (_, value) in inst.fields.iter_mut() {
            if let Some(mapped) = map_x86_operand(value) {
                *value = mapped;
            }
        }
    }
}

/// Map one operand of a selected stream to its x86-64 home, or `None` to leave it
/// unchanged (virtual registers `%vN`, the pinned `arena_base` = `r15`, the zero
/// token `xzr`, immediates, labels, symbols).
fn map_x86_operand(value: &str) -> Option<String> {
    // Stack pointer and the AArch64 zero-register alias `x31`.
    if value == "sp" {
        return Some("rsp".to_string());
    }
    if value == "x31" {
        return Some(ZERO_REGISTER.to_string());
    }
    // Physical FP registers `dN`/`vN`/`qN` (N < 16) alias `xmmN` (the NEON `v`/`q`
    // banks share the `d` register file).
    if let Some(fp) = value
        .strip_prefix(['d', 'v', 'q'])
        .and_then(|rest| rest.parse::<usize>().ok())
        .filter(|n| *n < 16)
    {
        return Some(format!("xmm{fp}"));
    }
    // Call-boundary ROLE TOKENS (plan-34-B Phase 4): the SysV home is an explicit
    // table lookup, no CFG role inference. An out-of-range index falls back to rax
    // (unreachable for the emitted arities; matches the former `map_abi_register`
    // bound).
    if let Some(n) = value.strip_prefix("%arg").and_then(|r| r.parse::<usize>().ok()) {
        return Some(CALL_ARGS.get(n).copied().unwrap_or("rax").to_string());
    }
    if let Some(n) = value.strip_prefix("%ret").and_then(|r| r.parse::<usize>().ok()) {
        return Some(RETS.get(n).copied().unwrap_or("rax").to_string());
    }
    if let Some(n) = value
        .strip_prefix("%sysarg")
        .and_then(|r| r.parse::<usize>().ok())
    {
        return Some(SYS_ARGS.get(n).copied().unwrap_or("rax").to_string());
    }
    if value == abi::SYSNR || value == abi::SYSRET {
        // The syscall number and the syscall result both live in `rax` on x86-64.
        return Some("rax".to_string());
    }
    if value == abi::CLOSURE_ENV {
        // The closure-env pointer inherits `x28`'s callee-saved x86 home (`r13`).
        return Some(map_scratch_register(28).to_string());
    }
    // Residual bare AArch64 registers.
    if let Some(n) = value
        .strip_prefix('x')
        .and_then(|rest| rest.parse::<usize>().ok())
        .filter(|n| *n <= 30)
    {
        if n > 8 {
            // Caller/callee-saved scratch (`x9`–`x30`) maps to an x86 GPR by pool.
            return Some(map_scratch_register(n).to_string());
        }
        // A residual bare ABI register (`x0`–`x8`) is genuine scratch that Phase 3b
        // left un-tokenized (the TLS handshake-timeout `tv` math temporaries):
        // reproduce the old no-boundary fallback and map it to that index's RETS
        // home (`x1` → `rdx`), which no live token collides with here.
        return Some(RETS.get(n).copied().unwrap_or("rax").to_string());
    }
    // `%vN`, `xzr`, `r14`/`r15`, immediates, labels, symbols: unchanged.
    None
}

/// Rewrite the flag-reading branch of a fused *float* compare into the x86
/// branch(es) that read `ucomisd`'s CF/ZF/PF with IEEE-754 unordered semantics.
///
/// After `ucomisd lhs, rhs` (`lhs` vs `rhs`): `CF=1` iff `lhs<rhs` or unordered;
/// `ZF=1` iff `lhs=rhs` or unordered; `PF=1` iff unordered (either is NaN). The
/// AArch64 `b.cc` mnemonics were chosen for `fcmp`'s NZCV, which differs, so the
/// integer `b.cc → jcc` mapping mishandles every NaN case. The mapping below
/// reproduces each AArch64 float relation's *exact* truth set on x86:
///
/// - `>`/`>=` (`b.gt`/`b.ge`) → `ja`/`jae`: `CF=0` already excludes unordered.
/// - `<`/`<=`/`=` (`b.mi`/`b.ls`/`b.eq`) → `jp skip; jb|jbe|je target; skip:`:
///   `jb`/`jbe`/`je` alone would also fire on unordered (CF/ZF set), so a leading
///   `jp` skips the branch when unordered (PF=1), yielding the ordered-only set.
/// - `<>` (`b.ne`) → `jp target; jne target`: true on unordered *or* ordered-≠.
/// - `b.lt`/`b.le` (integer-style `<`/`<=`, unordered ⇒ true) → `jb`/`jbe`.
/// - `b.vs`/`b.vc` (NaN / not-NaN finiteness checks) → `jp`/`jnp`.
///
/// `site` is a per-function index that makes each synthesized skip label unique.
/// Naming it from `target` alone let two ordered-only branches to the same label
/// (e.g. `IF a < b OR c < d THEN GOTO L`) emit two labels of the same name; the
/// encoder's name-keyed label map is last-writer-wins, so the first `jp` resolved
/// to the *second* label and a NaN first operand jumped clean over the second
/// comparison (bug-15).
fn x86_float_branch(cond: &str, target: &str, site: usize) -> Vec<CodeInstruction> {
    // Emit ONLY `x86.*`-namespaced branches: this function's output is re-lowered
    // (`route_function_through_mir`) after selection, and a real AArch64 `b.cc`
    // sitting right after the `fcmp` would re-fuse and be remapped a second time.
    // The `x86.*` ops are not flag-reading branches for `lower_to_mir`, so the
    // stream is a fixed point on the second pass.
    let br = |mnemonic: &str, tgt: &str| CodeInstruction::new(mnemonic).field("target", tgt);
    // `jp skip; <cc> target; skip:` — take <cc> only when ordered (PF clear).
    let ordered_only = |cc: &str| {
        let skip = format!("{target}__x86ford{site}");
        vec![
            br("x86.jp", &skip),
            br(cc, target),
            CodeInstruction::new("label").field("name", &skip),
        ]
    };
    match cond {
        "b.gt" => vec![br("x86.ja", target)], // ja  (CF=0 && ZF=0)          {GT}
        "b.ge" => vec![br("x86.jae", target)], // jae (CF=0)                 {EQ,GT}
        "b.mi" => ordered_only("x86.jb"),     // jb  (CF=1), NaN-excluded     {LT}
        "b.lo" => ordered_only("x86.jb"),     // b.lo(C=0)==LT after fcmp     {LT}
        "b.ls" => ordered_only("x86.jbe"),    // jbe (CF=1 || ZF=1), NaN-excl {LT,EQ}
        "b.eq" => ordered_only("x86.je"),     // je  (ZF=1), NaN-excluded     {EQ}
        "b.ne" => vec![br("x86.jp", target), br("x86.jne", target)], // jp||jne {LT,GT,uno}
        "b.hi" => vec![br("x86.jp", target), br("x86.ja", target)], // jp||ja  {GT,uno}
        "b.lt" => vec![br("x86.jb", target)], // jb  (CF=1) — LT or unordered {LT,uno}
        "b.le" => vec![br("x86.jbe", target)], // jbe (CF=1 || ZF=1)          {LT,EQ,uno}
        "b.vs" => vec![br("x86.jp", target)], // jp  (PF=1 → unordered/NaN)   {uno}
        "b.vc" => vec![br("x86.jnp", target)], // jnp (PF=0 → ordered)        {LT,EQ,GT}
        other => panic!("unmapped x86 float-compare branch condition '{other}'"),
    }
}

/// Select neutral MIR into x86-64 machine ops (plan-00-H). Mirrors the AArch64
/// selection's structural conversion — `addr_of` becomes a single RIP-relative
/// load (`adrp{dst,symbol}`, which the x86 encoder emits as `lea`; the page-pair
/// `add_pageoff` is unused), a fused flagless op splits into its `cmp`/`adds`/…
/// setter + the flag-reading branch (x86 `cmp; jcc` works the same way), and
/// `arena_base` realizes to the pinned `r15` — then remaps the residual AArch64
/// ABI registers to their SysV homes ([`remap_x86_abi`]).
pub(crate) fn select_x86(instructions: &[MirInstruction]) -> Vec<CodeInstruction> {
    let mut out = Vec::with_capacity(instructions.len());
    // Distinguishes the skip label of every ordered-only float branch in this
    // function (see `x86_float_branch`).
    let mut float_branch_site = 0_usize;
    for instruction in instructions {
        if instruction.op == MirOp::AddrOf {
            // Single RIP-relative reference (no aarch64 page pair): the x86
            // encoder turns `adrp{dst,symbol}` into `lea dst,[rip+disp32]`.
            out.push(CodeInstruction {
                op: CodeOp::Adrp,
                fields: instruction.fields.clone(),
            });
            continue;
        }
        if let Some(setter_op) = fused_setter_codeop(instruction.op) {
            let split = instruction
                .fields
                .iter()
                .position(|(key, _)| *key == FUSED_COND_FIELD)
                .expect("fused MIR op carries a cond field");
            let setter_fields = instruction.fields[..split].to_vec();
            let branch_op = CodeOp::from_mnemonic(&instruction.fields[split].1)
                .expect("fused MIR op carries a valid branch mnemonic");
            let mut branch_fields = Vec::new();
            let mut shared = false;
            for (key, value) in &instruction.fields[split + 1..] {
                if *key == FUSED_SHARE_FIELD {
                    shared = true;
                } else {
                    branch_fields.push((*key, value.clone()));
                }
            }
            if !shared {
                out.push(CodeInstruction {
                    op: setter_op,
                    fields: setter_fields,
                });
            }
            // A branch reading a float compare's flags needs the x86 IEEE remap:
            // `ucomisd` sets CF/ZF/PF (not the AArch64 NZCV the `b.cc` mnemonics
            // read), and an unordered (NaN) result sets CF=ZF=PF=1, so the naive
            // integer `b.cc → jcc` mapping mishandles every NaN case. Rewrite the
            // branch here where the setter kind is known.
            if matches!(setter_op, CodeOp::FCmpD | CodeOp::FCmpZeroD) {
                let target = branch_fields
                    .iter()
                    .find(|(k, _)| *k == "target")
                    .map(|(_, v)| v.clone())
                    .expect("float compare branch carries a target");
                for inst in x86_float_branch(&instruction.fields[split].1, &target, float_branch_site)
                {
                    out.push(inst);
                }
                float_branch_site += 1;
            } else {
                out.push(CodeInstruction {
                    op: branch_op,
                    fields: branch_fields,
                });
            }
        } else {
            // Non-fused MIR ops map 1:1 to a CodeOp via `to_code` (which applies
            // the neutral→concrete renames, e.g. `call`→`bl`); the x86 encoder
            // realizes each CodeOp as x86 bytes.
            out.push(CodeInstruction {
                op: instruction
                    .op
                    .to_code()
                    .expect("non-fused MIR op maps to a single CodeOp"),
                fields: instruction.fields.clone(),
            });
        }
    }
    for instruction in &mut out {
        crate::target::shared::code::mir::rename_field_values(
            &mut instruction.fields,
            ARENA_BASE,
            "r15",
        );
    }
    // plan-34-B Phase 4: the role tokens flow straight into `remap_x86_abi`, which
    // looks up each one's SysV home directly (no more `realize_abi_token` seam and
    // no control-flow role inference).
    remap_x86_abi(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::code::mir::lower_to_mir;

    /// Build one aarch64-form `CodeInstruction`.
    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut ins = CodeInstruction::new(op);
        for (k, v) in fields {
            ins = ins.field(k, v);
        }
        ins
    }

    /// Select a stream from aarch64-form instructions.
    fn sel(instructions: &[CodeInstruction]) -> Vec<CodeInstruction> {
        select_x86(&lower_to_mir(instructions))
    }

    /// Every field value in the selected stream, flattened.
    fn values(out: &[CodeInstruction]) -> Vec<String> {
        out.iter()
            .flat_map(|inst| inst.fields.iter().map(|(_, v)| v.clone()))
            .collect()
    }

    #[test]
    fn addr_of_becomes_lea_and_pageoff_drops() {
        // adrp; add_pageoff (same reg + symbol) fuses to addr_of, selected as Adrp.
        let out = sel(&[
            ci("adrp", &[("dst", "x9"), ("symbol", "g")]),
            ci(
                "add_pageoff",
                &[("dst", "x9"), ("src", "x9"), ("symbol", "g")],
            ),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].op, CodeOp::Adrp);
        // x9 scratch mapped to an x86 GPR.
        assert!(out[0].fields.iter().any(|(k, v)| *k == "dst" && v != "x9"));
    }

    #[test]
    fn sp_zero_and_fp_register_mapping() {
        let out = sel(&[
            ci("mov", &[("dst", "x9"), ("src", "sp")]),
            ci("mov", &[("dst", "x10"), ("src", "x31")]),
            ci("fmov_d_from_d", &[("dst", "d0"), ("src", "d3")]),
            ci("fadd_v", &[("dst", "v1"), ("lhs", "v2"), ("rhs", "q3")]),
            ci("ret", &[]),
        ]);
        let vals = values(&out);
        assert!(vals.contains(&"rsp".to_string()));
        assert!(vals.contains(&ZERO_REGISTER.to_string()));
        assert!(vals.contains(&"xmm0".to_string()));
        assert!(vals.contains(&"xmm3".to_string()));
        assert!(vals.contains(&"xmm1".to_string()));
        assert!(vals.contains(&"xmm2".to_string()));
    }

    #[test]
    fn call_argument_and_return_mapping() {
        // The `%argN` role tokens are the SysV call-argument bank (rdi, rsi, rdx,
        // rcx, r8, r9); `%ret0` read after the call is the result (rax). Phase 4:
        // a direct token lookup, no boundary inference.
        let out = sel(&[
            ci("mov_imm", &[("dst", "%arg0"), ("value", "1")]),
            ci("mov_imm", &[("dst", "%arg1"), ("value", "2")]),
            ci("mov_imm", &[("dst", "%arg2"), ("value", "3")]),
            ci("mov_imm", &[("dst", "%arg3"), ("value", "4")]),
            ci("mov_imm", &[("dst", "%arg4"), ("value", "5")]),
            ci("mov_imm", &[("dst", "%arg5"), ("value", "6")]),
            ci("bl", &[("target", "_mfb_f")]),
            ci("mov", &[("dst", "x9"), ("src", "%ret0")]),
            ci("ret", &[]),
        ]);
        let vals = values(&out);
        for arg in ["rdi", "rsi", "rdx", "rcx", "r8", "r9"] {
            assert!(vals.contains(&arg.to_string()), "missing arg {arg}");
        }
        // `%ret0` → rax (result).
        assert!(vals.contains(&"rax".to_string()));
    }

    #[test]
    fn internal_seventh_eighth_call_args() {
        // `%arg6`/`%arg7` (the 7th/8th internal call args) map to rax and rbp.
        let out = sel(&[
            ci("mov_imm", &[("dst", "%arg6"), ("value", "7")]),
            ci("mov_imm", &[("dst", "%arg7"), ("value", "8")]),
            ci("bl", &[("target", "_mfb_f")]),
            ci("ret", &[]),
        ]);
        let vals = values(&out);
        assert!(vals.contains(&"rax".to_string()));
        assert!(vals.contains(&"rbp".to_string()));
    }

    #[test]
    fn syscall_argument_and_number_mapping() {
        // `%sysargN` are the syscall-arg bank (rdi rsi rdx r10 r8 r9) — distinct
        // from `%argN` at index 3 (r10, not rcx) — and `%sysnr` is the syscall
        // number (rax).
        let out = sel(&[
            ci("mov_imm", &[("dst", "%sysarg0"), ("value", "1")]),
            ci("mov_imm", &[("dst", "%sysarg3"), ("value", "4")]),
            ci("mov_imm", &[("dst", "%sysnr"), ("value", "60")]),
            ci("svc", &[]),
            ci("ret", &[]),
        ]);
        let vals = values(&out);
        assert!(vals.contains(&"rdi".to_string())); // %sysarg0
        assert!(vals.contains(&"r10".to_string())); // %sysarg3 (not rcx)
        assert!(vals.contains(&"rax".to_string())); // %sysnr
    }

    #[test]
    fn scratch_register_pool_wraps() {
        // High scratch registers land on the pool; x20 → rbx, x27 → r12, x28 → r13.
        assert_eq!(map_scratch_register(20), "rbx");
        assert_eq!(map_scratch_register(27), "r12");
        assert_eq!(map_scratch_register(28), "r13");
        assert_eq!(map_scratch_register(19), "rbp");
        assert_eq!(map_scratch_register(9), "rbx");
    }

    #[test]
    fn operand_token_and_scratch_mapping() {
        // The direct token lookup (plan-34-B Phase 4): each role token maps to its
        // SysV home; a residual bare `x0`–`x8` scratch falls back to its RETS home;
        // `%vN`/`xzr` pass through unchanged.
        assert_eq!(map_x86_operand("%arg1").as_deref(), Some("rsi"));
        assert_eq!(map_x86_operand("%ret1").as_deref(), Some("rdx"));
        assert_eq!(map_x86_operand("%sysarg3").as_deref(), Some("r10"));
        assert_eq!(map_x86_operand("%sysnr").as_deref(), Some("rax"));
        assert_eq!(map_x86_operand("%sysret").as_deref(), Some("rax"));
        // `%closure_env` inherits x28's callee-saved home.
        assert_eq!(map_x86_operand("%closure_env").as_deref(), Some("r13"));
        // A residual bare ABI register with no token → that index's RETS home.
        assert_eq!(map_x86_operand("x1").as_deref(), Some("rdx"));
        // High scratch maps by pool; virtuals and the zero token pass through.
        assert_eq!(map_x86_operand("x20").as_deref(), Some("rbx"));
        assert_eq!(map_x86_operand("%v3"), None);
        assert_eq!(map_x86_operand("xzr"), None);
    }

    #[test]
    fn x30_link_register_is_dropped() {
        // A frame save of x30 (link register) is removed entirely.
        let out = sel(&[
            ci(
                "str_u64",
                &[("src", "x30"), ("base", "sp"), ("offset", "0")],
            ),
            ci("ret", &[]),
        ]);
        assert!(!values(&out).iter().any(|v| v == "x30"));
    }

    #[test]
    fn float_compare_branch_rewrites() {
        // Each fcmp_d + b.cc pair rewrites into the x86 IEEE branch sequence.
        // b.gt → ja ; b.ge → jae (single branch).
        let out = sel(&[
            ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
            ci("b.gt", &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert!(out.iter().any(|i| i.op.mnemonic() == "x86.ja"));

        for (cond, expect) in [
            ("b.ge", "x86.jae"),
            ("b.lt", "x86.jb"),
            ("b.le", "x86.jbe"),
            ("b.vs", "x86.jp"),
            ("b.vc", "x86.jnp"),
        ] {
            let out = sel(&[
                ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
                ci(cond, &[("target", "L")]),
                ci("ret", &[]),
            ]);
            assert!(
                out.iter().any(|i| i.op.mnemonic() == expect),
                "cond {cond} should emit {expect}"
            );
        }
    }

    #[test]
    fn float_compare_ordered_only_and_multi_branch() {
        // b.mi / b.ls / b.eq emit `jp skip; jcc target; skip:` (3 instructions).
        for (cond, cc) in [("b.mi", "x86.jb"), ("b.ls", "x86.jbe"), ("b.eq", "x86.je")] {
            let out = sel(&[
                ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
                ci(cond, &[("target", "L")]),
                ci("ret", &[]),
            ]);
            assert!(out.iter().any(|i| i.op.mnemonic() == "x86.jp"));
            assert!(out.iter().any(|i| i.op.mnemonic() == cc));
            assert!(out.iter().any(|i| i.op.mnemonic() == "label"));
        }
        // b.ne → jp target; jne target ; b.hi → jp target; ja target.
        let ne = sel(&[
            ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
            ci("b.ne", &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert!(ne.iter().any(|i| i.op.mnemonic() == "x86.jne"));
        let hi = sel(&[
            ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
            ci("b.hi", &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert!(hi.iter().any(|i| i.op.mnemonic() == "x86.ja"));
        // b.lo → ordered-only jb.
        let lo = sel(&[
            ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
            ci("b.lo", &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert!(lo.iter().any(|i| i.op.mnemonic() == "x86.jb"));
    }

    #[test]
    fn ordered_only_skip_labels_are_unique_per_branch_site() {
        // bug-15: two ordered-only float branches to the SAME target (e.g.
        // `IF a < b OR c < d THEN GOTO L`) once emitted two labels both named
        // `L__x86ford`. The encoder's label map is last-writer-wins, so the first
        // `jp` resolved to the second label and a NaN first operand skipped the
        // second comparison entirely.
        let out = sel(&[
            ci("fcmp_d", &[("lhs", "d0"), ("rhs", "d1")]),
            ci("b.mi", &[("target", "L")]),
            ci("fcmp_d", &[("lhs", "d2"), ("rhs", "d3")]),
            ci("b.mi", &[("target", "L")]),
            ci("label", &[("name", "L")]),
            ci("ret", &[]),
        ]);
        let labels: Vec<String> = out
            .iter()
            .filter(|i| i.op == CodeOp::Label)
            .map(|i| i.fields[0].1.clone())
            .collect();
        assert_eq!(labels.len(), 3, "two skip labels + the shared target: {labels:?}");
        let skips: Vec<&String> = labels.iter().filter(|n| n.contains("__x86ford")).collect();
        assert_eq!(skips.len(), 2);
        assert_ne!(skips[0], skips[1], "skip labels collide: {skips:?}");
        // Each `jp` targets its own skip label, which sits right after its `jb`.
        let jps: Vec<&String> = out
            .iter()
            .filter(|i| i.op.mnemonic() == "x86.jp")
            .map(|i| &i.fields[0].1)
            .collect();
        assert_eq!(jps, skips);
    }

    #[test]
    fn fcmp_zero_branch_rewrite() {
        // A compare-against-zero fused branch also takes the float remap.
        let out = sel(&[
            ci("fcmp_zero_d", &[("src", "d0")]),
            ci("b.mi", &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert_eq!(out[0].op, CodeOp::FCmpZeroD);
        assert!(out.iter().any(|i| i.op.mnemonic().starts_with("x86.")));
    }

    #[test]
    #[should_panic(expected = "unmapped x86 float-compare branch condition")]
    fn float_branch_unmapped_condition_panics() {
        // A non-flag condition reaching x86_float_branch panics.
        x86_float_branch("b.pl", "L", 0);
    }

    #[test]
    fn integer_compare_branch_not_remapped() {
        // An integer cmp + branch keeps the standard b.cc → jcc path (no x86.*).
        let out = sel(&[
            ci("cmp", &[("lhs", "x9"), ("rhs", "x10")]),
            ci("b.eq", &[("target", "L")]),
            ci("ret", &[]),
        ]);
        assert!(out.iter().any(|i| i.op == CodeOp::BranchEq));
        assert!(!out.iter().any(|i| i.op.mnemonic().starts_with("x86.")));
    }

    #[test]
    fn incoming_parameter_maps_to_arg_register() {
        // An incoming parameter is spelled `%arg1` and maps straight to its SysV
        // delivery register rsi — the Phase-3b bridge prologue is gone (the store
        // reads the argument register directly).
        let out = sel(&[
            ci("label", &[("name", "entry")]),
            ci("str_u64", &[("src", "%arg1"), ("base", "sp"), ("offset", "8")]),
            ci("ret", &[]),
        ]);
        // The store source is the SysV arg register rsi.
        assert!(values(&out).iter().any(|v| v == "rsi"));
        // The first instruction is still the entry label (no bridge inserted).
        assert_eq!(out[0].op, CodeOp::Label);
        assert_eq!(out.len(), 3, "no prologue bridge is inserted");
    }

    #[test]
    fn staged_error_result_def_and_use_colored_rets() {
        // The 4-register error-Result stages a value into `%ret1`; both the def and
        // any later read map to its RETS home (rdx), matching the callee — a direct
        // token lookup, no staged-result inference.
        let out = sel(&[
            ci("bl", &[("target", "_mfb_make_err")]),
            ci("mov", &[("dst", "%ret1"), ("src", "x9")]), // staged def
            ci("b", &[("target", "stage")]),
            ci("label", &[("name", "dead")]),
            ci("ret", &[]),
            ci("label", &[("name", "stage")]),
            ci("str_u64", &[("src", "%ret1"), ("base", "sp"), ("offset", "0")]), // read
            ci("ret", &[]),
        ]);
        // `%ret1` → rdx (RETS[1]) at both the def and the reset-block store.
        let vals = values(&out);
        assert!(vals.contains(&"rdx".to_string()));
        assert!(!vals.iter().any(|v| v == "x1"));
    }

    #[test]
    fn same_block_staged_use_reads_rets() {
        // A `%ret0` def followed by a same-block use both map to rax (RETS[0]) —
        // the token names the role directly.
        let out = sel(&[
            ci("bl", &[("target", "_mfb_make_err")]),
            ci("mov", &[("dst", "%ret0"), ("src", "x9")]), // staged def
            ci("cmp", &[("lhs", "%ret0"), ("rhs", "x10")]), // same-block use
            ci("b", &[("target", "stage")]),
            ci("label", &[("name", "stage")]),
            ci("str_u64", &[("src", "%ret0"), ("base", "sp"), ("offset", "0")]),
            ci("ret", &[]),
        ]);
        // No x0 residue — `%ret0` resolved to rax everywhere.
        let vals = values(&out);
        assert!(vals.contains(&"rax".to_string()));
        assert!(!vals.iter().any(|v| v == "x0"));
    }

    #[test]
    fn residual_bare_abi_register_falls_back_to_rets() {
        // A residual bare `x0`–`x8` scratch that Phase 3b left un-tokenized maps to
        // that index's RETS home (x0 → rax), reproducing the old no-boundary arm.
        let out = sel(&[
            ci("mov_imm", &[("dst", "x0"), ("value", "1")]),
            ci("b", &[("target", "nowhere")]),
            ci("ret", &[]),
        ]);
        assert!(values(&out).iter().any(|v| v == "rax"));
    }

    #[test]
    fn arena_base_realizes_to_r15() {
        // The AArch64 arena-base realization register, once lowered to the neutral
        // `arena_base` and selected, becomes the x86 pin r15.
        let realization = crate::target::shared::code::mir::arena_base_realization();
        let out = sel(&[
            ci(
                "ldr_u64",
                &[("dst", "x9"), ("base", realization), ("offset", "0")],
            ),
            ci("ret", &[]),
        ]);
        assert!(values(&out).iter().any(|v| v == "r15"));
    }
}
