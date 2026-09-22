//! plan-146-B: the `String` half of the self-update seam.
//!
//! plan-142's resolver (`resolve_self_update`) is built around a collection layout
//! (G10), so a `String` arm gates through [`CodeBuilder::resolve_string_self_update`]
//! instead. Every `String` arm that changes the block's length keeps its spare
//! bytes in the binding's **capacity shadow** — a frame slot `strcap_<name>` for a
//! local, the hidden global `$strcap$<name>` for a global — exactly as the
//! self-append (`s = s & t`) always has; [`is_string_self_update`] is the one rule
//! that decides which bindings get one, and both prescans call it. The regrow the
//! concat arm has always done is [`CodeBuilder::emit_string_regrow`], shared by
//! [`CodeBuilder::emit_string_reserve`].

use std::ops::RangeInclusive;

use crate::codegen::collection::assign::inplace_dest::InPlaceDest;
use crate::codegen::collection::assign::self_update::{self_update_builtin, SelfUpdateSite};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::control::string_self_append_operands_of;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

/// The bare names of the `String` builtins whose arm changes the block's length
/// and so needs the binding's capacity shadow ([`is_string_self_update`]). Each
/// plan-146 arm letter adds its names. `toString`'s identity arm changes nothing,
/// so it needs none.
pub(crate) const STRING_SHADOW_ARMS: &[&str] = &[
    // plan-146-C, the window arm: a shrink leaves spare bytes only a shadow can
    // describe (plan-143 findings B.3 fact 1).
    "left",
    "right",
    "mid",
    "stripPrefix",
    "stripSuffix",
    "trim",
    "trimStart",
    "trimEnd",
    "trimChars",
    "graphemeAt",
    "pathBaseName",
    "pathDirName",
    "pathExtension",
];

/// Whether `value` is a `String` self-update of the binding `root` recognises
/// whose arm needs a capacity shadow: a left-associated `&` chain rooted at it
/// (the self-append), or a call `f(root, …)` of a [`STRING_SHADOW_ARMS`] builtin.
/// Both prescans (`prescan_string_self_appends`, `add_global_string_capacities`)
/// ask this, so a local and a global get a shadow under the same rule.
pub(crate) fn is_string_self_update(value: &NirValue, root: &dyn Fn(&NirValue) -> bool) -> bool {
    if string_self_append_operands_of(value, root).is_some() {
        return true;
    }
    matches!(value, NirValue::Call { target, args, .. }
        if self_update_builtin(target).is_some_and(|bare| STRING_SHADOW_ARMS.contains(&bare))
            && args.first().is_some_and(root))
}

/// plan-146-C: how a window row locates the result inside its first argument.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowKind {
    /// `strings::left` / `right`.
    LeftRight {
        right: bool,
    },
    /// `strings::stripPrefix` / `stripSuffix`.
    Strip {
        suffix: bool,
    },
    /// `strings::trim` / `trimStart` / `trimEnd`.
    Trim {
        start: bool,
        end: bool,
    },
    TrimChars,
    GraphemeAt,
    /// `strings::mid`.
    Mid,
    PathBaseName,
    /// The one row whose window can be longer than the binding (`.`), and the one
    /// whose window can lie outside it (the same constant).
    PathDirName,
    PathExtension,
}

/// plan-146-C: the thirteen `String` builtins whose result is a contiguous window
/// of their first argument — `(bare name, arity, how to find the window)`. One arm
/// (`ArmId::StrWindow`) serves them all.
pub(crate) const STRING_WINDOW_FNS: &[(&str, RangeInclusive<usize>, WindowKind)] = &[
    ("left", 2..=2, WindowKind::LeftRight { right: false }),
    ("right", 2..=2, WindowKind::LeftRight { right: true }),
    ("mid", 3..=3, WindowKind::Mid),
    ("stripPrefix", 2..=2, WindowKind::Strip { suffix: false }),
    ("stripSuffix", 2..=2, WindowKind::Strip { suffix: true }),
    (
        "trim",
        1..=1,
        WindowKind::Trim {
            start: true,
            end: true,
        },
    ),
    (
        "trimStart",
        1..=1,
        WindowKind::Trim {
            start: true,
            end: false,
        },
    ),
    (
        "trimEnd",
        1..=1,
        WindowKind::Trim {
            start: false,
            end: true,
        },
    ),
    ("trimChars", 2..=2, WindowKind::TrimChars),
    ("graphemeAt", 2..=2, WindowKind::GraphemeAt),
    ("pathBaseName", 1..=1, WindowKind::PathBaseName),
    ("pathDirName", 1..=1, WindowKind::PathDirName),
    ("pathExtension", 1..=1, WindowKind::PathExtension),
];

