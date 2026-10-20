//! plan-176-O: the x86-64 encoder's instruction encoder: `encode_instruction`, the mnemonic
//! dispatch. `Encoder`, the REX/ModRM/SIB helpers and the `InstructionEncoder` impl live in
//! `encode/emitter/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// Per-instruction encoding (single source of truth for size + bytes)
// ---------------------------------------------------------------------------

/// The dispatch key is [`CodeOp::mnemonic`], which returns the **AArch64**
/// spelling for every op. The neutral MIR names (`i2f`, `f2i_trunc`, `rotr_w`,
/// `fmov_i2f`, …) exist only above `MirOp::to_code()`, which renames back to
/// `CodeOp` before selection, so they cannot reach here. This match used to
/// carry eight of them as extra alternatives; they were unreachable, and the
/// aarch64 and riscv64 emitters (which dispatch the same way) correctly carry
/// none (bug-326-A28).
pub(in crate::arch::x86_64::encode) fn encode_instruction(
    instruction: &CodeInstruction,
) -> Result<Encoded, String> {
    let m = instruction.op.mnemonic();
    match m {
        "label" => Ok(Encoded::plain(Vec::new())),
        "mov" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            // `mov dst, xzr` means "materialize 0" (AArch64's hardwired zero
            // register); x86 has none, and `enc_mov` would encode the sentinel as
            // `r8`, moving the caller's leftover r8 instead. Reached in practice by
            // the Win64 audio backend's NULL arguments
            // (`gen_windows_open`/`gen_windows_io` pass `abi::ZERO` for WASAPI
            // out-params), which would otherwise receive garbage rather than NULL.
            // Same 3-byte length as `enc_mov`, so encoded sizes are unchanged.
            if is_zero_token(src) {
                return Ok(Encoded::plain(alu_rr(0x31, dst, dst)));
            }
            Ok(Encoded::plain(enc_mov(dst, src)))
        }
        "mov_imm" => {
            let dst = reg(field(instruction, "dst")?)?;
            let value = immediate(field(instruction, "value")?)?;
            Ok(Encoded::plain(enc_mov_imm64(dst, value)))
        }
        // x86 add/sub always set EFLAGS, so the flag-setting `adds`/`subs`
        // variants are the same encoding (the following flag-branch reads them).
        "add" | "adds" => alu3(instruction, 0x01),
        "sub" | "subs" => alu3(instruction, 0x29),
        "and" => alu3(instruction, 0x21),
        "orr" => alu3(instruction, 0x09),
        "eor" => alu3(instruction, 0x31),
        "mvn" => {
            // dst = ~src : mov dst,src ; not dst
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            // `mvn dst, xzr` is the portable "materialize -1" idiom: AArch64 has a
            // hardwired zero register, x86 does not. `reg` maps the zero token to
            // sentinel 16, which `enc_mov` would encode as `r8` (REX.B + reg 0) —
            // so this produced `~r8`, i.e. whatever the caller last left in r8,
            // instead of -1. Exactly bug-154's shape (see the add_carry arm below,
            // which already guards it), and it is what made `process::waitFor`
            // return -3 for a signal-killed child on x86-64 where the documented
            // contract is -1: r8 happened to hold 2, and ~2 == -3.
            //
            // `xor dst,dst` is 3 bytes, the same length `enc_mov` emits, so the
            // instruction's encoded size is unchanged and the size-prediction pass
            // needs no corresponding update.
            let mut bytes = if is_zero_token(src) {
                alu_rr(0x31, dst, dst)
            } else {
                enc_mov(dst, src)
            };
            // F7 /2 = NOT r/m64
            bytes.push(rex(true, false, false, dst >= 8));
            bytes.push(0xF7);
            bytes.push(modrm(0b11, 2, dst));
            Ok(Encoded::plain(bytes))
        }
        // clz: count leading zeros. `lzcnt r64, r/m64` (F3 REX.W 0F BD /r). Matches
        // AArch64 `clz` (64 for a zero input), unlike `bsr` which is undefined on
        // zero. bug-178 C: LZCNT is an ABM/BMI feature (effectively x86-64-v3), NOT
        // baseline v2 — on a pre-v3 CPU the F3 prefix is ignored and the bytes
        // execute as `bsr` (silently wrong on zero). This is acceptable because the
        // backend already requires ~v3 unconditionally (it emits FMA3 and SSE4.1
        // `roundsd` with no feature gate); the effective baseline is v3, not v2.
        "clz" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(vec![
                0xF3,
                rex(true, dst >= 8, false, src >= 8),
                0x0F,
                0xBD,
                modrm(0b11, dst, src),
            ]))
        }
        // sxtw: sign-extend low 32 bits into 64. `movsxd r64, r/m32`
        // (REX.W 0x63 /r). AArch64 emits this to narrow a C `int` return before a
        // signed compare (bug-04); on x86 the same abstract op keeps the seam
        // encodable, though the x86 `cmp_imm` is already 32-bit.
        "sxtw" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(vec![
                rex(true, dst >= 8, false, src >= 8),
                0x63,
                modrm(0b11, dst, src),
            ]))
        }
        // rev_w / rev_x: byte-reverse a 32/64-bit value (`bswap`, 0F C8+rd; the
        // 64-bit form takes REX.W). AArch64 `rev` has dst/src, x86 `bswap` is
        // in-place, so copy first when they differ.
        "rev_w" | "rev_x" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let wide = m == "rev_x";
            let mut bytes = if dst == src {
                Vec::new()
            } else if wide {
                enc_mov(dst, src)
            } else {
                // 32-bit mov (zero-extends): 89 /r, REX only for r8–r15.
                let mut b = Vec::new();
                if dst >= 8 || src >= 8 {
                    b.push(rex(false, src >= 8, false, dst >= 8));
                }
                b.push(0x89);
                b.push(modrm(0b11, src, dst));
                b
            };
            if wide || dst >= 8 {
                bytes.push(rex(wide, false, false, dst >= 8));
            }
            bytes.push(0x0F);
            bytes.push(0xC8 + (dst & 7));
            Ok(Encoded::plain(bytes))
        }
        // rbit: reverse all 64 bits. x86 has no single instruction, so use the
        // classic swap-by-strides (1,2,4 bits) then a byte reverse (`bswap`). rax
        // (mask) and rdx (temp) are used as scratch. They are excluded from the
        // allocatable pool, so no body vreg lands there, but ABI-staging and the
        // residual scratch mapping (map_scratch_register uses rax/rdx) can leave a
        // live caller value in either across this multi-instruction expansion. The
        // allocator's clobber model does not cover that, so preserve rax/rdx with
        // an explicit push/pop. (dst is never rax/rdx — the algorithm overwrites
        // both — so the pop cannot clobber the result.)
        "rbit" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            // bug-284 C6: the bit-swap sequence uses rax as the mask register and
            // rdx as the scratch accumulator, both pushed and popped around it. A
            // `dst` coloured onto either is corrupted by that use and then restored
            // to its pre-expansion value by the pop, discarding the result.
            if dst == 0 || dst == 2 {
                return Err(
                    "x86-64 rbit cannot target rax or rdx: both are used as the \
                     mask and accumulator of the bit-swap sequence"
                        .to_string(),
                );
            }
            let mut b = Vec::new();
            b.push(0x50); // push rax
            b.push(0x52); // push rdx
            if dst != src {
                b.extend_from_slice(&enc_mov(dst, src));
            }
            // or dst, rdx : REX.W 09 /r (rm=dst, reg=rdx=2)
            let or_dst_rdx = |b: &mut Vec<u8>| {
                b.push(rex(true, false, false, dst >= 8));
                b.push(0x09);
                b.push(modrm(0b11, 2, dst));
            };
            // and reg, rax : REX.W 21 /r (rm=reg, reg=rax=0)
            let and_with_rax = |b: &mut Vec<u8>, r: u8| {
                b.push(rex(true, false, false, r >= 8));
                b.push(0x21);
                b.push(modrm(0b11, 0, r));
            };
            // mov rdx, dst : REX.W 89 /r (rm=rdx, reg=dst)
            let mov_rdx_dst = |b: &mut Vec<u8>| {
                b.push(rex(true, dst >= 8, false, false));
                b.push(0x89);
                b.push(modrm(0b11, dst, 2));
            };
            // Each step swaps adjacent bit-groups of width `shift`:
            //   x = ((x >> s) & mask) | ((x & mask) << s)
            for &(shift, mask) in &[
                (1u8, 0x5555_5555_5555_5555u64),
                (2, 0x3333_3333_3333_3333),
                (4, 0x0F0F_0F0F_0F0F_0F0F),
            ] {
                mov_rdx_dst(&mut b); // rdx = x
                b.extend(enc_mov_imm64(0, mask)); // rax = mask
                and_with_rax(&mut b, 2); // rdx = x & mask
                b.extend(enc_shift_imm_reg(4, 2, shift)); // rdx = (x & mask) << s
                b.extend(enc_shift_imm_reg(5, dst, shift)); // dst = x >> s
                and_with_rax(&mut b, dst); // dst = (x >> s) & mask
                or_dst_rdx(&mut b); // dst = ((x>>s)&mask) | ((x&mask)<<s)
            }
            // Final byte reverse: bswap dst (64-bit).
            b.push(rex(true, false, false, dst >= 8));
            b.push(0x0F);
            b.push(0xC8 + (dst & 7));
            b.push(0x5A); // pop rdx
            b.push(0x58); // pop rax
            Ok(Encoded::plain(b))
        }
        "mul" => {
            // dst = lhs * rhs (low 64). imul is commutative, so multiply into
            // whichever source already occupies dst. The naive `mov dst,lhs;
            // imul dst,rhs` corrupts the result when `dst == rhs` (the mov
            // clobbers rhs, giving lhs*lhs) — exactly what broke `index *
            // entry_size` in collection element addressing.
            let dst = reg(field(instruction, "dst")?)?;
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            let bytes = if dst == lhs {
                enc_imul_rr(dst, rhs)
            } else if dst == rhs {
                enc_imul_rr(dst, lhs)
            } else {
                let mut b = enc_mov(dst, lhs);
                b.extend_from_slice(&enc_imul_rr(dst, rhs));
                b
            };
            Ok(Encoded::plain(bytes))
        }
        "umulh" | "smulh" => {
            // rdx:rax = lhs * rhs ; dst = rdx (the high 64 bits).
            // F7 /4 = MUL (unsigned high), F7 /5 = IMUL (signed high). MUL/IMUL
            // implicitly read rax and write rdx:rax. Those registers are not in the
            // allocatable pool, but ABI-staging and the residual scratch mapping
            // (map_scratch_register uses rax/rdx) can leave a live caller value in
            // either across this multi-instruction expansion, and rhs may itself be
            // mapped onto rax — where the `mov rax,lhs` setup would destroy it
            // before the multiply reads it. Mirror div_seq: preserve rax/rdx
            // (except whichever is the destination, since that carries the result),
            // and when rhs aliases rax stage it in a stack slot and multiply from
            // memory.
            let dst = reg(field(instruction, "dst")?)?;
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            let ext = if instruction.op == crate::arch::ops::CodeOp::SMulH {
                5
            } else {
                4
            };
            let save_rax = dst != 0; // rax = low product / scratch; skip if dst==rax
            let save_rdx = dst != 2; // rdx = high product (result source); skip if dst==rdx
            let stage_rhs = rhs == 0; // rhs on rax → `mov rax,lhs` would clobber it
            let mut bytes = Vec::new();
            if save_rax {
                bytes.push(0x50); // push rax
            }
            if save_rdx {
                bytes.push(0x52); // push rdx
            }
            if stage_rhs {
                // sub rsp,8 ; mov [rsp],rhs — capture rhs (in rax) before it is
                // overwritten by `mov rax,lhs`.
                bytes.extend_from_slice(&enc_alu_imm32(5, 4, 8)); // sub rsp, 8
                bytes.push(rex(true, rhs >= 8, false, false)); // mov [rsp], rhs
                bytes.push(0x89);
                bytes.push(modrm(0b00, rhs, 4));
                bytes.push(0x24); // SIB: base=rsp
            }
            bytes.extend_from_slice(&enc_mov(0, lhs)); // mov rax, lhs
            if stage_rhs {
                // mul/imul qword [rsp]
                bytes.push(rex(true, false, false, false));
                bytes.push(0xF7);
                bytes.push(modrm(0b00, ext, 4));
                bytes.push(0x24); // SIB: base=rsp
                bytes.extend_from_slice(&enc_alu_imm32(0, 4, 8)); // add rsp, 8 (free staged rhs)
            } else {
                // mul/imul rhs (register — survives the rax push and `mov rax,lhs`)
                bytes.push(rex(true, false, false, rhs >= 8));
                bytes.push(0xF7);
                bytes.push(modrm(0b11, ext, rhs));
            }
            bytes.extend_from_slice(&enc_mov(dst, 2)); // dst = rdx (capture before restore)
            if save_rdx {
                bytes.push(0x5A); // pop rdx
            }
            if save_rax {
                bytes.push(0x58); // pop rax
            }
            Ok(Encoded::plain(bytes))
        }
        "udiv" => div_seq(instruction, false),
        "sdiv" => div_seq(instruction, true),
        "msub" => {
            // dst = minuend - lhs*rhs. The product goes through rax (two-operand
            // imul needs its lhs in a register), which normally is fine because rax
            // is non-allocatable. BUT the minuend can itself be rax — the rng draw
            // in math::rand's `raw mod range` uses the return register (x0→rax) as
            // the minuend. The old `mov rax,lhs; …; mov dst,minuend` order then read
            // the *clobbered* rax as the minuend, so the remainder came out 0 and
            // every rand returned its min. When the minuend is rax, capture it into
            // dst first (dst is distinct from lhs/rhs there, so this is safe); keep
            // the product-first order otherwise (safe when dst aliases lhs/rhs).
            let dst = reg(field(instruction, "dst")?)?;
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            let minuend = reg(field(instruction, "minuend")?)?;
            // bug-284 C6: this expansion stages the product in rax. A `dst` on rax
            // is overwritten by `mov dst, minuend` and then subtracted from itself,
            // yielding 0; an `rhs` on rax is destroyed by `mov rax, lhs` before the
            // multiply, yielding lhs*lhs. Neither has a correct encoding here.
            if dst == 0 {
                return Err("x86-64 msub cannot target rax: rax stages the product".to_string());
            }
            if rhs == 0 {
                return Err(
                    "x86-64 msub cannot take rhs in rax: rax is overwritten by the \
                     multiplicand before the multiply"
                        .to_string(),
                );
            }
            let mut bytes = Vec::new();
            if minuend == 0 {
                bytes.extend_from_slice(&enc_mov(dst, minuend)); // dst = minuend (rax)
                bytes.extend_from_slice(&enc_mov(0, lhs)); // rax = lhs
                bytes.extend_from_slice(&enc_imul_rr(0, rhs)); // rax = lhs*rhs
                bytes.extend_from_slice(&alu_rr(0x29, dst, 0)); // dst -= rax
            } else {
                bytes.extend_from_slice(&enc_mov(0, lhs)); // rax = lhs
                bytes.extend_from_slice(&enc_imul_rr(0, rhs)); // rax = lhs*rhs
                bytes.extend_from_slice(&enc_mov(dst, minuend)); // dst = minuend
                bytes.extend_from_slice(&alu_rr(0x29, dst, 0)); // dst -= rax
            }
            Ok(Encoded::plain(bytes))
        }
        "add_carry" => enc_add_carry(instruction),
        "sub_borrow" => enc_sub_borrow(instruction),
        "rorv" => var_shift(instruction, 1), // /1 = ROR
        "lslv" => var_shift(instruction, 4), // /4 = SHL
        "lsrv" => var_shift(instruction, 5), // /5 = SHR
        "asrv" => var_shift(instruction, 7), // /7 = SAR
        "lsl_imm" => shift_imm(instruction, 4),
        "lsr_imm" => shift_imm(instruction, 5),
        "asr_imm" => shift_imm(instruction, 7),
        "add_imm" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            // mov dst,src (if different) ; add dst, imm32
            let mut bytes = if dst == src {
                Vec::new()
            } else {
                enc_mov(dst, src)
            };
            bytes.extend_from_slice(&enc_alu_imm32(0, dst, imm)); // /0 = ADD
            Ok(Encoded::plain(bytes))
        }
        "sub_imm" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            let mut bytes = if dst == src {
                Vec::new()
            } else {
                enc_mov(dst, src)
            };
            bytes.extend_from_slice(&enc_alu_imm32(5, dst, imm)); // /5 = SUB
            Ok(Encoded::plain(bytes))
        }
        "add_sp" => {
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            Ok(Encoded::plain(enc_alu_imm32(0, 4, imm))) // add rsp, imm32
        }
        "sub_sp" => {
            let imm = checked_imm32(immediate(field(instruction, "imm")?)?)?;
            Ok(Encoded::plain(enc_alu_imm32(5, 4, imm))) // sub rsp, imm32
        }
        "cmp" => {
            let lhs = reg(field(instruction, "lhs")?)?;
            let rhs = reg(field(instruction, "rhs")?)?;
            // bug-397: a zero-token operand must never reach `alu_rr` — sentinel 16
            // encodes as r8 (`16 & 7 == 0` + REX), folding a caller's leftover r8
            // into the compare (same r8-collision family as bug-123/154). A zero
            // `rhs` is the real "cmp lhs, 0": route it through the immediate form.
            if is_zero_token(rhs) {
                return Ok(Encoded::plain(enc_alu_imm32(7, lhs, 0))); // cmp lhs, 0
            }
            // A zero `lhs` ("cmp 0, rhs", flags of `0 - rhs`) has no scratch-free
            // x86 form (no zero register); no producer emits it, so reject loudly
            // rather than silently encode r8.
            if is_zero_token(lhs) {
                return Err("x86-64 cmp with a zero-token lhs is not handled".to_string());
            }
            // cmp lhs, rhs : 0x39 /r (rm=lhs, reg=rhs)
            Ok(Encoded::plain(alu_rr(0x39, lhs, rhs)))
        }
        "cmp_imm" => {
            let lhs = reg(field(instruction, "lhs")?)?;
            let imm = checked_imm32(immediate(field(instruction, "rhs")?)?)?;
            // bug-397: a zero-token `lhs` would encode via enc_alu_imm32's rm=16 as
            // r8. "cmp 0, imm" is a degenerate, unproduced compare — reject loudly.
            if is_zero_token(lhs) {
                return Err("x86-64 cmp_imm with a zero-token lhs is not handled".to_string());
            }
            Ok(Encoded::plain(enc_alu_imm32(7, lhs, imm))) // /7 = CMP
        }
        "ldr_u64" => mem_load(instruction, MemWidth::U64),
        "ldr_u32" => mem_load(instruction, MemWidth::U32),
        "ldr_u16" => mem_load(instruction, MemWidth::U16),
        "ldr_u8" => mem_load(instruction, MemWidth::U8),
        "str_u64" => mem_store(instruction, MemWidth::U64),
        "str_u32" => mem_store(instruction, MemWidth::U32),
        // plan-50-D. `MemWidth::U16` already drives the 0x66 operand-size prefix
        // on the load side (`ldr_u16`), so the store needs only this arm.
        "str_u16" => mem_store(instruction, MemWidth::U16),
        "str_u8" => mem_store(instruction, MemWidth::U8),
        "b" => jmp_label(instruction, JccKind::Jmp),
        "b.eq" => jmp_label(instruction, JccKind::Je),
        "b.ne" => jmp_label(instruction, JccKind::Jne),
        "b.ge" => jmp_label(instruction, JccKind::Jge),
        "b.lt" => jmp_label(instruction, JccKind::Jl),
        "b.gt" => jmp_label(instruction, JccKind::Jg),
        "b.le" => jmp_label(instruction, JccKind::Jle),
        "b.hi" => jmp_label(instruction, JccKind::Ja),
        "b.lo" => jmp_label(instruction, JccKind::Jb),
        // Overflow / sign / unsigned-LE — the *_ovf checks and the IEEE float
        // flag-branches (plan-00-B/16/17). V→OF, N→SF, "C clear or Z" → BE.
        "b.vs" => jmp_label(instruction, JccKind::Jo),
        "b.vc" => jmp_label(instruction, JccKind::Jno),
        "b.mi" => jmp_label(instruction, JccKind::Js),
        "b.ls" => jmp_label(instruction, JccKind::Jbe),
        // x86-only float-compare branches (plan-00-H): `select_x86` rewrites the
        // branch after a float `ucomisd` into these (CF/ZF/PF semantics).
        "x86.jae" => jmp_label(instruction, JccKind::Jae),
        "x86.jp" => jmp_label(instruction, JccKind::Jp),
        "x86.jnp" => jmp_label(instruction, JccKind::Jnp),
        "x86.ja" => jmp_label(instruction, JccKind::Ja),
        "x86.jb" => jmp_label(instruction, JccKind::Jb),
        "x86.jbe" => jmp_label(instruction, JccKind::Jbe),
        "x86.je" => jmp_label(instruction, JccKind::Je),
        "x86.jne" => jmp_label(instruction, JccKind::Jne),
        "bl" => {
            let target = field(instruction, "target")?.into_owned();
            // For an external (libc) call: `mov eax, 8` (al = 8) then `E8 rel32`
            // (call). The SysV variadic ABI requires al = number of vector
            // registers used for the variadic args before calling a variadic
            // function (snprintf with a `%f`); 8 is a safe superset (the callee
            // saves xmm0-7). For an INTERNAL `_mfb_*` call the marker must NOT
            // be emitted: internal functions can take 7 arguments, and the 7th
            // is passed in rax (`CALL_ARGS[6]` in select) — `mov eax, 8` would
            // destroy it right before the call (the regex engine's 7-parameter
            // parse functions read a clobbered list pointer). No internal
            // function is variadic, so al is meaningless there.
            let internal = target.starts_with("_mfb_");
            let (bytes, disp_field_offset) = if internal {
                (vec![0xE8, 0, 0, 0, 0], 1)
            } else {
                (vec![0xB8, 8, 0, 0, 0, 0xE8, 0, 0, 0, 0], 6)
            };
            Ok(Encoded {
                bytes,
                side_effect: SideEffect::Reloc {
                    disp_field_offset,
                    target,
                    intent: RelocIntent::Call,
                },
            })
        }
        "blr" => {
            // call r/m64 : FF /2
            let r = reg(field(instruction, "register")?)?;
            let mut bytes = Vec::new();
            if r >= 8 {
                bytes.push(rex(false, false, false, true));
            }
            bytes.push(0xFF);
            bytes.push(modrm(0b11, 2, r));
            Ok(Encoded::plain(bytes))
        }
        "branch_self" => Ok(Encoded::plain(vec![0xEB, 0xFE])), // jmp $ (rel8 -2)
        "ret" => Ok(Encoded::plain(vec![0xC3])),
        "svc" => Ok(Encoded::plain(vec![0x0F, 0x05])), // syscall
        "adrp" => {
            // lea dst, [rip+disp32] ; disp32 patched by a data/GOT relocation.
            let dst = reg(field(instruction, "dst")?)?;
            let symbol = field(instruction, "symbol")?.into_owned();
            // REX.W + 0x8D /r, ModRM rip-relative. disp32 starts after REX(1)+
            // opcode(1)+modrm(1) = 3 bytes in.
            let mut bytes = vec![rex(true, dst >= 8, false, false), 0x8D];
            bytes.extend_from_slice(&mem_rip(dst));
            Ok(Encoded {
                bytes,
                side_effect: SideEffect::Reloc {
                    disp_field_offset: 3,
                    target: symbol,
                    // select_x86 collapses the page pair: the single reference is
                    // the *Lo intent. record_reloc re-routes to GOT for imports.
                    intent: RelocIntent::DataAddrLo,
                },
            })
        }
        // The low half of the AArch64 page pair: x86 already loaded the full
        // address into `dst` via the `adrp`-spelled lea, so this emits nothing.
        "add_pageoff" => Ok(Encoded::plain(Vec::new())),

        // --- Scalar double (SSE2) — the AArch64 `dN` bank maps to `xmm` --------
        // movq xmm, r64 — reinterpret i64 bits as f64 (66 REX.W 0F 6E /r).
        "fmov_d_from_x" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_movq_xmm_r64(dst, src)))
        }
        // movq r64, xmm — reinterpret f64 bits as i64 (66 REX.W 0F 7E /r, MR).
        "fmov_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_movq_r64_xmm(dst, src)))
        }
        // movaps xmm, xmm — register copy (0F 28 /r).
        "fmov_d_from_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(None, 0x28, dst, src)))
        }
        // addsd / subsd / mulsd / divsd — dst = lhs OP rhs (2-operand SSE).
        "fadd_d" => sse_arith(instruction, 0x58, true),
        "fmul_d" => sse_arith(instruction, 0x59, true),
        "fsub_d" => sse_arith(instruction, 0x5c, false),
        "fdiv_d" => sse_arith(instruction, 0x5e, false),
        // minsd / maxsd — dst = min/max(lhs, rhs). Treated as non-commutative so
        // `lhs` stays the destination-accumulator and `rhs` the source, matching
        // AArch64 `fminnm`/`fmaxnm` and RISC-V `fmin.d`/`fmax.d` for the finite,
        // distinct operands MFBASIC produces. (minsd/maxsd differ from fminnm on a
        // NaN input or a ±0 tie — neither is a value MFBASIC exposes through
        // `math::min`/`max`; see 15_x86_64-instruction-set.md.)
        "fminnm_d" => sse_arith(instruction, 0x5d, false),
        "fmaxnm_d" => sse_arith(instruction, 0x5f, false),
        // sqrtsd dst, src (F2 0F 51 /r).
        "fsqrt_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0xf2), 0x51, dst, src)))
        }
        // cvtsd2ss dst, src (F2 0F 5A /r) — round to binary32 under MXCSR (RNE).
        "fcvt_s_from_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0xf2), 0x5a, dst, src)))
        }
        // cvtss2sd dst, src (F3 0F 5A /r) — exact widening.
        "fcvt_d_from_s" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0xf3), 0x5a, dst, src)))
        }
        // ucomisd lhs, rhs — ordered compare into EFLAGS (66 0F 2E /r).
        "fcmp_d" => {
            let lhs = fp_reg(field(instruction, "lhs")?)?;
            let rhs = fp_reg(field(instruction, "rhs")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0x66), 0x2e, lhs, rhs)))
        }
        // fcmp against 0.0: zero the scratch xmm15 then ucomisd src, xmm15.
        "fcmp_zero_d" => {
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(None, 0x57, 15, 15); // xorps xmm15, xmm15
            b.extend(enc_sse_rr(Some(0x66), 0x2e, src, 15)); // ucomisd src, xmm15
            Ok(Encoded::plain(b))
        }
        // -x: flip the sign bit. Build the 0x8000…0000 mask in xmm15 (all-ones
        // via pcmpeqd, then psllq 63) and xorpd it in — no memory mask.
        "fneg_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = Vec::new();
            if dst != src {
                b.extend(enc_sse_rr(Some(0xf2), 0x10, dst, src)); // movsd dst, src
            }
            b.extend(enc_sse_rr(Some(0x66), 0x76, 15, 15)); // pcmpeqd xmm15, xmm15
            b.extend(enc_psxlq(0x06, 15, 63)); // psllq xmm15, 63 -> sign mask
            b.extend(enc_sse_rr(Some(0x66), 0x57, dst, 15)); // xorpd dst, xmm15
            Ok(Encoded::plain(b))
        }
        // |x|: clear the sign bit by shifting the 64-bit lane left 1 then right 1
        // (psllq/psrlq imm), avoiding a memory-resident mask constant.
        "fabs_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = Vec::new();
            if dst != src {
                b.extend(enc_sse_rr(Some(0xf2), 0x10, dst, src)); // movsd dst, src
            }
            b.extend(enc_psxlq(0x06, dst, 1)); // psllq dst, 1
            b.extend(enc_psxlq(0x02, dst, 1)); // psrlq dst, 1
            Ok(Encoded::plain(b))
        }
        // Scalar fused multiply-add family (FMA3, x86-64-v3), single-rounded. The
        // op is staged in the reserved xmm15 so `dst` need not alias a source: the
        // 231-form computes `xmm15 = ±(lhs*rhs) ± xmm15`, seeded with `addend`.
        // The neutral MIR mnemonic names the result; the x86 231-form that computes
        // it can carry a different name (fnmsub_d→vfnmadd231sd, fnmadd_d→
        // vfnmsub231sd), pinned by the byte tests:
        //   fmadd_d  → vfmadd231sd  (B9)  lhs*rhs + addend
        //   fmsub_d  → vfmsub231sd  (BB)  lhs*rhs - addend
        //   fnmsub_d → vfnmadd231sd (BD)  addend - lhs*rhs
        //   fnmadd_d → vfnmsub231sd (BF)  -(lhs*rhs) - addend
        "fmadd_d" | "fmsub_d" | "fnmsub_d" | "fnmadd_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let addend = fp_reg(field(instruction, "addend")?)?;
            let lhs = fp_reg(field(instruction, "lhs")?)?;
            let rhs = fp_reg(field(instruction, "rhs")?)?;
            let opcode = match m {
                "fmadd_d" => 0xB9,
                "fmsub_d" => 0xBB,
                "fnmsub_d" => 0xBD,
                _ => 0xBF,
            };
            let mut b = enc_sse_rr(Some(0xf2), 0x10, 15, addend); // movsd xmm15, addend
            b.extend(enc_vfma231sd(opcode, 15, lhs, rhs)); // xmm15 = ±(lhs*rhs) ± xmm15
            b.extend(enc_sse_rr(Some(0xf2), 0x10, dst, 15)); // movsd dst, xmm15
            Ok(Encoded::plain(b))
        }
        // cvtsi2sd xmm, r64 — signed i64 → f64 (F2 REX.W 0F 2A /r).
        "scvtf_d_from_x" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_cvt(0xf2, 0x2a, true, dst, src)))
        }
        // cvttsd2si r64, xmm — f64 → i64 toward zero (F2 REX.W 0F 2C /r).
        "fcvtzs_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_cvt(0xf2, 0x2c, false, dst, src)))
        }
        // f64 → i64 with a directed rounding mode: `roundsd xmm15, src, mode`
        // (SSE4.1, mode 1=−∞ floor / 2=+∞ ceil) then truncating `cvttsd2si`.
        "fcvtms_x_from_d" | "fcvtps_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mode = if m.starts_with("fcvtms") { 1 } else { 2 };
            let mut b = enc_roundsd(15, src, mode); // roundsd xmm15, src, mode
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, dst, 15)); // cvttsd2si dst, xmm15
            Ok(Encoded::plain(b))
        }
        // f64 → i64 round-to-nearest, ties AWAY from zero (AArch64 `fcvtas`).
        // SSE `roundsd`/`cvtsd2si` round ties to EVEN, so the ties-away rule has
        // to be realized directly.
        //
        // bug-295: the previous realization was `trunc(src + copysign(0.5, src))`,
        // and that addition itself rounds. For `x = 0.49999999999999994`
        // (0.5 − 2⁻⁵⁴) the sum `x + 0.5` is `1 − 2⁻⁵⁴`, exactly halfway between
        // the predecessor of 1.0 and 1.0; round-to-nearest-even picks 1.0 and the
        // truncation yields 1. AArch64 `fcvtas` and riscv RMM both yield 0 — the
        // input is strictly below one half, not a tie — so linux-x86_64 alone
        // returned a wrong, divergent answer.
        //
        // Instead compute the fraction exactly and let a second truncation decide:
        //
        //     t = trunc(x)                  exact
        //     f = t − x                     exact (x − trunc(x) is representable)
        //     d = trunc(2f)                 ∈ {−1, 0, +1}
        //     result = t − d
        //
        // No step rounds, so the just-below-half family can never be nudged onto a
        // tie. `2f` is exact (a doubling, and |2f| < 2), and at a genuine tie
        // |2f| is exactly 1, which is what carries the away-from-zero step.
        // xmm15 is the FP scratch (bug-17); one GPR is commandeered across the
        // sequence and restored, chosen to differ from `dst`.
        "fcvtas_x_from_d" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            // A commandeered GPR holds `d` for the final subtract. rax and rcx are
            // both non-allocatable (bug-284 pins that), so either is safe to
            // clobber under a push/pop; pick whichever is not `dst`.
            let scratch = if dst == 0 { 1 } else { 0 };
            let mut b = Vec::new();
            b.extend(enc_push_reg(scratch));
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, dst, src)); // cvttsd2si dst, src → t
            b.extend(enc_sse_cvt(0xf2, 0x2a, true, 15, dst)); // cvtsi2sd xmm15, dst → (double)t
            b.extend(enc_sse_rr(Some(0xf2), 0x5c, 15, src)); // subsd xmm15, src → f = t − x
            b.extend(enc_sse_rr(Some(0xf2), 0x58, 15, 15)); // addsd xmm15, xmm15 → 2f
            b.extend(enc_roundsd(15, 15, 3)); // roundsd xmm15, xmm15, trunc → d
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, scratch, 15)); // cvttsd2si scratch, xmm15
            b.extend(alu_rr(0x29, dst, scratch)); // sub dst, scratch → t − d
            b.extend(enc_pop_reg(scratch));
            Ok(Encoded::plain(b))
        }
        // 32-bit variable rotate-right (`rorv_w`/`rotr_w`): ror r32, cl.
        "rorv_w" => var_shift_w(instruction, 1),
        // movsd xmm, [base+disp] / movsd [base+disp], xmm (F2 0F 10 / 11 /r).
        "ldr_d" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?
                .parse::<i32>()
                .map_err(|_| "x86 ldr_d: bad offset".to_string())?;
            Ok(Encoded::plain(enc_movsd_mem(0x10, dst, base, disp)))
        }
        "str_d" => {
            let src = fp_reg(field(instruction, "src")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?
                .parse::<i32>()
                .map_err(|_| "x86 str_d: bad offset".to_string())?;
            Ok(Encoded::plain(enc_movsd_mem(0x11, src, base, disp)))
        }

        // --- v128 SIMD (SSE2/SSE4.1) — plan-00-H Phase 3 --------------------
        // 128-bit unaligned load/store (movups, 0F 10 / 11). `v`/`q` map to xmm.
        "ldr_q" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?
                .parse::<i32>()
                .map_err(|_| "x86 ldr_q: bad offset".to_string())?;
            Ok(Encoded::plain(enc_movups_mem(0x10, dst, base, disp)))
        }
        "str_q" => {
            let src = fp_reg(field(instruction, "src")?)?;
            let base = reg(field(instruction, "base")?)?;
            let disp = field(instruction, "offset")?
                .parse::<i32>()
                .map_err(|_| "x86 str_q: bad offset".to_string())?;
            Ok(Encoded::plain(enc_movups_mem(0x11, src, base, disp)))
        }
        // Packed f64×2 three-operand arithmetic (66 0F op); commutativity noted.
        "fadd_v" => vec3_op(instruction, 0x58, true),
        "fmul_v" => vec3_op(instruction, 0x59, true),
        "fsub_v" => vec3_op(instruction, 0x5C, false),
        "fdiv_v" => vec3_op(instruction, 0x5E, false),
        "fmin_v" => vec3_op(instruction, 0x5D, false),
        "fmax_v" => vec3_op(instruction, 0x5F, false),
        // Packed integer i64×2 add/sub (paddq/psubq).
        "add_v" => vec3_op(instruction, 0xD4, true),
        "sub_v" => vec3_op(instruction, 0xFB, false),
        // Bitwise 128-bit (pand/por/pxor) — all commutative.
        "and_v" => vec3_op(instruction, 0xDB, true),
        "orr_v" => vec3_op(instruction, 0xEB, true),
        "eor_v" => vec3_op(instruction, 0xEF, true),
        // Packed f64×2 unary sqrt (sqrtpd, 66 0F 51).
        "fsqrt_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            Ok(Encoded::plain(enc_sse_rr(Some(0x66), 0x51, dst, src)))
        }
        // fneg/fabs: xor/and each lane with the sign mask, built in xmm15 from
        // an all-ones vector shifted (psllq 63 = 0x8000…; psrlq 1 = 0x7FFF…).
        "fneg_v" | "fabs_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let neg = m == "fneg_v";
            let mut b = enc_sse_rr(Some(0x66), 0x76, 15, 15); // pcmpeqd xmm15,xmm15
            b.extend(enc_psxlq(
                if neg { 0x06 } else { 0x02 },
                15,
                if neg { 63 } else { 1 },
            ));
            if dst != src {
                b.extend(enc_movaps(dst, src));
            }
            // xorpd (66 0F 57) / andpd (66 0F 54) dst, xmm15
            b.extend(enc_sse_rr(
                Some(0x66),
                if neg { 0x57 } else { 0x54 },
                dst,
                15,
            ));
            Ok(Encoded::plain(b))
        }
        // Integer i64 negate: 0 - src (staged through xmm15 so dst may alias src).
        "neg_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15
            b.extend(enc_sse_rr(Some(0x66), 0xFB, 15, src)); // psubq xmm15, src
            b.extend(enc_movaps(dst, 15));
            Ok(Encoded::plain(b))
        }
        // Integer i64 absolute: mask = (0 > x) ? -1 : 0; |x| = (x ^ mask) - mask.
        "abs_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15
            b.extend(enc_sse38_rr(0x37, 15, src)); // pcmpgtq xmm15, src  (0 > src)
            if dst != src {
                b.extend(enc_movaps(dst, src));
            }
            b.extend(enc_sse_rr(Some(0x66), 0xEF, dst, 15)); // pxor dst, mask
            b.extend(enc_sse_rr(Some(0x66), 0xFB, dst, 15)); // psubq dst, mask
            Ok(Encoded::plain(b))
        }
        // Signed integer i64 lane compares → all-ones/all-zeros mask.
        "cmgt_v" => {
            // pcmpgtq dst(=lhs), rhs  (SSE4.2, 66 0F 38 37).
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let b = if dst == rhs && dst != lhs {
                let mut b = enc_movaps(15, rhs);
                b.extend(enc_movaps(dst, lhs));
                b.extend(enc_sse38_rr(0x37, dst, 15));
                b
            } else {
                let mut b = if dst == lhs {
                    Vec::new()
                } else {
                    enc_movaps(dst, lhs)
                };
                b.extend(enc_sse38_rr(0x37, dst, rhs));
                b
            };
            Ok(Encoded::plain(b))
        }
        "cmeq_v" => {
            // pcmpeqq (SSE4.1, 66 0F 38 29), commutative.
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(vec_sse38_commutative(0x29, dst, lhs, rhs)))
        }
        "cmge_v" => {
            // a>=b = ~(b>a): pcmpgtq(rhs,lhs) then NOT via pxor all-ones.
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = enc_movaps(15, lhs); // xmm15 = lhs (a)
            if dst != rhs {
                b.extend(enc_movaps(dst, rhs)); // dst = rhs (b)
            }
            b.extend(enc_sse38_rr(0x37, dst, 15)); // dst = b > a
            b.extend(enc_sse_rr(Some(0x66), 0x76, 15, 15)); // xmm15 = all ones
            b.extend(enc_sse_rr(Some(0x66), 0xEF, dst, 15)); // dst = ~(b>a) = a>=b
            Ok(Encoded::plain(b))
        }
        // Packed f64×2 compares (cmppd) → NaN-correct lane masks. fcmgt(a,b)=b<a
        // (LT, false on NaN); fcmge=b<=a (LE); fcmeq=a==b (EQ).
        "fcmgt_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(vec_cmppd_swapped(dst, lhs, rhs, 1)))
        }
        "fcmge_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(vec_cmppd_swapped(dst, lhs, rhs, 2)))
        }
        "fcmeq_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = if dst == rhs {
                enc_movaps(15, rhs)
            } else {
                Vec::new()
            };
            let r = if dst == rhs { 15 } else { rhs };
            if dst != lhs {
                b.extend(enc_movaps(dst, lhs));
            }
            b.extend(enc_cmppd(dst, r, 0));
            Ok(Encoded::plain(b))
        }
        // Packed f64×2 compares against zero. xmm15 = 0.0×2.
        "fcmgt_zero_v" | "fcmge_zero_v" | "fcmlt_zero_v" | "fcmle_zero_v" | "fcmeq_zero_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15 (0.0)
            match m {
                "fcmgt_zero_v" => {
                    // src>0 = 0<src : dst=0; cmpltpd dst,src
                    b.extend(enc_movaps(dst, 15));
                    b.extend(enc_cmppd(dst, src, 1));
                }
                "fcmge_zero_v" => {
                    // src>=0 = 0<=src : dst=0; cmplepd dst,src
                    b.extend(enc_movaps(dst, 15));
                    b.extend(enc_cmppd(dst, src, 2));
                }
                "fcmlt_zero_v" => {
                    // src<0 : dst=src; cmpltpd dst,0
                    if dst != src {
                        b.extend(enc_movaps(dst, src));
                    }
                    b.extend(enc_cmppd(dst, 15, 1));
                }
                "fcmle_zero_v" => {
                    // src<=0 : dst=src; cmplepd dst,0
                    if dst != src {
                        b.extend(enc_movaps(dst, src));
                    }
                    b.extend(enc_cmppd(dst, 15, 2));
                }
                _ => {
                    // src==0 : dst=src; cmpeqpd dst,0
                    if dst != src {
                        b.extend(enc_movaps(dst, src));
                    }
                    b.extend(enc_cmppd(dst, 15, 0));
                }
            }
            Ok(Encoded::plain(b))
        }
        // Directed rounding (roundpd, SSE4.1 66 0F 3A 09 /r ib).
        "frintp_v" | "frintm_v" | "frintz_v" | "frintn_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mode = match m {
                "frintn_v" => 0, // nearest-even
                "frintm_v" => 1, // floor
                "frintp_v" => 2, // ceil
                _ => 3,          // trunc
            };
            Ok(Encoded::plain(enc_roundpd(dst, src, mode)))
        }
        // Round to nearest, ties AWAY from zero (AArch64 frinta), packed. x86 has
        // no such roundpd mode, so the rule is realized directly.
        //
        // bug-295: this was the packed twin of the scalar `x + copysign(0.5, x)`
        // bug and double-rounded the same just-below-half family. It now uses the
        // same exact-fraction identity — t = trunc(x); f = t − x; result =
        // t − trunc(2f) — which no step rounds. dst holds t and src is read-only,
        // so both operands stay live and only xmm15 is needed: the packed form
        // needs neither the GPR nor the push/pop the old sequence used to
        // materialize its constants.
        "frinta_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = enc_roundpd(dst, src, 3); // dst = trunc(x) = t
            b.extend(enc_sse_rr(Some(0x66), 0x28, 15, dst)); // movapd xmm15, dst
            b.extend(enc_sse_rr(Some(0x66), 0x5C, 15, src)); // subpd xmm15, src → f = t − x
            b.extend(enc_sse_rr(Some(0x66), 0x58, 15, 15)); // addpd xmm15, xmm15 → 2f
            b.extend(enc_roundpd(15, 15, 3)); // xmm15 = trunc(2f) = d
            b.extend(enc_sse_rr(Some(0x66), 0x5C, dst, 15)); // subpd dst, xmm15 → t − d
            Ok(Encoded::plain(b))
        }
        // Immediate lane shifts (i64): psllq /6, psrlq /2 (66 0F 73).
        "shl_v" | "ushr_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let shift: u8 = field(instruction, "shift")?
                .parse()
                .map_err(|_| "x86 shl_v/ushr_v: bad shift".to_string())?;
            // AArch64 takes 0..=63 for `shl.2d` and 1..=64 for `ushr.2d`. A
            // `psllq`/`psrlq` count above 63 is architecturally defined to zero
            // the lane — exactly the AArch64 result for `ushr #64`. Past 64 the
            // immediate is malformed: reject rather than silently truncate.
            if shift > 64 {
                return Err(format!("x86 {m}: lane shift {shift} is out of range"));
            }
            let mut b = if dst == src {
                Vec::new()
            } else {
                enc_movaps(dst, src)
            };
            b.extend(enc_psxlq(
                if m == "shl_v" { 0x06 } else { 0x02 },
                dst,
                shift,
            ));
            Ok(Encoded::plain(b))
        }
        // Broadcast a GPR into both i64 lanes: movq xmm,r64 ; punpcklqdq xmm,xmm.
        "dup_v_from_x" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = reg(field(instruction, "src")?)?;
            let mut b = enc_movq_xmm_r64(dst, src);
            b.extend(enc_sse_rr(Some(0x66), 0x6C, dst, dst)); // punpcklqdq dst,dst
            Ok(Encoded::plain(b))
        }
        // Extract a lane to a GPR. Lane 0 = movq; lane 1 = pextrq (SSE4.1).
        "umov_x_from_v" => {
            let dst = reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let index: u8 = field(instruction, "index")?
                .parse()
                .map_err(|_| "x86 umov_x_from_v: bad index".to_string())?;
            if index == 0 {
                Ok(Encoded::plain(enc_movq_r64_xmm(dst, src)))
            } else {
                // pextrq r64, xmm, imm : 66 REX.W 0F 3A 16 /r ib
                Ok(Encoded::plain(vec![
                    0x66,
                    rex(true, src >= 8, false, dst >= 8),
                    0x0F,
                    0x3A,
                    0x16,
                    modrm(0b11, src, dst),
                    index,
                ]))
            }
        }
        // Bit-select (BSL): dst = (dst & lhs) | (~dst & rhs), dst is the mask.
        "bsl_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = enc_movaps(15, dst); // xmm15 = mask
            b.extend(enc_sse_rr(Some(0x66), 0xDB, dst, lhs)); // dst = mask & lhs
            b.extend(enc_sse_rr(Some(0x66), 0xDF, 15, rhs)); // xmm15 = ~mask & rhs (pandn)
            b.extend(enc_sse_rr(Some(0x66), 0xEB, dst, 15)); // dst |= xmm15
            Ok(Encoded::plain(b))
        }
        // Bit-insert-if-true (BIT): dst = (dst & ~rhs) | (lhs & rhs), mask in rhs.
        // Computed via the XOR identity dst = dst ^ ((dst ^ lhs) & rhs) so it needs
        // only ONE scratch — the reserved xmm15. The earlier form also clobbered
        // xmm14, which is ALLOCATABLE (only xmm15 is held out), destroying whatever
        // live FP value the allocator had colored there (allocation-dependent
        // corruption in the atan-based acos/asin kernels, which use BIT heavily).
        "bit_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            let mut b = enc_movaps(15, dst); // xmm15 = dst
            b.extend(enc_sse_rr(Some(0x66), 0xEF, 15, lhs)); // xmm15 = dst ^ lhs (pxor)
            b.extend(enc_sse_rr(Some(0x66), 0xDB, 15, rhs)); // xmm15 = (dst^lhs) & rhs
            b.extend(enc_sse_rr(Some(0x66), 0xEF, dst, 15)); // dst ^= xmm15 (pxor)
            Ok(Encoded::plain(b))
        }
        // Fused multiply-add/-subtract (single rounding, matches AArch64 fmla/fmls)
        // via FMA3 (x86-64-v3). fmla_v: dst += lhs*rhs → vfmadd231pd (B8); fmls_v:
        // dst -= lhs*rhs → vfnmadd231pd (BC). 231 form: reg=dst, vvvv=lhs, rm=rhs.
        "fmla_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(enc_vfma231pd(0xB8, dst, lhs, rhs)))
        }
        "fmls_v" => {
            let (dst, lhs, rhs) = three_fp(instruction)?;
            Ok(Encoded::plain(enc_vfma231pd(0xBC, dst, lhs, rhs)))
        }
        // Packed f64↔i64×2 conversions have no SSE2 form (AVX-512 only), so do
        // them lane-serial through rax/rdx (both free — excluded from the pool)
        // and xmm15. Lane 1 is brought to lane 0 with `pshufd imm=0xEE`.
        "fcvtzs_v" => {
            // dst[i] = trunc(src[i])  (cvttsd2si per lane). rax/rdx are the GPR
            // shuttle; preserve them (push/pop) — the kernels keep live pointers
            // there (they are only out of the *scratch pool*, not truly dead).
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = vec![0x50, 0x52]; // push rax ; push rdx
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, 0, src)); // cvttsd2si rax, src.lo
            b.extend(enc_pshufd(15, src, 0xEE)); // xmm15.lo = src.hi
            b.extend(enc_sse_cvt(0xf2, 0x2c, false, 2, 15)); // cvttsd2si rdx, src.hi
            b.extend(enc_movq_xmm_r64(dst, 0)); // dst.lo = rax
            b.extend(enc_movq_xmm_r64(15, 2)); // xmm15.lo = rdx
            b.extend(enc_sse_rr(Some(0x66), 0x6C, dst, 15)); // punpcklqdq dst,xmm15 → [rax,rdx]
            b.extend_from_slice(&[0x5A, 0x58]); // pop rdx ; pop rax
            Ok(Encoded::plain(b))
        }
        "scvtf_v" => {
            // dst[i] = (f64) src[i]  (cvtsi2sd per lane); preserve rax/rdx.
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let mut b = vec![0x50, 0x52]; // push rax ; push rdx
            b.extend(enc_movq_r64_xmm(0, src)); // rax = src.lo
            b.extend(enc_pshufd(15, src, 0xEE)); // xmm15.lo = src.hi
            b.extend(enc_movq_r64_xmm(2, 15)); // rdx = src.hi
            b.extend(enc_sse_cvt(0xf2, 0x2a, true, dst, 0)); // cvtsi2sd dst, rax
            b.extend(enc_sse_cvt(0xf2, 0x2a, true, 15, 2)); // cvtsi2sd xmm15, rdx
            b.extend(enc_sse_rr(Some(0x66), 0x14, dst, 15)); // unpcklpd dst,xmm15 → [lo,hi]
            b.extend_from_slice(&[0x5A, 0x58]); // pop rdx ; pop rax
            Ok(Encoded::plain(b))
        }
        // Arithmetic i64 lane shift-right by imm. SSE2 has no `psraq` (only 32-bit
        // `psrad`), so emulate: logical `psrlq` then OR in the sign fill for the
        // top `k` bits — sign_mask = (0 > src) per lane, shifted left by 64−k.
        "sshr_v" => {
            let dst = fp_reg(field(instruction, "dst")?)?;
            let src = fp_reg(field(instruction, "src")?)?;
            let k: u8 = field(instruction, "shift")?
                .parse()
                .map_err(|_| "x86 sshr_v: bad shift".to_string())?;
            // AArch64 `sshr.2d` takes 1..=64.
            if k > 64 {
                return Err(format!("x86 sshr_v: lane shift {k} is out of range"));
            }
            let mut b = enc_sse_rr(Some(0x66), 0xEF, 15, 15); // pxor xmm15,xmm15
            b.extend(enc_sse38_rr(0x37, 15, src)); // pcmpgtq xmm15,src → src<0 ? -1 : 0
            if k == 0 {
                // A no-op shift: clear the sign fill so `por` leaves `dst` alone.
                b.extend(enc_sse_rr(Some(0x66), 0xEF, 15, 15));
            } else if k < 64 {
                b.extend(enc_psxlq(0x06, 15, 64 - k)); // psllq xmm15, 64-k (top k bits)
            }
            // k == 64: keep the full sign mask unshifted. `psrlq dst, 64` zeroes
            // the lane (an x86 count > 63 is defined as such), so the `por` below
            // leaves exactly the sign fill — the AArch64 result. Clearing the mask
            // here (the old `else`) silently returned 0 for a negative lane
            // (bug-16).
            if dst != src {
                b.extend(enc_movaps(dst, src));
            }
            b.extend(enc_psxlq(0x02, dst, k)); // psrlq dst, k (logical)
            b.extend(enc_sse_rr(Some(0x66), 0xEB, dst, 15)); // por dst, sign fill
            Ok(Encoded::plain(b))
        }
        // Still unsupported: fcvtas_v (nearest ties-away), sshl_v, ushl_v.
        other => Err(format!("x86 encode: unsupported op {other}")),
    }
}
