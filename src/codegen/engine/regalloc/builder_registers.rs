// --- codegen tier imports (migration) ---
use super::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::regalloc;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::target::shared::regmodel::RegClass;
use std::collections::HashMap;
impl CodeBuilder<'_> {
    /// Mint an integer virtual register. The physical register is assigned after
    /// the whole function is lowered (`regalloc::allocate`); the liveness-driven
    /// coloring spills under pressure, so minting never fails.
    /// [`Self::temporary_vreg`] is the same allocation under the name the many
    /// scratch-register call sites use.
    pub(crate) fn allocate_register(&mut self) -> VirtualRegister {
        let vreg = self.next_vreg;
        self.next_vreg += 1;
        // Advance the bump counter: some lowerings advance it as a positional
        // reservation (`while self.next_register <= 12 { … }` in
        // `builder_numeric`), so it must always move or those loops never
        // terminate; coloring ignores the counter.
        self.next_register += 1;
        VirtualRegister::new(RegClass::Int, vreg)
    }

    /// Mint a floating-point (`d`-class) virtual register (plan-03 Stage C). The
    /// physical `d`-register is assigned after the whole function is lowered;
    /// chained float arithmetic stays resident in `d`-registers instead of
    /// round-tripping its bit pattern through a GPR.
    pub(crate) fn allocate_fp_register(&mut self) -> VirtualRegister {
        let vreg = self.next_fp_vreg;
        self.next_fp_vreg += 1;
        VirtualRegister::new(RegClass::Fp, vreg)
    }

    /// Color the fully-lowered instruction stream: rewrite every virtual
    /// register to a physical register (or spill slot) using the selected
    /// strategy. Allocates frame slots for any spills and records the
    /// callee-saved registers the coloring used so `finalize_frame` saves them.
    /// Must run after the body is fully emitted and before the peephole pass and
    /// `finalize_frame`, which both expect physical register names (plan-03).
    pub(crate) fn run_register_allocation(&mut self) -> Result<(), String> {
        // Every register the builders and kernels once hardcoded — the GPR
        // scratch pool (x8-x17/x20-x28) and the SIMD kernels' high-FP file
        // (d/v/q 16-31) — is now minted as a virtual register at the emit site
        // (`temporary_vreg`/`temporary_fp_vreg`), so the stream arriving here
        // carries only vregs, ABI-role registers, and pinned registers. There is
        // no rename/patch pass.
        // plan-34-D: the pre-selection stream is the shared MIR — it must name
        // no physical register. Tokens realize in `backend.select` below and
        // colors are assigned by `regalloc::allocate`; a physical name arriving
        // here is a shared-lowering regression.
        if let Some(offense) = regalloc::find_physical_operand(&self.instructions) {
            return Err(format!(
                "shared lowering for '{}' violated the zero-physical-register \
                 invariant (plan-34-D): {offense}",
                self.current_symbol
            ));
        }
        // MIR seam (plan-00-A): the fully-lowered, pre-allocation stream is the
        // point where the neutral MIR layer sits (`NIR → MIR → select → alloc`,
        // `mir.md §2`/§3). A `-mir` dump captures this function's MIR here (with
        // virtual registers intact); the stream is then raised to the neutral
        // MIR and selected straight back to AArch64 before allocation. This is
        // the sole code path since plan-00-G flipped the default to MIR and
        // deleted the `direct` (no-MIR) backend.
        if mir::capture_enabled() {
            mir::capture_function(&self.current_symbol, mir::lower_to_mir(&self.instructions));
        }
        let backend = mir::active_backend();
        // Move the (dropped-after) pre-selection stream through the MIR boundary
        // so each `fields` Vec is carried, not re-cloned (plan-84 Phase 2). The
        // capture above (rare, `-mir` only) already took its own borrowing copy.
        let neutral = mir::lower_to_mir_owned(std::mem::take(&mut self.instructions));
        self.instructions = backend.select(neutral);
        // plan-100 §3: the Opt2 seam — between selection and register
        // allocation, the last point where the stream is still in virtual
        // registers. Runs the Level-1 MIR constant folder today; the future
        // home of Plan2 (CFG + SSA/def-use and its demand-driven analyses) →
        // further Opt2 passes → out-of-SSA. The machine peepholes below
        // deliberately stay post-regalloc: they read physical registers.
        crate::optimizer::opt2::optimize_mir(
            &mut self.instructions,
            backend.register_model(),
            crate::optimizer::active_opt_level(),
        );
        // 16-aligned so FP spill slots hit `str q`'s alignment requirement (the
        // slot stride is `spill_slot_bytes()` = 16 on every backend).
        let spill_base = type_utils::align(self.stack_size, 16);
        let outcome = regalloc::allocate(
            &mut self.instructions,
            backend.register_model(),
            spill_base,
            &[],
        );
        for offset in &outcome.spill_slots {
            self.stack_slots.push(CodeStackSlot {
                name: format!("spill_{}", self.stack_slots.len()),
                type_: "spill".to_string(),
                offset: *offset as i32,
            });
        }
        self.stack_size =
            spill_base + outcome.spill_slots.len() * backend.register_model().spill_slot_bytes();
        for register in outcome.extra_callee_saved {
            if !self.used_callee_saved.contains(&register) {
                self.used_callee_saved.push(register);
            }
        }
        Ok(())
    }

    /// Mint a scratch virtual register for a builder that would otherwise name a
    /// physical register directly. The historical name for
    /// [`Self::allocate_register`] from when the two spellings differed in
    /// error handling (bug-70, `--regalloc bump`'s exhaustible pool — since
    /// removed); kept because the scratch-register call sites read naturally
    /// with it.
    pub(crate) fn temporary_vreg(&mut self) -> VirtualRegister {
        self.allocate_register()
    }

    /// Mint a floating-point virtual register for a builder that would otherwise
    /// name a physical high-FP register (`d`/`v`/`q` 16–31) directly. The FP
    /// sibling of [`Self::temporary_vreg`].
    pub(crate) fn temporary_fp_vreg(&mut self) -> VirtualRegister {
        self.allocate_fp_register()
    }

    pub(crate) fn mark_register_used(&mut self, register: &str) {
        if abi::is_callee_saved(register)
            && !self.used_callee_saved.iter().any(|saved| saved == register)
        {
            self.used_callee_saved.push(register.to_string());
        }
    }

    pub(crate) fn reset_temporary_registers(&mut self) {
        self.next_register = 8;
    }

    pub(crate) fn local_constants(&self) -> HashMap<String, Option<NirValue>> {
        self.locals
            .iter()
            .map(|(name, local)| (name.clone(), local.constant.clone()))
            .collect()
    }

    pub(crate) fn restore_local_constants(
        &mut self,
        constants: &HashMap<String, Option<NirValue>>,
    ) {
        for (name, local) in &mut self.locals {
            local.constant = constants.get(name).cloned().unwrap_or(None);
        }
    }

    pub(crate) fn clear_local_constants(&mut self) {
        for local in self.locals.values_mut() {
            local.constant = None;
        }
    }

    pub(crate) fn allocate_stack_object(&mut self, name: &str, size: usize) -> usize {
        let offset = self.stack_size;
        let size = align(size, 8);
        self.stack_size += size;
        self.stack_slots.push(CodeStackSlot {
            name: format!("{name}_{}", self.stack_slots.len()),
            type_: name.to_string(),
            offset: offset as i32,
        });
        offset
    }

    /// Spill `register` to a fresh 8-byte stack slot named `label`, returning the
    /// slot offset. Type-agnostic: the value may be a pointer, an Integer, a
    /// length — anything that fits a word. (It was `spill_to_slot`, a name
    /// that asserted a `String` type this helper never checked and that ~4 of its
    /// call sites did not hold.)
    pub(crate) fn spill_to_slot(&mut self, label: &str, register: impl Into<Operand>) -> usize {
        let slot = self.allocate_stack_object(label, 8);
        self.emit(abi::store_u64(register, abi::stack_pointer(), slot));
        slot
    }

    pub(crate) fn label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.next_label);
        self.next_label += 1;
        label
    }

    #[track_caller]
    pub(crate) fn emit(&mut self, mut instruction: CodeInstruction) {
        // plan-71-C Phase 0: refine the instruction's source to the builder's
        // `self.emit(...)` call site (the caller of `emit`), which is the exact
        // shared-builder line the audit needs — more precise than the `abi::`
        // helper line captured at construction. Audit-only metadata; byte-identical.
        instruction.source = Some(core::panic::Location::caller());
        self.instructions.push(instruction);
    }
}
