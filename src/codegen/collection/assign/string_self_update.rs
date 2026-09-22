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
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

/// The bare names of the `String` builtins whose arm changes the block's length
/// and so needs the binding's capacity shadow ([`is_string_self_update`]). Each
/// plan-146 arm letter adds its names. `toString`'s identity arm changes nothing,
/// so it needs none.
pub(crate) const STRING_SHADOW_ARMS: &[&str] = &[];

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

    #[test]
    fn is_string_self_update_accepts_exactly_the_self_append_today() {
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
        assert!(!is_string_self_update(
            &call("strings.left", vec![local("s"), text("3")]),
            &root
        ));
    }
}
