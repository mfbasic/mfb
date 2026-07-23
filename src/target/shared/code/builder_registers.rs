use super::*;

impl CodeBuilder<'_> {
    /// Mint an integer virtual register. The physical register is assigned after
    /// the whole function is lowered (`regalloc::allocate`).
    ///
    /// This is the **fallible** spelling: under `-regalloc bump` the eager
    /// physical pool can be exhausted, and that `Err` is returned to the caller
    /// to bubble. [`Self::temporary_vreg`] is the same allocation with the error
    /// *recorded* on the builder instead of returned, for the many lowerings
    /// that build an instruction list and have no `Result` to bubble through
    /// (bug-70). Neither is more correct — pick by whether the call site can
    /// propagate an error. Under linear-scan (the default) both are infallible.
    pub(super) fn allocate_register(&mut self) -> Result<String, String> {
        let vreg = self.next_vreg;
        self.next_vreg += 1;
        debug_assert_eq!(self.vreg_eager.len(), vreg as usize);
        // Advance the bump counter for *both* strategies. Some lowerings advance
        // it as a positional reservation (`while self.next_register <= 12 { … }`
        // in `builder_numeric`), so it must always move or those loops never
        // terminate; linear-scan simply ignores the counter when coloring.
        let slot = self.next_register;
        self.next_register += 1;
        match self.regalloc_kind {
            regalloc::RegallocKind::BumpAndReset => {
                // Compute the bump allocator's eager physical now — both to drive
                // the byte-identical `BumpAndReset` replay (index == virtual
                // register number) and to mark its callee-saved use in the legacy
                // order so the frame layout is unchanged (plan-03 Stage A §4.1).
                match abi::temporary_register(slot) {
                    Ok(physical) => {
                        self.mark_register_used(&physical);
                        self.vreg_eager.push(physical);
                    }
                    Err(err) => {
                        // The fixed-pool bump oracle has no spilling, so a deep
                        // single-statement expression can exhaust it. Keep
                        // `vreg_eager` aligned with `next_vreg` (push a placeholder)
                        // so later minting does not trip the length invariant, then
                        // surface the graceful error — the caller aborts the build
                        // before coloring rewrites this vreg (bug-70).
                        self.vreg_eager.push(String::new());
                        return Err(format!(
                            "{err} while lowering native function '{}'",
                            self.current_symbol
                        ));
                    }
                }
            }
            regalloc::RegallocKind::LinearScan => {
                // No eager physical: the liveness-driven coloring assigns physical
                // registers (or spill slots) after the whole function is lowered,
                // so a deep expression that would overflow the bump pool no longer
                // fails — it spills instead (plan-03 Stage B §4.4).
                self.vreg_eager.push(String::new());
            }
        }
        Ok(regalloc::vreg_name(vreg))
    }

    /// Mint a floating-point (`d`-class) virtual register (plan-03 Stage C). The
    /// physical `d`-register is assigned after the whole function is lowered;
    /// chained float arithmetic stays resident in `d`-registers instead of
    /// round-tripping its bit pattern through a GPR.
    pub(super) fn allocate_fp_register(&mut self) -> Result<String, String> {
        let vreg = self.next_fp_vreg;
        self.next_fp_vreg += 1;
        debug_assert_eq!(self.fp_vreg_eager.len(), vreg as usize);
        match self.regalloc_kind {
            regalloc::RegallocKind::BumpAndReset => {
                // The bump oracle replays a per-statement `d0`–`d7` sequence.
                match abi::fp_temporary_register(self.next_fp_register) {
                    Ok(physical) => {
                        self.next_fp_register += 1;
                        self.fp_vreg_eager.push(physical);
                    }
                    Err(err) => {
                        // Fixed 8-deep FP pool; a deep float expression exhausts
                        // it. Keep `fp_vreg_eager` aligned with `next_fp_vreg` and
                        // surface the graceful error (bug-70); the caller aborts
                        // before coloring runs.
                        self.fp_vreg_eager.push(String::new());
                        return Err(format!(
                            "{err} while lowering native function '{}'",
                            self.current_symbol
                        ));
                    }
                }
            }
            regalloc::RegallocKind::LinearScan => {
                self.fp_vreg_eager.push(String::new());
            }
        }
        Ok(regalloc::fp_vreg_name(vreg))
    }

    /// Color the fully-lowered instruction stream: rewrite every virtual
    /// register to a physical register (or spill slot) using the selected
    /// strategy. Allocates frame slots for any spills and records the
    /// callee-saved registers the coloring used so `finalize_frame` saves them.
    /// Must run after the body is fully emitted and before the peephole pass and
    /// `finalize_frame`, which both expect physical register names (plan-03).
    pub(super) fn run_register_allocation(&mut self) -> Result<(), String> {
        // Surface any scratch-register exhaustion an infallible vreg minter
        // recorded (only `-regalloc bump` can exhaust) as a clean build error
        // before coloring rewrites the placeholder vregs. Aborting here — rather
        // than letting `regalloc::allocate`'s `rewrite` panic on the empty
        // placeholder or the former `.expect` ICE — is the graceful path (bug-70).
        if let Some(err) = self.regalloc_error.take() {
            return Err(err);
        }
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
        let neutral = mir::lower_to_mir(&self.instructions);
        self.instructions = backend.select(&neutral);
        // 16-aligned so FP spill slots hit `str q`'s alignment requirement (the
        // slot stride is `spill_slot_bytes()` = 16 on every backend).
        let spill_base = type_utils::align(self.stack_size, 16);
        let outcome = regalloc::allocate(
            self.regalloc_kind,
            &mut self.instructions,
            &self.vreg_eager,
            &self.fp_vreg_eager,
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
    /// physical register directly. Infallible under linear-scan (the active
    /// strategy); a convenience over `allocate_register` for the many builder
    /// call sites that used fixed `xN` scratch and cannot bubble a `Result`.
    ///
    /// Under `-regalloc bump` a deep-enough single-statement expression can
    /// exhaust the fixed pool. Rather than panic (the former `.expect`, an ICE),
    /// the exhaustion is recorded in `regalloc_error` and surfaced as a clean
    /// build error by `run_register_allocation`; a placeholder vreg is returned so
    /// lowering can proceed to that checkpoint, where the build aborts before this
    /// vreg is colored (bug-70).
    pub(super) fn temporary_vreg(&mut self) -> String {
        match self.allocate_register() {
            Ok(vreg) => vreg,
            Err(err) => {
                if self.regalloc_error.is_none() {
                    self.regalloc_error = Some(err);
                }
                // `allocate_register` already advanced `next_vreg` and pushed the
                // matching `vreg_eager` placeholder, so this names that vreg.
                regalloc::vreg_name(self.next_vreg - 1)
            }
        }
    }

    /// Mint a floating-point virtual register for a builder that would otherwise
    /// name a physical high-FP register (`d`/`v`/`q` 16–31) directly. Infallible
    /// under linear-scan; the FP sibling of [`Self::temporary_vreg`] for the SIMD
    /// kernels that used fixed FP homes and cannot bubble a `Result`. Records an
    /// exhaustion under `-regalloc bump` like [`Self::temporary_vreg`] (bug-70).
    pub(super) fn temporary_fp_vreg(&mut self) -> String {
        match self.allocate_fp_register() {
            Ok(vreg) => vreg,
            Err(err) => {
                if self.regalloc_error.is_none() {
                    self.regalloc_error = Some(err);
                }
                regalloc::fp_vreg_name(self.next_fp_vreg - 1)
            }
        }
    }

    pub(super) fn mark_register_used(&mut self, register: &str) {
        if abi::is_callee_saved(register)
            && !self.used_callee_saved.iter().any(|saved| saved == register)
        {
            self.used_callee_saved.push(register.to_string());
        }
    }

    pub(super) fn reset_temporary_registers(&mut self) {
        self.next_register = 8;
        self.next_fp_register = 0;
    }

    pub(super) fn local_constants(&self) -> HashMap<String, Option<NirValue>> {
        self.locals
            .iter()
            .map(|(name, local)| (name.clone(), local.constant.clone()))
            .collect()
    }

    pub(super) fn restore_local_constants(
        &mut self,
        constants: &HashMap<String, Option<NirValue>>,
    ) {
        for (name, local) in &mut self.locals {
            local.constant = constants.get(name).cloned().unwrap_or(None);
        }
    }

    pub(super) fn clear_local_constants(&mut self) {
        for local in self.locals.values_mut() {
            local.constant = None;
        }
    }

    pub(super) fn allocate_stack_object(&mut self, name: &str, size: usize) -> usize {
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
    pub(super) fn spill_to_slot(&mut self, label: &str, register: &str) -> usize {
        let slot = self.allocate_stack_object(label, 8);
        self.emit(abi::store_u64(register, abi::stack_pointer(), slot));
        slot
    }

    pub(super) fn label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.next_label);
        self.next_label += 1;
        label
    }

    pub(super) fn emit(&mut self, instruction: CodeInstruction) {
        self.instructions.push(instruction);
    }
}
