pub(crate) mod aarch64;
/// The neutral cross-arch MIR instruction vocabulary (`CodeOp`). Lives here, not
/// under `aarch64/`, because every backend consumes it (bug-82).
pub(crate) mod ops;
pub(crate) mod riscv64;
pub(crate) mod x86_64;