/// A binding's capacity shadow, opened for one statement: `slot` holds the spare
/// bytes past the block's length. For a global it is a working copy of the hidden
/// global `publish`, stored back by [`CodeBuilder::publish_string_shadow`].
pub(crate) struct StringShadow {
    pub(crate) slot: usize,
    pub(crate) publish: Option<String>,
}

/// The slots, registers and labels one regrow of a `String` block uses
/// ([`CodeBuilder::emit_string_regrow`]). The caller allocates them, in its own
/// order, so an arm's frame and register numbering are its own.
pub(crate) struct StringRegrow<'a> {
    /// Holds the block pointer; repointed at the new block.
    pub(crate) name_slot: usize,
    /// The shadow; set to the new block's spare bytes.
    pub(crate) shadow_slot: usize,
    /// The byte count the new block must hold at least.
    pub(crate) need_slot: usize,
    /// The block's length after the regrow: stored in its header, and the spare
    /// bytes are counted from it.
    pub(crate) len_after_slot: usize,
    pub(crate) newcap_slot: usize,
    pub(crate) newbuf_slot: usize,
    pub(crate) oldsize_slot: usize,
    pub(crate) ptr: &'a VirtualRegister,
    pub(crate) len: &'a VirtualRegister,
    /// Scratch for the current payload capacity.
    pub(crate) cap: &'a VirtualRegister,
    pub(crate) spare: &'a VirtualRegister,
    pub(crate) newcap: &'a VirtualRegister,
    pub(crate) step_scratch: &'a VirtualRegister,
    pub(crate) newlen: &'a VirtualRegister,
    /// Left pointing just past the copied bytes, for `fill`.
    pub(crate) dst: &'a VirtualRegister,
    pub(crate) oldsize: &'a VirtualRegister,
    pub(crate) alloc_ok: &'a str,
    pub(crate) cap_keep: &'a str,
    /// The label prefixes of the geometric step and the old-bytes copy.
    pub(crate) step_prefix: &'a str,
    pub(crate) copy_prefix: &'a str,
}

