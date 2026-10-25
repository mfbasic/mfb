//! plan-179-C: charging arena blocks against the arena memory cap.
//!
//! One process-wide counter, `_mfb_rt_arena_mapped_bytes`, holds the bytes of every
//! arena block mapped and not yet unmapped, across every thread's arena. It is
//! guarded by `_mfb_rt_arena_map_lock`, a mutex taken only when a block is mapped or
//! unmapped — never on the allocation fast path. The lock calls go through the
//! thread seam, so Windows gets an SRWLOCK.
//!
//! The counter is touched only while `_mfb_rt_arena_map_limit` is nonzero. That one
//! test is what keeps the two hazards of a held lock away:
//!
//! * `_mfb_shutdown` stores 0 into the limit first, so a SIGINT/SIGTERM that lands
//!   on a thread inside a charge, refund or destroy section runs its teardown without
//!   taking the (non-recursive) lock that thread holds.
//! * A spawn's fork child stores 0 into the limit before it allocates, so the child's
//!   copy of a lock another parent thread held at `fork()` is never waited on.

use super::*;
use crate::codegen::engine::builder::EmitCtx;
use crate::codegen::runtime::thread::emit_thread_external_call;

/// The process-wide mapped-bytes counter (u64).
pub(crate) const ARENA_MAPPED_BYTES_SYMBOL: &str = "_mfb_rt_arena_mapped_bytes";
/// The mutex guarding the counter: 64 bytes, 8-aligned, which holds a macOS
/// `pthread_mutex_t` (64), a glibc or musl one (at most 48) and an SRWLOCK (8).
pub(crate) const ARENA_MAP_LOCK_SYMBOL: &str = "_mfb_rt_arena_map_lock";
const ARENA_MAP_LOCK_BYTES: usize = crate::codegen::builtins::os::OS_ENV_LOCK_SIZE;

/// The counter and the statically initialized lock (no runtime init call: the
/// bytes are each platform's `PTHREAD_MUTEX_INITIALIZER` / `SRWLOCK_INIT`, the same
/// image the `os::` env lock uses).
pub(super) fn data_objects(platform: &dyn CodegenPlatform) -> Vec<CodeDataObject> {
    vec![
        CodeDataObject {
            symbol: ARENA_MAPPED_BYTES_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.arena_mapped_bytes.v1 { u64 mappedBytes }".to_string(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        },
        CodeDataObject {
            symbol: ARENA_MAP_LOCK_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.arena_map_lock.v1 { u8 mutex[64] }".to_string(),
            align: 8,
            size: ARENA_MAP_LOCK_BYTES,
            value: crate::codegen::builtins::os::os_env_lock_init_hex(platform.family()),
        },
    ]
}

/// The instruction stream a site emits into, and the symbol it emits for.
pub(crate) struct CapSite<'a> {
    pub(crate) symbol: &'a str,
    pub(crate) platform: &'a dyn CodegenPlatform,
    pub(crate) platform_imports: &'a HashMap<String, String>,
    pub(crate) instructions: &'a mut Vec<CodeInstruction>,
    pub(crate) relocations: &'a mut Vec<CodeRelocation>,
}

impl CapSite<'_> {
    fn address(&mut self, symbol: &str, dst: &VirtualRegister) {
        push_symbol_address(
            self.symbol,
            symbol,
            dst,
            self.instructions,
            self.relocations,
        );
    }

    /// `pthread_mutex_lock`/`unlock(&_mfb_rt_arena_map_lock)`.
    fn lock_call(&mut self, call: &str) -> Result<(), String> {
        push_symbol_address(
            self.symbol,
            ARENA_MAP_LOCK_SYMBOL,
            abi::c_arg(0),
            self.instructions,
            self.relocations,
        );
        emit_thread_external_call(
            &mut EmitCtx {
                symbol: self.symbol,
                platform_imports: self.platform_imports,
                platform: self.platform,
                instructions: self.instructions,
                relocations: self.relocations,
            },
            call,
        )
    }

    /// `counter += delta` (or `-=`) under the lock.
    fn adjust(
        &mut self,
        delta: &VirtualRegister,
        add: bool,
        vregs: &mut Vregs,
    ) -> Result<(), String> {
        let counter = vregs.next();
        let value = vregs.next();
        self.lock_call("pthread_mutex_lock")?;
        self.address(ARENA_MAPPED_BYTES_SYMBOL, &counter);
        self.instructions.extend([
            abi::load_u64(&value, &counter, 0),
            if add {
                abi::add_registers(&value, &value, delta)
            } else {
                abi::subtract_registers(&value, &value, delta)
            },
            abi::store_u64(&value, &counter, 0),
        ]);
        self.lock_call("pthread_mutex_unlock")
    }
}

