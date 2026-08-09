//! x86-64 instruction selection (plan-00-H): neutral MIR → x86-64 machine ops.
//!
//! The x86 counterpart of `arch::aarch64::select`. It consumes the shared
//! neutral [`MirInstruction`] stream (via `mir::Backend::select`) and produces
//! [`CodeInstruction`]s with x86/SysV registers, using the shared MIR primitives
//! (`fused_setter_codeop`, `rename_field_values`, …) — so all the ISA-specific
//! selection lives here, not in shared `mir.rs`.

use crate::arch::ops::CodeOp;
use crate::target::shared::code::mir::{
    code_fields_from_mir, fused_setter_codeop, MirInstruction, MirOp, ARENA_BASE, FUSED_COND_FIELD,
    FUSED_SHARE_FIELD,
};
use crate::target::shared::code::CodeInstruction;
use crate::target::shared::code::{AbiConvention, AbiRole, Operand};

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
    //
    // plan-47-B Phase 1 Win64 audit (finding, no live hazard yet): this pool is
    // convention-independent — Win64 select would reuse it unchanged. The high
    // indices (x19/x20/x27/x28 → rbp/rbx/r12/r13) are Win64 callee-saved too, so
    // "survives an intervening call" still holds. The LATENT hazard is the low
    // remainder: `rcx`/`r8`/`r9`/`rsi`/`rdi` are Win64 *argument* registers (arg
    // 0–3 and the internal-extension 4/5) that were not SysV args 0–2, so a
    // hand-written helper that parks a value in low scratch and then stages call
    // arguments over it would corrupt it — but only under Win64 codegen, which no
    // backend selects yet (47-B lands the ABI tables before a `Win64Backend` is
    // reachable). No helper is corrupted today. This must be re-audited / the pool
    // made abi-aware before the Win64 backend selects end to end (later 47-B / 47-D):
    // recorded here so it is not rediscovered as a silent Windows-only miscompile.
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

/// Which x86-64 calling convention `select_x86` realizes the residual ABI
/// registers against (plan-47-B). `SysV` reads the constants above unchanged —
/// every non-Windows caller passes it, so their bytes do not move. `Win64` reads
/// the `*_WIN64` tables below.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum X86Abi {
    SysV,
    // Constructed by `Win64Backend::select`, which the win_x86_64 CodegenPlatform
    // wires in 47-B A2; until then only the unit tests reach the Win64 arms.
    #[allow(dead_code)]
    Win64,
}

// Win64 (plan-47-B §4.1/§4.2): int args rcx,rdx,r8,r9; then an INTERNAL extension
// for arguments 4–7 (rdi,rsi,rax,rbp) so the compiler's own 8-parameter calls
// keep 8 register homes exactly as SysV does (REGISTER_ARGUMENT_COUNT stays 8).
// External calls are capped at 4 by `Win64RegisterModel::external_int_argument_registers`
// and spill the rest to the stack tail above the 32-byte shadow space. rdi/rsi are
// excluded from the Win64 allocatable pool so the allocator never colors them while
// they carry an argument; rax/rbp are already reserved (result reg / frame pointer),
// exactly as under SysV. Args 7/8 use rax/rbp (not r10) — mirroring SysV — so `r10`
// stays allocatable and the Int pool keeps FOUR registers; a 3-register pool cannot
// even allocate `add_carry` (which needs 4 simultaneously-live), so the plan's
// "accepted 3-register regression" was in fact a hard failure (§Corrections).
const CALL_ARGS_WIN64: &[&str] = &["rcx", "rdx", "r8", "r9", "rdi", "rsi", "rax", "rbp"];

/// The C-ABI return bank (plan-85-A §2): `rax:rdx`, the ≤2 registers the platform
/// C ABI returns in, identical on SysV and Win64. `%retC` keeps `rax` — the one
/// register MFB's aligned convention does *not* claim — so a genuine C boundary is
/// the sole `rax`-bearing site.
const C_RETS: &[&str] = &["rax", "rdx"];