impl CodeBuilder<'_> {
    /// Resolve `site.name = value` as the `String` self-update `name(site.name, …)`
    /// with an argument count in `arity`, or decline having emitted nothing. The
    /// gates, in order (plan-146-B §3): `G-string-dest`, `G-string-type`, G2,
    /// G3/G4, G5/G6, `G21-string`, G1, `G-shadow` (when `needs_shadow`) and
    /// `G-global-operand`. `Some(args)` = every gate passed.
    pub(crate) fn resolve_string_self_update<'v>(
        &self,
        site: &SelfUpdateSite<'_>,
        value: &'v NirValue,
        name: &str,
        arity: RangeInclusive<usize>,
        needs_shadow: bool,
    ) -> Option<&'v [NirValue]> {
        // G-string-dest — a local, a global, or (letter G) a reference; never a
        // record or `STATE` field.
        if site.field.is_some()
            || !matches!(
                site.dest,
                InPlaceDest::Direct { .. } | InPlaceDest::Global { .. } | InPlaceDest::Ref { .. }
            )
        {
            return None;
        }
        // G-string-type.
        if site.type_ != ParameterType::String {
            return None;
        }
        // G2 — a call.
        let NirValue::Call { target, args, .. } = value else {
            return None;
        };
        // G3 / G4 — the builtin and its arity.
        if self_update_builtin(target) != Some(name) || !arity.contains(&args.len()) {
            return None;
        }
        // G5 / G6 — the first argument is the binding itself.
        if !args.first().is_some_and(|arg| site.is_self(arg)) {
            return None;
        }
        // G21-string — no later argument reads the binding: the arm rewrites the
        // bytes the copying path would still read.
        if args[1..].iter().any(|arg| site.read_by(arg)) {
            return None;
        }
        // G1 — a by-ref capture has no shadow it shares with its owner (plan-142-G
        // Correction G1); letter G lifts this.
        if site.by_ref {
            return None;
        }
        // G-shadow.
        if needs_shadow && !self.string_shadow_exists(site) {
            return None;
        }
        // G-global-operand — an argument that stores to the global would replace
        // the block under the arm.
        if let InPlaceDest::Global { name, .. } = &site.dest {
            let leaf = crate::codegen::engine::value::store_reach::StoreLeaf::Global(name);
            if self.values_reach_store(&args[1..], leaf) {
                return None;
            }
        }
        Some(args)
    }

    /// Whether `site`'s binding has a capacity shadow: the local's frame slot, or
    /// the global's hidden global.
    pub(crate) fn string_shadow_exists(&self, site: &SelfUpdateSite<'_>) -> bool {
        match &site.dest {
            InPlaceDest::Global { name, .. } => self.global_string_capacity(name).is_some(),
            InPlaceDest::Direct { .. } => self.string_capacity_slots.contains_key(site.name),
            _ => false,
        }
    }

    /// Open `site`'s capacity shadow for the statement: the local's frame slot, or
    /// a working copy (`concat_global_strcap`) of the global's hidden global, which
    /// [`publish_string_shadow`](Self::publish_string_shadow) stores back. `Ok(None)`
    /// when the binding has none.
    pub(crate) fn string_shadow_slot(
        &mut self,
        site: &SelfUpdateSite<'_>,
    ) -> Result<Option<StringShadow>, String> {
        let shadow_global = match &site.dest {
            InPlaceDest::Global { name, .. } => match self.global_string_capacity(name) {
                Some(shadow) => Some(shadow),
                None => return Ok(None),
            },
            _ => None,
        };
        let frame_shadow = self.string_capacity_slots.get(site.name).copied();
        if shadow_global.is_none() && frame_shadow.is_none() {
            return Ok(None);
        }
        Ok(Some(match shadow_global {
            Some(shadow) => {
                let slot = self.allocate_stack_object("concat_global_strcap", 8);
                let address = self.load_global_address(&shadow)?;
                let spare = self.allocate_register();
                self.emit(abi::load_u64(&spare, address.as_str(), 0));
                self.emit(abi::store_u64(&spare, abi::stack_pointer(), slot));
                StringShadow {
                    slot,
                    publish: Some(shadow),
                }
            }
            None => StringShadow {
                slot: frame_shadow.ok_or("native self-append lost its capacity shadow")?,
                publish: None,
            },
        }))
    }

    /// Store a global's working shadow back into its hidden global. Nothing for a
    /// local's frame slot.
    pub(crate) fn publish_string_shadow(&mut self, shadow: &StringShadow) -> Result<(), String> {
        if let Some(hidden) = &shadow.publish {
            let spare = self.allocate_register();
            self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow.slot));
            let address = self.load_global_address(hidden)?;
            self.emit(abi::store_u64(&spare, address.as_str(), 0));
        }
        Ok(())
    }

    /// Regrow the `String` block at `r.name_slot` geometrically: allocate a block
    /// of payload capacity `max(step(len + spare), need)`, store `len_after` in its
    /// header, copy the block's current `len` bytes, let `fill` write what follows
    /// them (from `r.dst`, which it may advance), free the old block at its true
    /// size (`len + spare + 9`), repoint the slot and set the shadow to the spare
    /// bytes past `len_after`. The allocation comes before any write, so an
    /// `ErrOutOfMemory` leaves the binding unchanged. This is the self-append's
    /// regrow (bug-77, bug-560), shared.
    pub(crate) fn emit_string_regrow(
        &mut self,
        r: &StringRegrow<'_>,
        fill: &mut dyn FnMut(&mut Self, &VirtualRegister) -> Result<(), String>,
    ) -> Result<(), String> {
        self.emit(abi::load_u64(r.ptr, abi::stack_pointer(), r.name_slot));
        self.emit(abi::load_u64(r.len, r.ptr, 0)); // len
        self.emit(abi::load_u64(r.spare, abi::stack_pointer(), r.shadow_slot)); // spare
        self.emit(abi::add_registers(r.cap, r.len, r.spare)); // current payload capacity
                                                              // bug-77: oldsize = payload_capacity + 9 ([len:8][bytes][NUL]). The
                                                              // headroom is tracked only in the shadow slot, so a tight len+9 free
                                                              // would under-free; capture the real size now before it is clobbered.
        self.emit(abi::add_immediate(r.oldsize, r.cap, 9));
        self.emit(abi::store_u64(
            r.oldsize,
            abi::stack_pointer(),
            r.oldsize_slot,
        ));
        self.emit_geometric_step(
            r.cap,
            r.newcap,
            r.step_scratch,
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            r.step_prefix,
        );
        // newcap_payload = max(step, need).
        self.emit(abi::load_u64(r.newlen, abi::stack_pointer(), r.need_slot));
        self.emit(abi::compare_registers(r.newcap, r.newlen));
        self.emit(abi::branch_hi(r.cap_keep));
        self.emit(abi::branch_eq(r.cap_keep));
        self.emit(abi::move_register(r.newcap, r.newlen));
        self.emit(abi::label(r.cap_keep));
        self.emit(abi::store_u64(
            r.newcap,
            abi::stack_pointer(),
            r.newcap_slot,
        ));
        // alloc size = 8 (len word) + newcap_payload + 1 (NUL).
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
        self.emit(abi::add_immediate(abi::c_arg(0), r.newcap, 9));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(r.alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(r.alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            r.newbuf_slot,
        ));
        // newbuf[0] = len_after.
        self.emit(abi::load_u64(
            r.newlen,
            abi::stack_pointer(),
            r.len_after_slot,
        ));
        self.emit(abi::store_u64(r.newlen, abi::mfb_return(1), 0));
        // Copy the current bytes (len) to newbuf+8.
        self.emit(abi::load_u64(r.ptr, abi::stack_pointer(), r.name_slot));
        self.emit(abi::load_u64(r.len, r.ptr, 0)); // len
        self.emit(abi::add_immediate(r.ptr, r.ptr, 8)); // old data
        self.emit(abi::add_immediate(r.dst, abi::mfb_return(1), 8)); // new data
        self.emit_copy_bytes(r.dst, r.ptr, r.len, r.copy_prefix);
        // dst now points at newbuf+8+len.
        fill(self, r.dst)?;
        // bug-77: free the old buffer before installing the new pointer. The
        // old buffer pointer is still live at name_slot (overwritten just
        // below) and its size is in oldsize_slot; the new buffer is already
        // spilled in newbuf_slot, so it survives this call. arena_free clobbers
        // all caller-saved registers. This free runs exactly once per regrow.
        // plan-71-C Family-1a: ptr is arg 0 of arena-free → `%arg0`.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            r.name_slot,
        ));
        self.emit(abi::load_u64(
            abi::c_arg(1),
            abi::stack_pointer(),
            r.oldsize_slot,
        ));
        self.emit_arena_free_call();
        // Install new buffer; spare = newcap_payload - len_after.
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            r.newbuf_slot,
        ));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            r.name_slot,
        ));
        self.emit(abi::load_u64(r.newcap, abi::stack_pointer(), r.newcap_slot));
        self.emit(abi::load_u64(
            r.newlen,
            abi::stack_pointer(),
            r.len_after_slot,
        ));
        self.emit(abi::subtract_registers(r.newcap, r.newcap, r.newlen));
        self.emit(abi::store_u64(
            r.newcap,
            abi::stack_pointer(),
            r.shadow_slot,
        ));
        Ok(())
    }

    /// Make the `String` block at `name_slot` hold at least the byte count in
    /// `need_slot`: it already fits in `len + spare`, or it regrows geometrically
    /// ([`emit_string_regrow`](Self::emit_string_regrow)), keeping its length and
    /// bytes. Allocates before any write, so an `ErrOutOfMemory` leaves the
    /// binding unchanged.
    pub(crate) fn emit_string_reserve(
        &mut self,
        name_slot: usize,
        shadow_slot: usize,
        need_slot: usize,
    ) -> Result<(), String> {
        let len_slot = self.allocate_stack_object("str_reserve_len", 8);
        let newcap_slot = self.allocate_stack_object("str_reserve_newcap", 8);
        let newbuf_slot = self.allocate_stack_object("str_reserve_newbuf", 8);
        let oldsize_slot = self.allocate_stack_object("str_reserve_oldsize", 8);
        let ptr = self.temporary_vreg();
        let len = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let spare = self.temporary_vreg();
        let newcap = self.temporary_vreg();
        let step_scratch = self.temporary_vreg();
        let newlen = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let oldsize = self.temporary_vreg();
        let need = self.temporary_vreg();
        let regrow = self.label("str_reserve_regrow");
        let alloc_ok = self.label("str_reserve_alloc_ok");
        let cap_keep = self.label("str_reserve_cap_keep");
        let done = self.label("str_reserve_done");

        // Fits when need <= len + spare.
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&len, &ptr, 0));
        self.emit(abi::store_u64(&len, abi::stack_pointer(), len_slot));
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot));
        self.emit(abi::add_registers(&cap, &len, &spare));
        self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
        self.emit(abi::compare_registers(&need, &cap));
        self.emit(abi::branch_hi(&regrow));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&regrow));
        self.emit_string_regrow(
            &StringRegrow {
                name_slot,
                shadow_slot,
                need_slot,
                len_after_slot: len_slot,
                newcap_slot,
                newbuf_slot,
                oldsize_slot,
                ptr: &ptr,
                len: &len,
                cap: &cap,
                spare: &spare,
                newcap: &newcap,
                step_scratch: &step_scratch,
                newlen: &newlen,
                dst: &dst,
                oldsize: &oldsize,
                alloc_ok: &alloc_ok,
                cap_keep: &cap_keep,
                step_prefix: "str_reserve_step",
                copy_prefix: "str_reserve_old",
            },
            &mut |b, dst| {
                // The block stays a valid `String` of its old length.
                let zero = b.temporary_vreg();
                b.emit(abi::move_immediate(&zero, "Integer", "0"));
                b.emit(abi::store_u8(&zero, dst, 0));
                Ok(())
            },
        )?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Set the length of the `String` block at `name_slot` to the byte count in
    /// `new_len_slot` (no more than `len + spare`): add the bytes it gives up to the
    /// shadow (`shadow += oldLen - newLen`, which a grow makes negative), store the
    /// length, and write the NUL.
    pub(crate) fn emit_string_set_len(
        &mut self,
        name_slot: usize,
        shadow_slot: usize,
        new_len_slot: usize,
    ) {
        let ptr = self.temporary_vreg();
        let old_len = self.temporary_vreg();
        let new_len = self.temporary_vreg();
        let spare = self.temporary_vreg();
        let zero = self.temporary_vreg();
        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64(&old_len, &ptr, 0));
        self.emit(abi::load_u64(&new_len, abi::stack_pointer(), new_len_slot));
        self.emit(abi::load_u64(&spare, abi::stack_pointer(), shadow_slot));
        self.emit(abi::add_registers(&spare, &spare, &old_len));
        self.emit(abi::subtract_registers(&spare, &spare, &new_len));
        self.emit(abi::store_u64(&spare, abi::stack_pointer(), shadow_slot));
        self.emit(abi::store_u64(&new_len, &ptr, 0));
        self.emit(abi::add_registers(&ptr, &ptr, &new_len));
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u8(&zero, &ptr, 8));
    }

    /// plan-146-C: `s = f(s, …)` for a builtin whose result is a contiguous window
    /// of `s`'s own bytes ([`STRING_WINDOW_FNS`]). The window step is the code the
    /// copying lowering runs (every check and raise happens there, before a byte
    /// moves); then the window is moved down to offset 8 of `s`'s own block, the
    /// length and NUL are stored, and the bytes it gave up become spare capacity in
    /// the binding's shadow.
    pub(crate) fn try_inplace_string_window_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let NirValue::Call { target, .. } = value else {
            return Ok(false);
        };
        let Some(bare) = self_update_builtin(target) else {
            return Ok(false);
        };
        let Some((name, arity, kind)) = STRING_WINDOW_FNS
            .iter()
            .find(|(name, _, _)| *name == bare)
            .map(|(name, arity, kind)| (*name, arity.clone(), *kind))
        else {
            return Ok(false);
        };
        let Some(args) = self.resolve_string_self_update(site, value, name, arity, true) else {
            return Ok(false);
        };
        let args: Vec<NirValue> = args.to_vec();

        let block_slot = site.dest.block_slot();
        let shadow = self
            .string_shadow_slot(site)?
            .ok_or("native String window self-update lost its capacity shadow")?;
        // The window step reads the binding's block and the remaining arguments,
        // which `G21-string` proved do not read the binding.
        let mut rest = Vec::new();
        for arg in &args[1..] {
            rest.push(self.lower_value(arg)?);
        }
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
        let value_result = ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(block.render()),
            text: String::new(),
        };
        let (ptr, len) = self.emit_string_window(kind, &value_result, &rest)?;
        // The marker slot: its presence proves this arm fired (`ArmId::markers`).
        let ptr_slot = self.allocate_stack_object("inplace_str_window", 8);
        let len_slot = self.allocate_stack_object("inplace_str_window_len", 8);
        self.emit(abi::store_u64(&ptr, abi::stack_pointer(), ptr_slot));
        self.emit(abi::store_u64(&len, abi::stack_pointer(), len_slot));
        // `fs::pathDirName("")` is `.` — one byte where the binding had none — and
        // its window points at the constant, never into the block, so a regrow
        // cannot invalidate it. Every other row's window is inside the block and no
        // longer than it.
        if kind == WindowKind::PathDirName {
            self.emit_string_reserve(block_slot, shadow.slot, len_slot)?;
        }
        // Move the window to `block + 8`. The destination is never after the
        // source, so a forward copy reads each byte before it is overwritten.
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), block_slot));
        self.emit(abi::add_immediate(&dst, &dst, 8));
        self.emit(abi::load_u64(&src, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), len_slot));
        self.emit_copy_bytes(&dst, &src, &count, "inplace_str_window_move");
        self.emit_string_set_len(block_slot, shadow.slot, len_slot);
        self.publish_string_shadow(&shadow)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// The window step of one [`STRING_WINDOW_FNS`] row: the copying lowering's own
    /// half, except `graphemeAt`'s, whose copying path reads its span out of a
    /// freshly built `List OF String` (plan-146-C Correction C1).
    fn emit_string_window(
        &mut self,
        kind: WindowKind,
        value: &ValueResult,
        rest: &[ValueResult],
    ) -> Result<(VirtualRegister, VirtualRegister), String> {
        use crate::codegen::builtins::strings::{
            gen_graphemes, gen_left_right, gen_strip, gen_trim,
        };
        let arg = |i: usize| -> Result<&ValueResult, String> {
            rest.get(i)
                .ok_or_else(|| "native String window self-update lost an argument".to_string())
        };
        match kind {
            WindowKind::LeftRight { right } => {
                gen_left_right::lower_strings_left_right_window(self, value, arg(0)?, right)
            }
            WindowKind::Strip { suffix } => {
                gen_strip::lower_strings_strip_window(self, value, arg(0)?, suffix)
            }
            WindowKind::Trim { start, end } => {
                gen_trim::lower_strings_trim_window(self, value, start, end)
            }
            WindowKind::TrimChars => {
                crate::codegen::builtins::strings::func_trim_chars::window(self, value, arg(0)?)
            }
            WindowKind::GraphemeAt => gen_graphemes::grapheme_at_window(self, value, arg(0)?),
            WindowKind::Mid => self.emit_string_mid_window(value, arg(0)?, arg(1)?),
            WindowKind::PathBaseName => self.fs_path_base_name_window(value),
            WindowKind::PathDirName => self.fs_path_dir_name_window(value),
            WindowKind::PathExtension => {
                let (start, span, _done) = self.fs_path_extension_window(value)?;
                Ok((start, span))
            }
        }
    }

    /// `strings::mid`'s window, with its `ErrIndexOutOfRange` raise emitted right
    /// after it (the copying lowering emits the same raise after its copy; nothing
    /// has written to the binding at either point).
    fn emit_string_mid_window(
        &mut self,
        value: &ValueResult,
        start: &ValueResult,
        count: &ValueResult,
    ) -> Result<(VirtualRegister, VirtualRegister), String> {
        use crate::codegen::collection::search::builder_search::MidStringRegs;
        let value_slot = self.spill_to_slot("inplace_str_mid_value", &value.location);
        let start_slot = self.spill_to_slot("inplace_str_mid_start", &start.location);
        let count_slot = self.spill_to_slot("inplace_str_mid_count", &count.location);
        let regs: Vec<VirtualRegister> = (0..13).map(|_| self.temporary_vreg()).collect();
        let window = self.lower_mid_string_window(
            value_slot,
            start_slot,
            count_slot,
            &MidStringRegs {
                value_ptr: &regs[0],
                string_len: &regs[1],
                cursor: &regs[2],
                remaining: &regs[3],
                scalar_index: &regs[4],
                start_index: &regs[5],
                count_value: &regs[6],
                end_index: &regs[7],
                byte: &regs[8],
                mask: &regs[9],
                start_ptr: &regs[10],
                end_ptr: &regs[11],
                byte_len: &regs[12],
            },
        )?;
        let ok = self.label("inplace_str_mid_ok");
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&window.invalid_range));
        self.raise_error("strings.mid", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&ok));
        let ptr = self.temporary_vreg();
        let len = self.temporary_vreg();
        self.emit(abi::load_u64(
            &ptr,
            abi::stack_pointer(),
            window.start_ptr_slot,
        ));
        self.emit(abi::load_u64(
            &len,
            abi::stack_pointer(),
            window.byte_len_slot,
        ));
        Ok((ptr, len))
    }

    /// plan-146-B: `s = toString(s)` on a `String`. `toString` of a `String` is the
    /// identity (`.ai/codegen-invariants.md`: "`toString(String)` is the IDENTITY
    /// arm — it hands back its own argument"), so in place it is a no-op: the block
    /// stays, nothing is freed, nothing is stored. The copying path allocated a
    /// block, copied the bytes into it (bug-667) and freed the old one, every
    /// statement. Emits one marker slot, so the matrix can see the arm fired.
    pub(crate) fn try_inplace_string_identity_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        // No `G-shadow`: the length does not change, so the block keeps whatever
        // spare bytes it had, and the shadow still describes it.
        if self
            .resolve_string_self_update(site, value, "toString", 1..=1, false)
            .is_none()
        {
            return Ok(false);
        }
        self.allocate_stack_object("inplace_str_identity", 8);
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::collection::assign::self_update::{FieldContainer, FieldSite};
    use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
    use crate::operators::BinaryOp;

    fn call(target: &str, args: Vec<NirValue>) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args,
            loc: NirSourceLoc::default(),
        }
    }
    fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }
    fn text(value: &str) -> NirValue {
        NirValue::Const {
            type_: ParameterType::String,
            value: value.to_string(),
        }
    }
    fn site(dest: InPlaceDest) -> SelfUpdateSite<'static> {
        SelfUpdateSite {
            name: "s",
            type_: ParameterType::String,
            dest,
            by_ref: false,
            field: None,
        }
    }

    /// plan-146-B Phase 2: each gate of `resolve_string_self_update` declines its
    /// shape, and the resolver answers the arguments when every gate passes.
    #[test]
    fn resolve_string_self_update_runs_every_gate() {
        let harness = BuilderHarness::default();
        let mut builder = harness.builder("gates", &TestPlatform);
        builder.string_capacity_slots.insert("s".to_string(), 16);
        let direct = || InPlaceDest::Direct { slot: 8 };
        let left = call("strings.left", vec![local("s"), text("3")]);
        let resolve = |b: &CodeBuilder<'_>, site: &SelfUpdateSite<'_>, value: &NirValue, shadow| {
            b.resolve_string_self_update(site, value, "left", 2..=2, shadow)
                .is_some()
        };
        // Every gate passes.
        assert!(resolve(&builder, &site(direct()), &left, true));
        // G-string-dest: a field site.
        let mut field = site(direct());
        field.field = Some(FieldSite {
            container: FieldContainer::Record { local: "s" },
            path: Vec::new(),
            field: "b",
            field_index: 1,
            record_type: ParameterType::named("Rec"),
        });
        assert!(!resolve(&builder, &field, &left, true), "G-string-dest");
        // G-string-type.
        let mut list = site(direct());
        list.type_ = ParameterType::list_of(ParameterType::Integer);
        assert!(!resolve(&builder, &list, &left, true), "G-string-type");
        // G2: not a call.
        assert!(!resolve(&builder, &site(direct()), &local("s"), true), "G2");
        // G3: another builtin; G4: the wrong arity.
        let right = call("strings.right", vec![local("s"), text("3")]);
        assert!(!resolve(&builder, &site(direct()), &right, true), "G3");
        let short = call("strings.left", vec![local("s")]);
        assert!(!resolve(&builder, &site(direct()), &short, true), "G4");
        // G5/G6: the first argument is another binding.
        let other = call("strings.left", vec![local("t"), text("3")]);
        assert!(!resolve(&builder, &site(direct()), &other, true), "G5/G6");
        // G21-string: a later argument reads the binding.
        let reads = call(
            "strings.left",
            vec![local("s"), call("len", vec![local("s")])],
        );
        assert!(
            !resolve(&builder, &site(direct()), &reads, true),
            "G21-string"
        );
        // G1: a by-ref capture.
        let mut by_ref = site(InPlaceDest::Ref {
            ref_slot: 8,
            block_slot: 24,
        });
        by_ref.by_ref = true;
        assert!(!resolve(&builder, &by_ref, &left, true), "G1");
        // G-shadow: no shadow, and only when the arm needs one.
        builder.string_capacity_slots.clear();
        assert!(!resolve(&builder, &site(direct()), &left, true), "G-shadow");
        assert!(resolve(&builder, &site(direct()), &left, false));
        // A global without a hidden shadow global fails G-shadow too.
        let global = SelfUpdateSite {
            name: "g",
            type_: ParameterType::String,
            dest: InPlaceDest::Global {
                name: "g".to_string(),
                block_slot: 24,
            },
            by_ref: false,
            field: None,
        };
        let global_left = call(
            "strings.left",
            vec![
                NirValue::Global {
                    name: "g".to_string(),
                    type_: ParameterType::String,
                },
                text("3"),
            ],
        );
        assert!(
            !resolve(&builder, &global, &global_left, true),
            "G-shadow (global)"
        );
        assert!(resolve(&builder, &global, &global_left, false));
    }

    /// The shadow rule: the `&` chain, and the calls whose arm changes the block's
    /// length ([`STRING_SHADOW_ARMS`]). `toString`'s identity arm changes nothing,
    /// so it is not one.
    #[test]
    fn is_string_self_update_accepts_the_self_append_and_the_length_changing_arms() {
        let root = |v: &NirValue| matches!(v, NirValue::Local(n) if n == "s");
        let append = NirValue::Binary {
            op: BinaryOp::Concat,
            left: Box::new(local("s")),
            right: Box::new(text("t")),
            loc: NirSourceLoc::default(),
        };
        assert!(is_string_self_update(&append, &root));
        assert!(!is_string_self_update(
            &call("toString", vec![local("s")]),
            &root
        ));
        assert!(is_string_self_update(
            &call("strings.left", vec![local("s"), text("3")]),
            &root
        ));
        // Another binding's self-update is not this one's.
        assert!(!is_string_self_update(
            &call("strings.left", vec![local("t"), text("3")]),
            &root
        ));
        // A `String` builtin with no arm that changes the length.
        assert!(!is_string_self_update(
            &call("strings.upper", vec![local("s")]),
            &root
        ));
    }
}
