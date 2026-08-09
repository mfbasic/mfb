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
// x0/x1 are the SysV return registers (rax/rdx). x2/x3 extend the set only for
// the runtime's 4-register error-Result convention (tag=x0, value=x1,
// message=x2, source=x3), which `make_error_result` produces and the error/TRAP
// path consumes immediately (no intervening call), so caller-saved rcx/rsi are
// safe distinct homes. Without these, x2/x3 fell back to rax and aliased,
// corrupting propagated errors.
const RETS: &[&str] = &["rax", "rdx", "rcx", "rsi"];

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
// Win64 result bank: SysV's rcx/rsi (slots 2/3 of the 4-register fallible-result
// convention) collide with Win64 argument register rcx, so use r8/r9 — caller-saved,
// unpinned, and consumed by the error/TRAP path with no intervening call (§4.1).
const RETS_WIN64: &[&str] = &["rax", "rdx", "r8", "r9"];

/// Context-free direct map from an ABI *role token* to its x86-64 register under
/// `abi` — **no CFG role inference** (`%argN` → `CALL_ARGS[N]`, `%sysargN` →
/// `SYS_ARGS[N]`, `%retN` → `RETS[N]`, `%sysnr`/`%sysret` → `rax`); `None` for any
/// non-role value. This is the map bug-85 tried to realize directly and
/// `remap_x86_abi`'s fixpoint replaced. plan-71 drives the fixpoint toward it: at a
/// site where this map and the inference AGREE the fixpoint is deletable
/// byte-identically; where they DISAGREE a later letter must re-tokenize (Category
/// 1) or stage a move (Category 2). The audit in `remap_x86_abi` reports every
/// disagreement so plan-71-A can census the split (plan-71-A §3/§4).
fn map_token_direct(value: &str, abi: X86Abi) -> Option<String> {
    let (call_args, sys_args, rets): (&[&str], &[&str], &[&str]) = match abi {
        X86Abi::SysV => (CALL_ARGS, SYS_ARGS, RETS),
        // Win64 emits no raw syscall (OS calls go through the IAT), so `SYS_ARGS`
        // is unreachable under Win64; it is passed only to keep the arity uniform.
        X86Abi::Win64 => (CALL_ARGS_WIN64, SYS_ARGS, RETS_WIN64),
    };
    let index_after = |prefix: &str| {
        value
            .strip_prefix(prefix)
            .and_then(|rest| rest.parse::<usize>().ok())
    };
    if let Some(n) = index_after("%arg") {
        return call_args.get(n).map(|reg| reg.to_string());
    }
    if let Some(n) = index_after("%sysarg") {
        return sys_args.get(n).map(|reg| reg.to_string());
    }
    if let Some(n) = index_after("%ret") {
        return rets.get(n).map(|reg| reg.to_string());
    }
    if value == "%sysnr" || value == "%sysret" {
        // The syscall number lives in `rax` (SysV) and a syscall's result comes
        // back in `rax`; both agree with the inference's `x8`→rax / `x0`-result
        // coloring at a syscall boundary.
        return Some("rax".to_string());
    }
    None
}

/// The C-ABI return bank (plan-85-A §2): `rax:rdx`, the ≤2 registers the platform
/// C ABI returns in, identical on SysV and Win64 (`RETS`/`RETS_WIN64` both start
/// `rax, rdx`). `%retC` keeps `rax` — the one register MFB's aligned convention
/// does *not* claim — so a genuine C boundary is the sole `rax`-bearing site.
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
        // MFB's aligned convention: an MFB argument (and a C-call argument) draws
        // from the call-argument bank on both ABIs — unchanged from the legacy
        // realization, so args never move on any target.
        (AbiConvention::Mfb, AbiRole::Arg) | (AbiConvention::C, AbiRole::Arg) => call_args,
        // The MFB RESULT alignment is SysV-ONLY: on SysV the result shares the
        // aligned call bank (`[rdi,rsi,rdx,rcx]`, no `rax`), so an MFB result reused
        // as an argument is a self-move — the property that lets the fixpoint go.
        // On Win64 the MFB result uses the `rax`-based bank (`RETS_WIN64`), NOT the
        // aligned `rcx`, for a hard ENCODING reason: `rcx` is the x86 variable-shift
        // COUNT register, so an aligned Win64 result feeding a variable shift is
        // unencodable (the shift guard rejects an `rcx` target). Win64 has no
        // result→argument reuse needing the aligned self-move, so context-free
        // realization stays correct without it. (Windows byte-identity is a
        // non-goal; Win64 correctness is proven by EXECUTION on the Windows box.)
        (AbiConvention::Mfb, AbiRole::Ret) => match abi {
            X86Abi::SysV => CALL_ARGS,
            X86Abi::Win64 => RETS_WIN64,
        },
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