/// Before a block map: charge `map_size` to the counter, or branch to `over` (with
/// the lock released) when the charged total would pass the limit or overflow.
/// Leaves `charged` 1 when a charge was made, 0 when the limit is 0 (no
/// enforcement) — the refund after a failed map reads it.
pub(crate) fn emit_map_charge(
    site: &mut CapSite<'_>,
    map_size: &VirtualRegister,
    charged: &VirtualRegister,
    over: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let limit_addr = vregs.next();
    let limit = vregs.next();
    let counter = vregs.next();
    let total = vregs.next();
    let skip = format!("{}_cap_charge_skip", site.symbol);
    let refuse = format!("{}_cap_charge_refuse", site.symbol);
    site.instructions
        .push(abi::move_immediate(charged, "Integer", "0"));
    site.address(ARENA_MAP_LIMIT_SYMBOL, &limit_addr);
    site.instructions.extend([
        abi::load_u64(&limit, &limit_addr, 0),
        abi::compare_immediate(&limit, "0"),
        abi::branch_eq(&skip),
    ]);
    site.lock_call("pthread_mutex_lock")?;
    site.address(ARENA_MAPPED_BYTES_SYMBOL, &counter);
    site.address(ARENA_MAP_LIMIT_SYMBOL, &limit_addr);
    site.instructions.extend([
        abi::load_u64(&total, &counter, 0),
        abi::add_registers(&total, &total, map_size),
        // A wrapped sum is past any limit.
        abi::compare_registers(&total, map_size),
        abi::branch_lo(&refuse),
        abi::load_u64(&limit, &limit_addr, 0),
        abi::compare_registers(&total, &limit),
        abi::branch_hi(&refuse),
        abi::store_u64(&total, &counter, 0),
        abi::move_immediate(charged, "Integer", "1"),
    ]);
    site.lock_call("pthread_mutex_unlock")?;
    site.instructions
        .extend([abi::branch(&skip), abi::label(&refuse)]);
    site.lock_call("pthread_mutex_unlock")?;
    site.instructions
        .extend([abi::branch(over), abi::label(&skip)]);
    Ok(())
}

/// After a failed block map: give back the charge `emit_map_charge` made.
pub(crate) fn emit_map_refund(
    site: &mut CapSite<'_>,
    map_size: &VirtualRegister,
    charged: &VirtualRegister,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let done = format!("{}_cap_refund_done", site.symbol);
    site.instructions
        .extend([abi::compare_immediate(charged, "0"), abi::branch_eq(&done)]);
    site.adjust(map_size, false, vregs)?;
    site.instructions.push(abi::label(&done));
    Ok(())
}

/// Before a block unmap: give back its `block_size`, unless the limit is 0.
pub(crate) fn emit_unmap_refund(
    site: &mut CapSite<'_>,
    block_size: &VirtualRegister,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let limit_addr = vregs.next();
    let limit = vregs.next();
    let done = format!("{}_cap_unmap_done", site.symbol);
    site.address(ARENA_MAP_LIMIT_SYMBOL, &limit_addr);
    site.instructions.extend([
        abi::load_u64(&limit, &limit_addr, 0),
        abi::compare_immediate(&limit, "0"),
        abi::branch_eq(&done),
    ]);
    site.adjust(block_size, false, vregs)?;
    site.instructions.push(abi::label(&done));
    Ok(())
}

/// `_mfb_rt_arena_map_limit = 0`: stop all accounting from here on (shutdown, a
/// fork child). Uses `scratch` only.
pub(crate) fn emit_disable_accounting(
    symbol: &str,
    scratch: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    push_symbol_address(
        symbol,
        ARENA_MAP_LIMIT_SYMBOL,
        scratch,
        instructions,
        relocations,
    );
    instructions.push(abi::store_u64(abi::ZERO, scratch, 0));
}
