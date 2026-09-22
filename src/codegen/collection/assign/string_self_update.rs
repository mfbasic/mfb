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
use crate::codegen::collection::assign::self_update::{
    self_update_builtin, self_update_call_parts, SelfUpdateSite,
};
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
    // plan-146-D, the grow arm.
    "padLeft",
    "padRight",
    "padLeftToWidth",
    "padRightToWidth",
    "repeat",
    "resourcePath",
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
    self_update_call_parts(value).is_some_and(|(target, args)| {
        self_update_builtin(target).is_some_and(|bare| STRING_SHADOW_ARMS.contains(&bare))
            && args.first().is_some_and(root)
    })
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

/// plan-146-D: how a grow row measures its result and where the new bytes go.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrowKind {
    /// `strings::padLeft` / `padRight`: pad to a SCALAR count.
    Pad { right: bool },
    /// `strings::padLeftToWidth` / `padRightToWidth`: pad to a COLUMN count.
    PadToWidth { right: bool },
    /// `strings::repeat`.
    Repeat,
    /// `os::resourcePath`: the result is `<base>/` + `relative`, so it is prefix
    /// growth whose prefix comes from the host.
    ResourcePath,
}

/// plan-146-D: the `String` builtins whose result is their first argument with
/// bytes added — `(bare name, arity, how it grows)`. One arm (`ArmId::StrGrow`)
/// serves them all.
pub(crate) const STRING_GROW_FNS: &[(&str, RangeInclusive<usize>, GrowKind)] = &[
    ("padLeft", 2..=3, GrowKind::Pad { right: false }),
    ("padRight", 2..=3, GrowKind::Pad { right: true }),
    (
        "padLeftToWidth",
        2..=3,
        GrowKind::PadToWidth { right: false },
    ),
    (
        "padRightToWidth",
        2..=3,
        GrowKind::PadToWidth { right: true },
    ),
    ("repeat", 2..=2, GrowKind::Repeat),
    ("resourcePath", 1..=1, GrowKind::ResourcePath),
];

