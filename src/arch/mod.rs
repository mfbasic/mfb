pub(crate) mod aarch64;
/// Operand decoding (`field`/`immediate`/`shift`) shared by every backend
/// encoder (bug-341-B7).
pub(crate) mod encode_operand;
/// The shared two-pass plan-to-image encoding driver (`encode_plan`) every
/// backend's `encode()` delegates to (bug-341-B1).
pub(crate) mod encode_plan;
/// The ISA-neutral linkable-image container types (`EncodedImage` and siblings).
/// Lives here, not under `aarch64/encode/`, because every backend encoder and
/// both linkers consume them — they describe a linkable image, not an ISA
/// (bug-341-B2).
pub(crate) mod image;
/// The neutral cross-arch MIR instruction vocabulary (`CodeOp`). Lives here, not
/// under `aarch64/`, because every backend consumes it (bug-82).
pub(crate) mod ops;
pub(crate) mod riscv64;
pub(crate) mod x86_64;
