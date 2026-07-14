> Along the way I fixed several genuinely important bugs, notably that the shared allocator's **call-clobber masks were AArch64/x86-hardcoded** — a value live across an internal helper call sat in a riscv caller-saved register and got clobbered. That fix is `is_riscv`-gated, so aarch64/x86 stay byte-identical.

Is there a reaons to gate this, other than aarch64/x86 stay byte-identical? 