/// plan-146-D: what a grow row's measure step hands its write step.
enum GrowWrite {
    /// `pad_count` copies of the padChar block's bytes.
    Pad {
        pad_slot: usize,
        pad_len_slot: usize,
        pad_count_slot: usize,
    },
    /// `times - 1` further copies of the block's own bytes.
    Repeat { times_slot: usize },
    /// A buffer holding the prefix bytes, and their count.
    Prefix { ptr_slot: usize, len_slot: usize },
}

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
        // G2 — a call (either node shape a lowering produces).
        let Some((target, args)) = self_update_call_parts(value) else {
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

    /// plan-146-D: `s = f(s, …)` for a builtin whose result is `s` with bytes
    /// added ([`STRING_GROW_FNS`]). The measure step computes the result's length
    /// and raises everything the row can raise; then the block is reserved (which
    /// allocates only when the binding outgrows its spare capacity, so a loop
    /// allocates `O(log n)` times), the added bytes are written — after `s`'s for
    /// an append, before them for a prefix, which moves `s`'s bytes right first —
    /// and the length is stored.
    pub(crate) fn try_inplace_string_grow_assign(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        let Some((target, _)) = self_update_call_parts(value) else {
            return Ok(false);
        };
        let Some(bare) = self_update_builtin(target) else {
            return Ok(false);
        };
        let Some((name, arity, kind)) = STRING_GROW_FNS
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
            .ok_or("native String grow self-update lost its capacity shadow")?;
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
        let value_slot = self.spill_to_slot("inplace_str_grow_value", &value_result.location);
        // The marker slot: its presence proves this arm fired (`ArmId::markers`).
        let newlen_slot = self.allocate_stack_object("inplace_str_grow", 8);
        let grow = self.emit_string_grow_measure(kind, value_slot, &rest, newlen_slot)?;
        // Every raise is behind us; the reserve is the only remaining failure, and
        // it allocates before it writes.
        self.emit_string_reserve(block_slot, shadow.slot, newlen_slot)?;
        self.emit_string_grow_write(kind, block_slot, newlen_slot, &grow);
        self.emit_string_set_len(block_slot, shadow.slot, newlen_slot);
        self.publish_string_shadow(&shadow)?;
        if let Some(local) = self.locals.get_mut(site.name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// What a grow row's measure step leaves for its write step.
    fn emit_string_grow_measure(
        &mut self,
        kind: GrowKind,
        value_slot: usize,
        rest: &[ValueResult],
        newlen_slot: usize,
    ) -> Result<GrowWrite, String> {
        let arg = |i: usize| -> Result<&ValueResult, String> {
            rest.get(i)
                .ok_or_else(|| "native String grow self-update lost an argument".to_string())
        };
        match kind {
            GrowKind::Pad { right } => {
                use crate::codegen::builtins::strings::gen_pad::{pad_measure, PadScratch};
                let width_slot = self.spill_to_slot("inplace_str_grow_width", &arg(0)?.location);
                let pad_slot = self.string_pad_char_slot(rest.get(1))?;
                let regs: Vec<VirtualRegister> = (0..9).map(|_| self.temporary_vreg()).collect();
                let measure = pad_measure(
                    self,
                    value_slot,
                    width_slot,
                    pad_slot,
                    &PadScratch {
                        scratch9: &regs[0],
                        scratch10: &regs[1],
                        scratch11: &regs[2],
                        scratch12: &regs[3],
                        scratch13: &regs[4],
                        scratch14: &regs[5],
                        scratch15: &regs[6],
                        scratch16: &regs[7],
                        scratch17: &regs[8],
                    },
                )?;
                let member = if right {
                    "strings.padRight"
                } else {
                    "strings.padLeft"
                };
                self.emit_grow_raise_here(&measure.invalid, member)?;
                let total = self.temporary_vreg();
                self.emit(abi::load_u64(
                    &total,
                    abi::stack_pointer(),
                    measure.total_slot,
                ));
                self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
                Ok(GrowWrite::Pad {
                    pad_slot,
                    pad_len_slot: measure.pad_len_slot,
                    pad_count_slot: measure.pad_count_slot,
                })
            }
            GrowKind::PadToWidth { right } => {
                let pad_slot = self.string_pad_char_slot(rest.get(1))?;
                let member = if right {
                    "strings.padRightToWidth"
                } else {
                    "strings.padLeftToWidth"
                };
                let (pad_len_slot, pad_count_slot) = self.emit_pad_to_width_measure(
                    value_slot,
                    arg(0)?,
                    pad_slot,
                    newlen_slot,
                    member,
                )?;
                Ok(GrowWrite::Pad {
                    pad_slot,
                    pad_len_slot,
                    pad_count_slot,
                })
            }
            GrowKind::ResourcePath => {
                // The prefix is built in the function's self-update scratch, which
                // the prescan gives every function holding this self-update.
                if self.self_update_scratch.is_none() {
                    return Err(
                        "native os.resourcePath self-update in a function without a scratch"
                            .to_string(),
                    );
                }
                // The relative path is checked first: `os::resourcePath` rejects a
                // `.` or `..` component before it builds anything.
                self.emit_reject_dot_components(value_slot)?;
                let (prefix_ptr_slot, prefix_len_slot) = self.emit_resource_prefix()?;
                let prefix_len = self.temporary_vreg();
                let value_ptr = self.temporary_vreg();
                let value_len = self.temporary_vreg();
                let total = self.temporary_vreg();
                let overflow = self.label("inplace_str_respath_overflow");
                let ok = self.label("inplace_str_respath_total_ok");
                self.emit(abi::load_u64(
                    &prefix_len,
                    abi::stack_pointer(),
                    prefix_len_slot,
                ));
                self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
                self.emit(abi::load_u64(&value_len, &value_ptr, 0));
                self.emit_checked_size_add(&total, &prefix_len, &value_len, &overflow);
                self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
                self.emit(abi::branch(&ok));
                self.emit(abi::label(&overflow));
                self.raise_error_bare("ErrOutOfMemory")?;
                self.emit(abi::label(&ok));
                Ok(GrowWrite::Prefix {
                    ptr_slot: prefix_ptr_slot,
                    len_slot: prefix_len_slot,
                })
            }
            GrowKind::Repeat => {
                use crate::codegen::builtins::strings::func_repeat::repeat_measure;
                let times_slot = self.spill_to_slot("inplace_str_grow_times", &arg(0)?.location);
                let measure = repeat_measure(self, value_slot, times_slot)?;
                self.emit_grow_raise_here(&measure.invalid, "strings.repeat")?;
                let total = self.temporary_vreg();
                self.emit(abi::load_u64(
                    &total,
                    abi::stack_pointer(),
                    measure.total_slot,
                ));
                self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
                Ok(GrowWrite::Repeat { times_slot })
            }
        }
    }

    /// Raise `ErrInvalidArgument` at a measure half's `invalid` label, which the
    /// copying lowering emits after its result instead. Nothing has been written to
    /// the binding at either point (failure atomicity).
    fn emit_grow_raise_here(&mut self, invalid: &str, label: &str) -> Result<(), String> {
        let ok = self.label("inplace_str_grow_ok");
        self.emit(abi::branch(&ok));
        self.emit(abi::label(invalid));
        self.raise_error(label, "ErrInvalidArgument")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    /// The padChar argument's block, or — when the call defaults it — a one-byte
    /// `" "` `String` block built in the frame. The copying lowering materializes
    /// an arena block for the default; an arm must allocate nothing per statement,
    /// and every reader of a `String` only wants `[len][bytes][NUL]` at the
    /// pointer, which a stack object provides.
    fn string_pad_char_slot(&mut self, pad: Option<&ValueResult>) -> Result<usize, String> {
        if let Some(pad) = pad {
            return Ok(self.spill_to_slot("inplace_str_grow_pad", &pad.location));
        }
        let block = self.allocate_stack_object("inplace_str_grow_space", 16);
        let one = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let address = self.temporary_vreg();
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        self.emit(abi::store_u64(&one, abi::stack_pointer(), block));
        self.emit(abi::move_immediate(&byte, "Byte", "32"));
        self.emit(abi::add_immediate(&address, abi::stack_pointer(), block));
        self.emit(abi::store_u8(&byte, &address, 8));
        self.emit(abi::store_u8(abi::ZERO, &address, 9));
        let slot = self.allocate_stack_object("inplace_str_grow_padptr", 8);
        self.emit(abi::store_u64(&address, abi::stack_pointer(), slot));
        Ok(slot)
    }

    /// The `*ToWidth` measure: `__strings_padToWidthCopies`' rule, natively — a
    /// negative `columns`, a `padChar` that is not exactly one scalar, and a
    /// zero-column `padChar` all raise; then `copies = (columns − displayWidth(value))
    /// / displayWidth(padChar)`, truncating (the undershoot rule), and the result's
    /// byte length is `byteLen(value) + copies × byteLen(padChar)`.
    /// Returns `(pad_len_slot, pad_count_slot)`.
    fn emit_pad_to_width_measure(
        &mut self,
        value_slot: usize,
        columns: &ValueResult,
        pad_slot: usize,
        newlen_slot: usize,
        member: &str,
    ) -> Result<(usize, usize), String> {
        use crate::codegen::builtins::strings::func_display_width::display_width_of;
        if columns.type_ != ParameterType::Integer {
            return Err(format!(
                "strings.padToWidth columns must be Integer, got {}",
                columns.type_
            ));
        }
        let columns_slot = self.spill_to_slot("inplace_str_width_columns", &columns.location);
        let pad_len_slot = self.allocate_stack_object("inplace_str_width_padlen", 8);
        let pad_count_slot = self.allocate_stack_object("inplace_str_width_copies", 8);
        let invalid = self.label("inplace_str_width_invalid");
        let ok = self.label("inplace_str_width_ok");
        let no_pad = self.label("inplace_str_width_no_pad");
        let have_pad = self.label("inplace_str_width_have_pad");

        let cols = self.temporary_vreg();
        let pad_ptr = self.temporary_vreg();
        let pad_len = self.temporary_vreg();
        let scalars = self.temporary_vreg();
        let unit = self.temporary_vreg();
        let have = self.temporary_vreg();
        let copies = self.temporary_vreg();
        let value_ptr = self.temporary_vreg();
        let value_len = self.temporary_vreg();
        let total = self.temporary_vreg();

        // columns >= 0.
        self.emit(abi::load_u64(&cols, abi::stack_pointer(), columns_slot));
        self.emit(abi::compare_immediate(&cols, "0"));
        self.emit(abi::branch_lt(&invalid));
        // `len(padChar) = 1` — one scalar, as the MFBASIC helper counts it.
        self.emit(abi::load_u64(&pad_ptr, abi::stack_pointer(), pad_slot));
        self.emit(abi::load_u64(&pad_len, &pad_ptr, 0));
        self.emit(abi::store_u64(&pad_len, abi::stack_pointer(), pad_len_slot));
        self.emit(abi::compare_immediate(&pad_len, "0"));
        self.emit(abi::branch_eq(&invalid));
        {
            let loop_label = self.label("inplace_str_width_scalars_loop");
            let not_cont = self.label("inplace_str_width_scalars_not_cont");
            let after = self.label("inplace_str_width_scalars_after");
            let done = self.label("inplace_str_width_scalars_done");
            let cursor = self.temporary_vreg();
            let byte = self.temporary_vreg();
            let masked = self.temporary_vreg();
            let mask = self.temporary_vreg();
            self.emit(abi::add_immediate(&cursor, &pad_ptr, 8));
            self.emit_scalar_count_loop(
                &cursor,
                &byte,
                &scalars,
                &masked,
                &mask,
                &unit,
                &pad_len,
                &loop_label,
                &not_cont,
                &after,
                &done,
            );
        }
        self.emit(abi::compare_immediate(&scalars, "1"));
        self.emit(abi::branch_ne(&invalid));
        // `unit = displayWidth(padChar) >= 1`.
        let pad_value = ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(pad_ptr.render()),
            text: String::new(),
        };
        self.emit(abi::load_u64(&pad_ptr, abi::stack_pointer(), pad_slot));
        let unit_value = display_width_of(self, &pad_value)?;
        self.emit(abi::move_register(&unit, &unit_value.location));
        self.emit(abi::compare_immediate(&unit, "1"));
        self.emit(abi::branch_lt(&invalid));
        // `have = displayWidth(value)`; no padding when it already fills the width.
        let value_value = ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(value_ptr.render()),
            text: String::new(),
        };
        self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
        let have_value = display_width_of(self, &value_value)?;
        self.emit(abi::move_register(&have, &have_value.location));
        self.emit(abi::load_u64(&cols, abi::stack_pointer(), columns_slot));
        self.emit(abi::compare_registers(&have, &cols));
        self.emit(abi::branch_ge(&no_pad));
        // `copies = (columns - have) / unit`, truncating toward zero.
        self.emit(abi::subtract_registers(&copies, &cols, &have));
        self.emit(abi::signed_divide_registers(&copies, &copies, &unit));
        self.emit(abi::branch(&have_pad));
        self.emit(abi::label(&no_pad));
        self.emit(abi::move_immediate(&copies, "Integer", "0"));
        self.emit(abi::label(&have_pad));
        self.emit(abi::store_u64(
            &copies,
            abi::stack_pointer(),
            pad_count_slot,
        ));
        // `newLen = byteLen(value) + copies * byteLen(padChar)`.
        self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&value_len, &value_ptr, 0));
        self.emit(abi::load_u64(&pad_len, abi::stack_pointer(), pad_len_slot));
        self.emit_checked_size_multiply(&total, &copies, &pad_len, &invalid);
        self.emit_checked_size_add(&total, &value_len, &total, &invalid);
        self.emit(abi::store_u64(&total, abi::stack_pointer(), newlen_slot));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.raise_error(member, "ErrInvalidArgument")?;
        self.emit(abi::label(&ok));
        Ok((pad_len_slot, pad_count_slot))
    }

    /// Write a grow row's added bytes into the (already reserved) block.
    fn emit_string_grow_write(
        &mut self,
        kind: GrowKind,
        block_slot: usize,
        newlen_slot: usize,
        write: &GrowWrite,
    ) {
        match (kind, write) {
            (
                GrowKind::Pad { right } | GrowKind::PadToWidth { right },
                GrowWrite::Pad {
                    pad_slot,
                    pad_len_slot,
                    pad_count_slot,
                },
            ) => {
                let block = self.temporary_vreg();
                let len = self.temporary_vreg();
                let newlen = self.temporary_vreg();
                let dst = self.temporary_vreg();
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
                if right {
                    // Append: the pads start where the bytes end.
                    self.emit(abi::add_immediate(&dst, &block, 8));
                    self.emit(abi::add_registers(&dst, &dst, &len));
                } else {
                    // Prefix: move the bytes right by the pad width first, from the
                    // far end, so the copy never overwrites a byte it has yet to
                    // read; then the pads go at the front.
                    self.emit_string_move_right(&block, &len, &newlen);
                    self.emit(abi::add_immediate(&dst, &block, 8));
                }
                self.emit_pad_copies(&dst, *pad_slot, *pad_len_slot, *pad_count_slot);
            }
            (GrowKind::Repeat, GrowWrite::Repeat { times_slot }) => {
                // The block already holds copy 1; write copies 2..times after it,
                // each from the fixed prefix `[8, 8 + len)`.
                let block = self.temporary_vreg();
                let len = self.temporary_vreg();
                let times = self.temporary_vreg();
                let dst = self.temporary_vreg();
                let src = self.temporary_vreg();
                let count = self.temporary_vreg();
                let outer = self.label("inplace_str_repeat_outer");
                let outer_done = self.label("inplace_str_repeat_done");
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&times, abi::stack_pointer(), *times_slot));
                self.emit(abi::add_immediate(&dst, &block, 8));
                self.emit(abi::add_registers(&dst, &dst, &len));
                self.emit(abi::label(&outer));
                self.emit(abi::compare_immediate(&times, "1"));
                self.emit(abi::branch_le(&outer_done));
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::add_immediate(&src, &block, 8));
                self.emit(abi::move_register(&count, &len));
                self.emit_copy_bytes(&dst, &src, &count, "inplace_str_repeat_copy");
                self.emit(abi::subtract_immediate(&times, &times, 1));
                self.emit(abi::branch(&outer));
                self.emit(abi::label(&outer_done));
            }
            (
                GrowKind::ResourcePath,
                GrowWrite::Prefix {
                    ptr_slot,
                    len_slot: prefix_len_slot,
                },
            ) => {
                let block = self.temporary_vreg();
                let len = self.temporary_vreg();
                let newlen = self.temporary_vreg();
                let dst = self.temporary_vreg();
                let src = self.temporary_vreg();
                let count = self.temporary_vreg();
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::load_u64(&len, &block, 0));
                self.emit(abi::load_u64(&newlen, abi::stack_pointer(), newlen_slot));
                self.emit_string_move_right(&block, &len, &newlen);
                self.emit(abi::load_u64(&block, abi::stack_pointer(), block_slot));
                self.emit(abi::add_immediate(&dst, &block, 8));
                self.emit(abi::load_u64(&src, abi::stack_pointer(), *ptr_slot));
                self.emit(abi::load_u64(
                    &count,
                    abi::stack_pointer(),
                    *prefix_len_slot,
                ));
                self.emit_copy_bytes(&dst, &src, &count, "inplace_str_respath_prefix");
            }
            _ => unreachable!("a grow row's write step matches its measure step"),
        }
    }

    /// Move the block's `len` bytes right so they end at `newlen`, copying from the
    /// far end (the regions overlap).
    fn emit_string_move_right(
        &mut self,
        block: &VirtualRegister,
        len: &VirtualRegister,
        newlen: &VirtualRegister,
    ) {
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let left = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let loop_label = self.label("inplace_str_grow_move_loop");
        let done = self.label("inplace_str_grow_move_done");
        // src = block + 8 + len, dst = block + 8 + newlen, both one past the end.
        self.emit(abi::add_immediate(&src, block, 8));
        self.emit(abi::add_registers(&src, &src, len));
        self.emit(abi::add_immediate(&dst, block, 8));
        self.emit(abi::add_registers(&dst, &dst, newlen));
        self.emit(abi::move_register(&left, len));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&left, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::subtract_immediate(&src, &src, 1));
        self.emit(abi::subtract_immediate(&dst, &dst, 1));
        self.emit(abi::load_u8(&byte, &src, 0));
        self.emit(abi::store_u8(&byte, &dst, 0));
        self.emit(abi::subtract_immediate(&left, &left, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
    }

    /// Write `pad_count` copies of the padChar block's bytes at `dst`.
    fn emit_pad_copies(
        &mut self,
        dst: &VirtualRegister,
        pad_slot: usize,
        pad_len_slot: usize,
        pad_count_slot: usize,
    ) {
        let pad = self.temporary_vreg();
        let pad_len = self.temporary_vreg();
        let count = self.temporary_vreg();
        let src = self.temporary_vreg();
        let left = self.temporary_vreg();
        let dst = dst.clone();
        let outer = self.label("inplace_str_pad_outer");
        let outer_done = self.label("inplace_str_pad_outer_done");
        let inner = self.label("inplace_str_pad_inner");
        let inner_done = self.label("inplace_str_pad_inner_done");
        let byte = self.temporary_vreg();
        self.emit(abi::load_u64(&count, abi::stack_pointer(), pad_count_slot));
        self.emit(abi::load_u64(&pad, abi::stack_pointer(), pad_slot));
        self.emit(abi::load_u64(&pad_len, abi::stack_pointer(), pad_len_slot));
        self.emit(abi::label(&outer));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&outer_done));
        self.emit(abi::add_immediate(&src, &pad, 8));
        self.emit(abi::move_register(&left, &pad_len));
        self.emit(abi::label(&inner));
        self.emit(abi::compare_immediate(&left, "0"));
        self.emit(abi::branch_eq(&inner_done));
        self.emit(abi::load_u8(&byte, &src, 0));
        self.emit(abi::store_u8(&byte, &dst, 0));
        self.emit(abi::add_immediate(&src, &src, 1));
        self.emit(abi::add_immediate(&dst, &dst, 1));
        self.emit(abi::subtract_immediate(&left, &left, 1));
        self.emit(abi::branch(&inner));
        self.emit(abi::label(&inner_done));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));
    }

    /// plan-146-D: the `os::resourcePath` prefix — `<executable dir>[/<suffix>]/` —
    /// written into a frame buffer once per call of the function holding the
    /// self-update, and its byte length returned in a slot.
    ///
    /// The copying lowering acquires the executable path from the host on every
    /// call (`os/func_resource_path.rs:lower_resource_path`); an arm may not (that
    /// is an allocation per statement), and a running process's own executable path
    /// cannot change, so the arm asks `os::executablePath()` once and caches the
    /// block it returns in `string_resource_base` (freed by the function's scope
    /// drop, `prescan_string_resource_base`). The prefix is then the same
    /// computation the helper does: strip `strip` components off the end of that
    /// path, append `/`, and — in an app build — the mode's resource suffix and a
    /// second `/` (`os/gen_paths.rs:resource_base_offset`). Those suffix bytes are
    /// compile-time constants, written as immediates rather than a data object.
    ///
    /// Returns `(prefix_ptr_slot, prefix_len_slot)`.
    fn emit_resource_prefix(&mut self) -> Result<(usize, usize), String> {
        let slot = self
            .string_resource_base
            .ok_or("native os.resourcePath self-update in a function with no base slot")?;
        let have = self.label("inplace_str_respath_have");
        let block = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_ne(&have));
        let helper = crate::target::shared::runtime::catalog::spec_for_call("os.executablePath")
            .ok_or("os.executablePath has no runtime helper spec")?
            .helper;
        let base = self.lower_runtime_helper_call(helper, "os.executablePath", &[], false)?;
        self.emit(abi::store_u64(&base.location, abi::stack_pointer(), slot));
        self.emit(abi::label(&have));

        let module_name = self.module_name.clone();
        let (strip, suffix) =
            crate::codegen::builtins::os::resource_base_offset(self.build_mode, &module_name);
        // The prefix buffer: the executable path's directory part, plus `/`, plus
        // the suffix and a second `/` in an app build.
        let extra = if suffix.is_empty() {
            1
        } else {
            suffix.len() + 2
        };
        let ptr_slot = self.allocate_stack_object("inplace_str_respath_prefix_ptr", 8);
        let len_slot = self.allocate_stack_object("inplace_str_respath_prefix_len", 8);
        let scratch_need = self.allocate_stack_object("inplace_str_respath_need", 8);

        let path = self.temporary_vreg();
        let path_len = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let scan = self.temporary_vreg();
        let left = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let need = self.temporary_vreg();
        let scan_loop = self.label("inplace_str_respath_scan");
        let scan_hit = self.label("inplace_str_respath_scan_hit");
        let prefix_ready = self.label("inplace_str_respath_prefix_ready");
        let fail = self.label("inplace_str_respath_fail");

        self.emit(abi::load_u64(&path, abi::stack_pointer(), slot));
        self.emit(abi::load_u64(&path_len, &path, 0));
        self.emit(abi::add_immediate(&bytes, &path, 8));
        // Scan back for the `strip`-th separator. The executable path uses the
        // host's separator: `/` everywhere but Windows, where it is `\`.
        let separator = if self.platform.target().starts_with("windows") {
            "92"
        } else {
            "47"
        };
        self.emit(abi::move_register(&scan, &path_len));
        self.emit(abi::move_immediate(&left, "Integer", &strip.to_string()));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_immediate(&scan, "0"));
        self.emit(abi::branch_eq(&fail));
        self.emit(abi::subtract_immediate(&scan, &scan, 1));
        self.emit(abi::add_registers(&byte, &bytes, &scan));
        self.emit(abi::load_u8(&byte, &byte, 0));
        self.emit(abi::compare_immediate(&byte, separator));
        self.emit(abi::branch_eq(&scan_hit));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_hit));
        self.emit(abi::subtract_immediate(&left, &left, 1));
        self.emit(abi::compare_immediate(&left, "0"));
        self.emit(abi::branch_eq(&prefix_ready));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&fail));
        self.raise_error("os.resourcePath", "ErrUnsupported")?;
        self.emit(abi::label(&prefix_ready));
        // The prefix is `scan` directory bytes plus `extra` joined bytes; build it
        // in the function's self-update scratch, which is reused across statements.
        self.emit(abi::add_immediate(&need, &scan, extra));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), len_slot));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), scratch_need));
        let dest = self.emit_reserve_self_update_scratch(scratch_need)?;
        let out = self.temporary_vreg();
        let src = self.temporary_vreg();
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&out, abi::stack_pointer(), dest));
        self.emit(abi::store_u64(&out, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&path, abi::stack_pointer(), slot));
        self.emit(abi::add_immediate(&src, &path, 8));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), len_slot));
        self.emit(abi::subtract_immediate(&count, &count, extra));
        self.emit_copy_bytes(&out, &src, &count, "inplace_str_respath_dir");
        // `out` now points just past the directory bytes: write `/`, the suffix and
        // its `/` as immediates (a compile-time constant, so no data object).
        let mut joined = String::from("/");
        if !suffix.is_empty() {
            joined.push_str(&suffix);
            joined.push('/');
        }
        let literal = self.temporary_vreg();
        for (i, b) in joined.bytes().enumerate() {
            self.emit(abi::move_immediate(&literal, "Byte", &b.to_string()));
            self.emit(abi::store_u8(&literal, &out, i));
        }
        Ok((ptr_slot, len_slot))
    }

    /// plan-146-D: `os::resourcePath` rejects a relative path with a `.` or `..`
    /// component (`os/gen_paths.rs:emit_reject_dot_component`, raised as
    /// `ErrInvalidPath` before anything is built). The arm checks the binding's own
    /// bytes the same way, before it writes one.
    fn emit_reject_dot_components(&mut self, value_slot: usize) -> Result<(), String> {
        let ptr = self.temporary_vreg();
        let len = self.temporary_vreg();
        let bytes = self.temporary_vreg();
        let index = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let comp_len = self.temporary_vreg();
        let all_dots = self.temporary_vreg();
        let walk = self.label("inplace_str_respath_walk");
        let boundary = self.label("inplace_str_respath_boundary");
        let not_boundary = self.label("inplace_str_respath_char");
        let next = self.label("inplace_str_respath_next");
        let not_dot = self.label("inplace_str_respath_not_dot");
        let end = self.label("inplace_str_respath_end");
        let bad = self.label("inplace_str_respath_bad");
        let component_ok = self.label("inplace_str_respath_component_ok");
        let done = self.label("inplace_str_respath_validated");

        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&len, &ptr, 0));
        self.emit(abi::add_immediate(&bytes, &ptr, 8));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&comp_len, "Integer", "0"));
        self.emit(abi::move_immediate(&all_dots, "Integer", "1"));
        self.emit(abi::label(&walk));
        self.emit(abi::compare_registers(&index, &len));
        self.emit(abi::branch_ge(&end));
        self.emit(abi::add_registers(&byte, &bytes, &index));
        self.emit(abi::load_u8(&byte, &byte, 0));
        // `/` is a component boundary on every target; `\` is one on Windows too,
        // and rejecting it everywhere only ever rejects MORE, never less, so the
        // arm's check is never weaker than the helper's.
        self.emit(abi::compare_immediate(&byte, "47"));
        self.emit(abi::branch_eq(&boundary));
        self.emit(abi::compare_immediate(&byte, "92"));
        self.emit(abi::branch_eq(&boundary));
        self.emit(abi::branch(&not_boundary));
        self.emit(abi::label(&boundary));
        self.emit_reject_dot_component_check(&comp_len, &all_dots, &bad, &component_ok);
        self.emit(abi::label(&component_ok));
        self.emit(abi::move_immediate(&comp_len, "Integer", "0"));
        self.emit(abi::move_immediate(&all_dots, "Integer", "1"));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&not_boundary));
        self.emit(abi::add_immediate(&comp_len, &comp_len, 1));
        self.emit(abi::compare_immediate(&byte, "46"));
        self.emit(abi::branch_eq(&not_dot));
        self.emit(abi::move_immediate(&all_dots, "Integer", "0"));
        self.emit(abi::label(&not_dot));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&walk));
        self.emit(abi::label(&end));
        self.emit_reject_dot_component_check(&comp_len, &all_dots, &bad, &done);
        self.emit(abi::label(&bad));
        self.raise_error("os.resourcePath", "ErrInvalidPath")?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// One component's verdict: an all-dots component of length 1 (`.`) or 2 (`..`)
    /// branches to `bad`, anything else to `ok`.
    fn emit_reject_dot_component_check(
        &mut self,
        comp_len: &VirtualRegister,
        all_dots: &VirtualRegister,
        bad: &str,
        ok: &str,
    ) {
        self.emit(abi::compare_immediate(all_dots, "0"));
        self.emit(abi::branch_eq(ok));
        self.emit(abi::compare_immediate(comp_len, "1"));
        self.emit(abi::branch_eq(bad));
        self.emit(abi::compare_immediate(comp_len, "2"));
        self.emit(abi::branch_eq(bad));
        self.emit(abi::branch(ok));
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
        let Some((target, _)) = self_update_call_parts(value) else {
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