/// Realize a **convention-explicit** ABI token (plan-85-A `Operand::Abi`) to its
/// final **aligned** x86-64 register under `abi`, by a direct table lookup — the
/// whole point of the six-token vocabulary (no CFG role inference). Per §2:
/// `%argMFB`/`%retMFB`/`%argC` all draw from the call-argument bank (on SysV the
/// aligned `[rdi,rsi,rdx,rcx,…]`, so a result register == the argument register),
/// `%retC` from `rax:rdx`, `%argSys`/`%retSys` from the syscall file. This is the
/// map `select_x86` applies directly, bypassing `remap_x86_abi` (deleted in
/// plan-85-D). Panics on an out-of-range index (a construction bug).
fn realize_abi_operand(
    convention: AbiConvention,
    role: AbiRole,
    index: usize,
    abi: X86Abi,
) -> &'static str {
    let (call_args, sys_args): (&[&str], &[&str]) = match abi {
        X86Abi::SysV => (CALL_ARGS, SYS_ARGS),
        // Win64 emits no raw syscall (OS calls go through the IAT), so `SYS_ARGS`
        // is unreachable under Win64; it is passed only to keep the arity uniform.
        X86Abi::Win64 => (CALL_ARGS_WIN64, SYS_ARGS),
    };
    let bank: &[&str] = match (convention, role) {
        // MFB's ALIGNED convention (plan-85-A §2): an MFB argument, an MFB result, and
        // a C-call argument ALL draw from the call-argument bank on BOTH ABIs (SysV
        // `[rdi,rsi,rdx,rcx]`, Win64 `[rcx,rdx,r8,r9]`). Alignment is the whole point:
        // an MFB result reused as an argument is the SAME register (a self-move), so
        // no staging is needed and the CFG fixpoint is deletable. This MUST hold on
        // Win64 too — the shared lowering relies on result==arg register coincidence
        // everywhere, so a non-aligned Win64 result (e.g. `rax`) silently corrupts a
        // result-fed argument (breaks `io` end-to-end). The consequence — a Win64
        // result landing on `rcx` that then feeds a variable shift — is handled in the
        // encoder (`var_shift` shifts in a scratch when dst==rcx), NOT by de-aligning.
        (AbiConvention::Mfb, _) | (AbiConvention::C, AbiRole::Arg) => call_args,
        (AbiConvention::C, AbiRole::Ret) => C_RETS,
        (AbiConvention::Sys, AbiRole::Arg) => match abi {
            X86Abi::SysV => sys_args,
            X86Abi::Win64 => {
                unreachable!("Win64 emits no syscall boundary; OS calls go through the IAT")
            }
        },
        (AbiConvention::Sys, AbiRole::Ret) => match abi {
            X86Abi::SysV => return "rax",
            X86Abi::Win64 => {
                unreachable!("Win64 emits no syscall boundary; OS calls go through the IAT")
            }
        },
    };
    bank.get(index).copied().unwrap_or_else(|| {
        panic!("ABI token index {index} out of range for {convention:?}/{role:?} on x86")
    })
}