/// Realize a convention-explicit ABI token in its **rendered string** form
/// (`%argMFB{k}`/`%retMFB{k}`/`%argC{k}`/`%retC{k}`/`%argSys{k}`/`%retSys`) directly
/// to its aligned x86-64 register under `abi` — the string counterpart of
/// [`realize_abi_operand`], used once the tokens flow as `Raw` strings (the shared
/// `RET`/`ARG`/`SYSARG` arrays and the fused compare-branch `expand_fused` erasure).
/// This is the whole map after `remap_x86_abi`'s deletion (plan-85-D): no CFG role
/// inference — parse the convention+role+index and table-look-up. `None` for any
/// non-convention string (a physical register, immediate, symbol, label, or vreg).
fn map_convention_token(value: &str, abi: X86Abi) -> Option<String> {
    let (convention, role, index): (AbiConvention, AbiRole, usize) =
        if let Some(rest) = value.strip_prefix("%argMFB") {
            (AbiConvention::Mfb, AbiRole::Arg, rest.parse().ok()?)
        } else if let Some(rest) = value.strip_prefix("%retMFB") {
            (AbiConvention::Mfb, AbiRole::Ret, rest.parse().ok()?)
        } else if let Some(rest) = value.strip_prefix("%argSys") {
            (AbiConvention::Sys, AbiRole::Arg, rest.parse().ok()?)
        } else if let Some(rest) = value.strip_prefix("%argC") {
            (AbiConvention::C, AbiRole::Arg, rest.parse().ok()?)
        } else if let Some(rest) = value.strip_prefix("%retC") {
            (AbiConvention::C, AbiRole::Ret, rest.parse().ok()?)
        } else if value == "%retSys" {
            (AbiConvention::Sys, AbiRole::Ret, 0)
        } else {
            return None;
        };
    Some(realize_abi_operand(convention, role, index, abi).to_string())
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
        // plan-34-B Phase-3b seam: realize a role token to its AArch64 spelling
        // (`%arg3` → `x3`) so `remap_x86_abi`'s existing role inference reproduces
        // today's result exactly (byte-identical). plan-71-A now DEFERS the ABI
        // *role* tokens (`is_abi_role_token`: `%argN`/`%sysargN`/`%retN`/`%sysnr`/
        // `%sysret`) past this seam so `remap_x86_abi` can realize AND cross-check
        // them against `map_token_direct`; every other token (`%scratchN`,
        // `%localN`, `%mathpool`, `%sysnr_darwin`, …) is realized here as before.
        for (_, value) in instruction.fields.iter_mut() {
            // plan-85-A direct-realize seam: a convention-explicit `Operand::Abi`
            // token is realized *here*, directly to its aligned x86 register,
            // bypassing `remap_x86_abi` entirely — the seam plan-85-D widens to
            // "every operand direct, fixpoint gone". Legacy `%arg`/`%ret` role
            // tokens still defer to the fixpoint below.
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
            // plan-85-D: realize every ABI token DIRECTLY here — the convention
            // tokens (`%retMFB`/`%argC`/…) by aligned table lookup, the transitional
            // legacy role tokens (`%arg`/`%ret`/`%sysarg`) context-free — so nothing
            // defers to `remap_x86_abi`'s CFG inference. Under the aligned convention
            // an MFB result and its reuse-as-argument share the call bank, so the
            // context-free map reproduces the (former) inference without a boundary
            // scan. The residual `xN` scratch / `sp` / `x31` / `dN` still flow to the
            // mechanical remap below (which no longer colors any `x0`–`x8` role).
            if let Some(reg) = map_convention_token(&rendered, abi) {
                *value = Operand::from(reg);
                continue;
            }
            if let Some(reg) = map_token_direct(&rendered, abi) {
                *value = Operand::from(reg);
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

    /// The string-form convention-token realizer reproduces the plan-85-A §2 table
    /// exactly — the aligned MFB bank on SysV (`[rdi,rsi,rdx,rcx]` for MFB arg AND
    /// result), `rax:rdx` for the genuine C return, the syscall file for `%argSys`,
    /// and the Win64 homes. This is the whole x86 realization after the fixpoint is
    /// deleted, so its correctness is the deletion's correctness.
    #[test]
    fn map_convention_token_matches_aligned_table() {
        let s = |v: &str| map_convention_token(v, X86Abi::SysV).unwrap();
        let w = |v: &str| map_convention_token(v, X86Abi::Win64).unwrap();
        // SysV: one aligned bank for MFB arg, MFB return, and C-call arg.
        for (tok, reg) in [
            ("%argMFB0", "rdi"),
            ("%argMFB3", "rcx"),
            ("%retMFB0", "rdi"),
            ("%retMFB1", "rsi"),
            ("%retMFB2", "rdx"),
            ("%retMFB3", "rcx"),
            ("%argC0", "rdi"),
            ("%argC1", "rsi"),
            ("%retC0", "rax"),
            ("%retC1", "rdx"),
            ("%argSys0", "rdi"),
            ("%argSys3", "r10"),
            ("%retSys", "rax"),
        ] {
            assert_eq!(s(tok), reg, "SysV {tok}");
        }
        // Win64: MFB args use the Win64 call bank (rcx,rdx,…); the MFB RESULT keeps
        // its legacy rax-based bank (byte-identical), and the C return is rax:rdx.
        for (tok, reg) in [
            ("%argMFB0", "rcx"),
            ("%argMFB1", "rdx"),
            ("%retMFB0", "rax"),
            ("%retMFB1", "rdx"),
            ("%retMFB2", "r8"),
            ("%retMFB3", "r9"),
            ("%argC0", "rcx"),
            ("%retC0", "rax"),
            ("%retC1", "rdx"),
        ] {
            assert_eq!(w(tok), reg, "Win64 {tok}");
        }
        // Non-convention strings are passed through (None).
        assert!(map_convention_token("rax", X86Abi::SysV).is_none());
        assert!(map_convention_token("x5", X86Abi::SysV).is_none());
        assert!(map_convention_token("%v3", X86Abi::SysV).is_none());
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

    // ---- plan-71-A: the context-free cross-check gate ----

    #[test]
    fn map_token_direct_matches_the_abi_tables() {
        // SysV: call args, syscall args, returns, and the syscall nr/result.
        for (n, reg) in ["rdi", "rsi", "rdx", "rcx", "r8", "r9", "rax", "rbp"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                map_token_direct(&format!("%arg{n}"), X86Abi::SysV).as_deref(),
                Some(reg)
            );
        }
        for (n, reg) in ["rdi", "rsi", "rdx", "r10", "r8", "r9"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                map_token_direct(&format!("%sysarg{n}"), X86Abi::SysV).as_deref(),
                Some(reg)
            );
        }
        for (n, reg) in ["rax", "rdx", "rcx", "rsi"].into_iter().enumerate() {
            assert_eq!(
                map_token_direct(&format!("%ret{n}"), X86Abi::SysV).as_deref(),
                Some(reg)
            );
        }
        assert_eq!(
            map_token_direct("%sysnr", X86Abi::SysV).as_deref(),
            Some("rax")
        );
        assert_eq!(
            map_token_direct("%sysret", X86Abi::SysV).as_deref(),
            Some("rax")
        );
        // Win64 reads the *_WIN64 tables.
        for (n, reg) in ["rcx", "rdx", "r8", "r9"].into_iter().enumerate() {
            assert_eq!(
                map_token_direct(&format!("%arg{n}"), X86Abi::Win64).as_deref(),
                Some(reg)
            );
        }
        assert_eq!(
            map_token_direct("%ret0", X86Abi::Win64).as_deref(),
            Some("rax")
        );
        assert_eq!(
            map_token_direct("%ret2", X86Abi::Win64).as_deref(),
            Some("r8")
        );
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
        // legacy RETS [rax,rdx,rcx,rsi].
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
        // Win64: MFB args use the Win64 call bank (rcx,rdx,r8,r9); the MFB RESULT
        // keeps its legacy rax-based bank (RETS_WIN64 = rax,rdx,r8,r9) so Win64
        // stays byte-identical. %retC still rax:rdx.
        for (n, arg_reg) in ["rcx", "rdx", "r8", "r9"].into_iter().enumerate() {
            assert_eq!(
                realize_abi_operand(AbiConvention::Mfb, AbiRole::Arg, n, X86Abi::Win64),
                arg_reg
            );
        }
        for (n, ret_reg) in ["rax", "rdx", "r8", "r9"].into_iter().enumerate() {
            assert_eq!(
                realize_abi_operand(AbiConvention::Mfb, AbiRole::Ret, n, X86Abi::Win64),
                ret_reg
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
    fn explicit_abi_token_bypasses_the_fixpoint() {
        use crate::target::shared::abi;
        // A typed `Operand::Abi` is realized directly by the plan-85-A seam to its
        // ALIGNED register, NOT by `remap_x86_abi`. Proof: `%retMFB0` realizes to
        // `rdi` (aligned CALL_ARGS[0]); the fixpoint / legacy `%ret0` would give
        // `rax` (RETS[0]). If `rax` appeared, the token flowed through the fixpoint.
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
            "no rax expected — the fixpoint's RETS[0] must not appear, got {vals:?}"
        );
    }
}