/// Map the MECHANICAL residual an already-ABI-realized x86 stream still carries
/// (plan-85-D — the replacement for the deleted `remap_x86_abi` CFG fixpoint). By
/// the time this runs, `select_x86` stage 1 has realized every ABI *role*/convention
/// token directly to its x86 register, so the ONLY `xN` left are leftover physical
/// scratch (`x9`–`x30`, from `realize_abi_token` of `%scratchN`/`%localN`), plus
/// `sp`, the `x31` zero spelling, the `x30`/`lr` link register (dropped — `call`
/// pushes/pops the return address), and the `dN`/`vN`/`qN` float bank. There is no
/// role inference: an `x0`–`x8` here would mean an unrealized ABI token escaped
/// stage 1, which a `debug_assert` flags (release maps it context-free so it still
/// encodes). This is the mechanical tail of the old fixpoint, verbatim.
fn realize_x86_residual(instructions: &mut Vec<CodeInstruction>, abi: X86Abi) {
    // The link register has no x86 equivalent — drop its frame save/restore.
    instructions.retain(|inst| {
        !inst
            .fields
            .iter()
            .any(|(_, value)| value == "x30" || value == "lr")
    });
    for inst in instructions.iter_mut() {
        for (_, value) in inst.fields.iter_mut() {
            let text = value.render();
            let text = text.as_str();
            if text == "sp" {
                *value = Operand::from("rsp");
                continue;
            }
            if text == "x31" {
                *value = Operand::from(crate::target::shared::abi::ZERO);
                continue;
            }
            if let Some(fp) = text
                .strip_prefix(['d', 'v', 'q'])
                .and_then(|rest| rest.parse::<usize>().ok())
                .filter(|n| *n < 16)
            {
                *value = Operand::from(format!("xmm{fp}"));
                continue;
            }
            let Some(n) = text
                .strip_prefix('x')
                .and_then(|rest| rest.parse::<usize>().ok())
                .filter(|n| *n <= 30)
            else {
                continue;
            };
            if n > 8 {
                *value = Operand::from(map_scratch_register(n));
            } else {
                // A residual `x0`–`x8` means an ABI token was not realized in stage 1
                // — a construction bug now that every arg/result/scratch is explicit.
                // Fail loudly in debug; in release, map it context-free to the call
                // bank so it still ENCODES (the same register the old fixpoint gave a
                // stray argument-position `xN`) rather than emitting an invalid `xN`.
                debug_assert!(
                    false,
                    "residual x{n} on x86 after direct ABI-token realization (unrealized token?)"
                );
                let bank = match abi {
                    X86Abi::SysV => CALL_ARGS,
                    X86Abi::Win64 => CALL_ARGS_WIN64,
                };
                *value = Operand::from(bank.get(n).copied().unwrap_or("rax"));
            }
        }
    }
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
pub(crate) fn select_x86(instructions: Vec<MirInstruction>, abi: X86Abi) -> Vec<CodeInstruction> {
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
                fields: code_fields_from_mir(&instruction.fields),
                source: instruction.source,
            });
            continue;
        }
        if let Some(setter_op) = fused_setter_codeop(instruction.op) {
            let split = instruction
                .fields
                .iter()
                .position(|(key, _)| *key == FUSED_COND_FIELD)
                .expect("fused MIR op carries a cond field");
            let setter_fields = code_fields_from_mir(&instruction.fields[..split]);
            let branch_op = CodeOp::from_mnemonic(&instruction.fields[split].1.render())
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
                    source: instruction.source,
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
                    .map(|(_, v)| v.render())
                    .expect("float compare branch carries a target");
                for inst in x86_float_branch(
                    &instruction.fields[split].1.render(),
                    &target,
                    float_branch_site,
                ) {
                    out.push(inst);
                }
                float_branch_site += 1;
            } else {
                out.push(CodeInstruction {
                    op: branch_op,
                    fields: branch_fields,
                    source: instruction.source,
                });
            }
        } else {
            // Non-fused MIR ops map 1:1 to a CodeOp via `to_code` (which applies
            // the neutral→concrete renames, e.g. `call`→`bl`); the x86 encoder
            // realizes each CodeOp as x86 bytes. MOVE the field bag instead of
            // `code_fields_from_mir`'s `to_vec` clone (plan-84 Phase 2).
            let op = instruction
                .op
                .to_code()
                .expect("non-fused MIR op maps to a single CodeOp");
            let source = instruction.source;
            out.push(CodeInstruction {
                op,
                fields: instruction.fields,
                // plan-71-C Phase 0: carry the builder source so the audit names it.
                source,
            });
        }
    }
    for instruction in &mut out {
        crate::target::shared::code::mir::rename_operand_field_values(
            &mut instruction.fields,
            ARENA_BASE,
            "r15",
        );
        // plan-85-D direct-realize seam: every ABI token is realized to its final
        // x86 register HERE, directly — there is no `remap_x86_abi` CFG inference
        // anymore (deleted). A typed `Operand::Abi` (the six-token convention
        // vocabulary) realizes to its ALIGNED register directly; convention tokens
        // are ALWAYS typed now (plan-85-D typed every emission, every parameter/result
        // `location`, and the fused compare-branch expansion — none ever reach here as
        // a `Raw` string). The syscall-number register `%sysnr` realizes to `rax`; the
        // remaining neutral tokens (`%scratchN`, `%localN`, `%mathpool`,
        // `%sysnr_darwin`, …) realize via the shared `realize_abi_token` to their `xN`
        // spelling for the mechanical residual pass below.
        for (_, value) in instruction.fields.iter_mut() {
            if let Operand::Abi {
                convention,
                role,
                index,
            } = value
            {
                let reg = realize_abi_operand(*convention, *role, *index as usize, abi);
                *value = Operand::from(reg);
                continue;
            }
            let rendered = value.render();
            // The syscall-number register lives in `rax` on x86-64 (the one
            // intentionally-kept neutral ABI token with no convention spelling; its
            // positional `x8` would otherwise fall through to the residual pass).
            if rendered == "%sysnr" {
                *value = Operand::from("rax");
                continue;
            }
            if let Some(reg) = crate::target::shared::abi::realize_abi_token(&rendered) {
                *value = Operand::from(reg);
            }
        }
    }
    // plan-85-D: `remap_x86_abi`'s CFG role inference is GONE — stage 1 above realized
    // every ABI token directly (aligned, no inference). Only the mechanical residual
    // (scratch `xN`, `sp`, `x31`, float bank, `x30`/`lr` drop) remains to map.
    realize_x86_residual(&mut out, abi);
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
        select_x86(lower_to_mir(instructions), X86Abi::SysV)
    }

    /// Every field value in the selected stream, flattened.
    fn values(out: &[CodeInstruction]) -> Vec<String> {
        out.iter()
            .flat_map(|inst| inst.fields.iter().map(|(_, v)| v.render()))
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
        // The legacy `x31` zero spelling now maps to the neutral zero token (which
        // the encoder emits as an immediate zero), not the freed r14.
        assert!(vals.contains(&"xzr".to_string()));
        assert!(vals.contains(&"xmm0".to_string()));
        assert!(vals.contains(&"xmm3".to_string()));
        assert!(vals.contains(&"xmm1".to_string()));
        assert!(vals.contains(&"xmm2".to_string()));
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
            .map(|i| i.fields[0].1.render())
            .collect();
        assert_eq!(
            labels.len(),
            3,
            "two skip labels + the shared target: {labels:?}"
        );
        let skips: Vec<String> = labels
            .iter()
            .filter(|n| n.contains("__x86ford"))
            .cloned()
            .collect();
        assert_eq!(skips.len(), 2);
        assert_ne!(skips[0], skips[1], "skip labels collide: {skips:?}");
        // Each `jp` targets its own skip label, which sits right after its `jb`.
        let jps: Vec<String> = out
            .iter()
            .filter(|i| i.op.mnemonic() == "x86.jp")
            .map(|i| i.fields[0].1.render())
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

    #[test]
    fn realize_abi_operand_maps_to_aligned_registers() {
        // plan-85-A §2 aligned SysV realization: MFB args/results and C args all
        // draw from the aligned call-argument bank; %retC keeps rax:rdx; the
        // syscall file is unchanged.
        for (n, reg) in ["rdi", "rsi", "rdx", "rcx", "r8", "r9", "rax", "rbp"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                realize_abi_operand(AbiConvention::Mfb, AbiRole::Arg, n, X86Abi::SysV),
                reg
            );
            assert_eq!(
                realize_abi_operand(AbiConvention::C, AbiRole::Arg, n, X86Abi::SysV),
                reg
            );
        }
        // %retMFB is the byte-CHANGING choice: aligned [rdi,rsi,rdx,rcx], NOT the
        // old rax-first result bank [rax,rdx,rcx,rsi].
        for (n, reg) in ["rdi", "rsi", "rdx", "rcx"].into_iter().enumerate() {
            assert_eq!(
                realize_abi_operand(AbiConvention::Mfb, AbiRole::Ret, n, X86Abi::SysV),
                reg
            );
        }
        // %retC keeps the genuine C return bank rax:rdx.
        assert_eq!(
            realize_abi_operand(AbiConvention::C, AbiRole::Ret, 0, X86Abi::SysV),
            "rax"
        );
        assert_eq!(
            realize_abi_operand(AbiConvention::C, AbiRole::Ret, 1, X86Abi::SysV),
            "rdx"
        );
        // Syscalls.
        for (n, reg) in ["rdi", "rsi", "rdx", "r10", "r8", "r9"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                realize_abi_operand(AbiConvention::Sys, AbiRole::Arg, n, X86Abi::SysV),
                reg
            );
        }
        assert_eq!(
            realize_abi_operand(AbiConvention::Sys, AbiRole::Ret, 0, X86Abi::SysV),
            "rax"
        );
        // Win64: MFB arg AND result share the aligned Win64 call bank
        // (rcx,rdx,r8,r9); a variable-shifted result landing on rcx is handled by
        // the encoder, not by de-aligning. %retC still rax:rdx.
        for (n, reg) in ["rcx", "rdx", "r8", "r9"].into_iter().enumerate() {
            assert_eq!(
                realize_abi_operand(AbiConvention::Mfb, AbiRole::Arg, n, X86Abi::Win64),
                reg
            );
            assert_eq!(
                realize_abi_operand(AbiConvention::Mfb, AbiRole::Ret, n, X86Abi::Win64),
                reg
            );
        }
        assert_eq!(
            realize_abi_operand(AbiConvention::C, AbiRole::Ret, 0, X86Abi::Win64),
            "rax"
        );
        assert_eq!(
            realize_abi_operand(AbiConvention::C, AbiRole::Ret, 1, X86Abi::Win64),
            "rdx"
        );
    }

    #[test]
    fn explicit_abi_token_realizes_to_aligned_register() {
        use crate::target::shared::abi;
        // A typed `Operand::Abi` is realized directly by the plan-85-A seam to its
        // ALIGNED register. Proof: `%retMFB0` realizes to `rdi` (aligned
        // CALL_ARGS[0]); the old rax-first result bank would give `rax`. If `rax`
        // appeared, the aligned realization was not applied.
        let inst = CodeInstruction::new("mov")
            .field("dst", abi::mfb_return(0))
            .field("src", abi::mfb_arg(1));
        let out = sel(&[inst]);
        let vals = values(&out);
        assert!(
            vals.iter().any(|v| v == "rdi"),
            "%retMFB0 must realize aligned to rdi, got {vals:?}"
        );
        assert!(
            vals.iter().any(|v| v == "rsi"),
            "%argMFB1 must realize aligned to rsi, got {vals:?}"
        );
        assert!(
            !vals.iter().any(|v| v == "rax"),
            "no rax expected — the old rax-first result bank must not appear, got {vals:?}"
        );
    }
}
